//! Microsoft sign-in.
//!
//! Authorization-code + PKCE over a loopback redirect, not device code (LAUNCHER.md §4):
//! click, approve in your browser, done — no code to type and no second device. The chain is
//! Microsoft -> Xbox Live -> XSTS -> Minecraft, which is four hops and each one fails
//! differently, so each has its own error.
//!
//! Nothing here works until the Azure app is approved for the Minecraft API. Before approval
//! `api.minecraftservices.com` answers 403 no matter how correct the tokens are.

use crate::error::{Error, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

const AUTHORIZE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";

/// `consumers` tenant is mandatory: XboxLive.signin is not available to work accounts.
const SCOPE: &str = "XboxLive.signin offline_access";

const KEYRING_SERVICE: &str = "gg.vantage.launcher";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
}

/// A signed-in account plus the token the game needs.
///
/// The token is never written anywhere. It expires in hours, and the refresh token in the OS
/// keychain is what survives a restart — so the worst a stolen Vantage install can hand over is
/// something already expired.
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub account: Account,
    pub token: String,
}

/// The client ID is public (this is a PKCE public client — there is no secret). It comes from
/// the environment, or a one-line file in the store, so nothing is baked into the binary.
pub fn client_id(root: &std::path::Path) -> Option<String> {
    if let Ok(v) = std::env::var("VANTAGE_CLIENT_ID") {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    std::fs::read_to_string(root.join("client-id.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn pkce() -> (String, String) {
    let verifier: String = {
        let mut rng = rand::thread_rng();
        (0..64)
            .map(|_| {
                const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
                SET[rng.gen_range(0..SET.len())] as char
            })
            .collect()
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Block on the loopback listener until the browser comes back with `?code=`.
/// Answers with a small page so the user is not left staring at a dead tab.
fn await_code(listener: TcpListener, expect_state: &str) -> Result<String> {
    for stream in listener.incoming() {
        let mut stream = stream?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;

        let target = line.split_whitespace().nth(1).unwrap_or("");
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut code = None;
        let mut state = None;
        let mut err = None;
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("code", v)) => code = Some(v.to_string()),
                Some(("state", v)) => state = Some(v.to_string()),
                Some(("error", v)) => err = Some(v.to_string()),
                _ => {}
            }
        }

        let ok = code.is_some() && err.is_none();
        let body = if ok {
            "<h2>Signed in</h2><p>You can close this tab and go back to Vantage.</p>"
        } else {
            "<h2>Sign-in failed</h2><p>Go back to Vantage for the reason.</p>"
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
             <!doctype html><meta charset=utf-8><title>Vantage</title>\
             <body style=\"background:#0B0D12;color:#EDF0F5;font-family:system-ui;\
             display:grid;place-items:center;height:100vh;margin:0\"><div>{body}</div>"
        );

        if let Some(e) = err {
            return Err(Error::Other(format!("Microsoft returned: {e}")));
        }
        // State check is what stops another local process feeding us a code.
        if state.as_deref() != Some(expect_state) {
            return Err(Error::Other("state mismatch — sign-in rejected".into()));
        }
        if let Some(c) = code {
            return Ok(c);
        }
    }
    Err(Error::Other("browser never returned an authorization code".into()))
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    let cmd = ("xdg-open", vec![url]);
    #[cfg(target_os = "macos")]
    let cmd = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let cmd = ("cmd", vec!["/C", "start", "", url]);
    std::process::Command::new(cmd.0)
        .args(cmd.1)
        .spawn()
        .map_err(|e| Error::Other(format!("could not open a browser: {e}")))?;
    Ok(())
}

#[derive(Deserialize)]
struct MsToken {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}
#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}
#[derive(Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Deserialize)]
struct McToken {
    access_token: String,
}

/// Everything after Microsoft has issued a token: Xbox Live, XSTS, Minecraft, profile.
///
/// Shared by sign-in and refresh. Signing in and resuming differ only in how the Microsoft
/// token is obtained, and duplicating four hops so the two could drift apart would be a poor
/// trade for the one branch it saves.
async fn finish(
    http: &reqwest::Client,
    ms_access: &str,
    refresh_token: Option<String>,
) -> Result<LiveSession> {
    // 2. Microsoft -> Xbox Live
    let xbl: XboxResponse = http
        .post(XBL)
        .json(&serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access}")
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| Error::Other(format!("Xbox Live refused the token: {e}")))?
        .json()
        .await?;

    let uhs = xbl
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| Error::Other("Xbox Live returned no user hash".into()))?;

    // 3. Xbox Live -> XSTS
    let xsts_res = http
        .post(XSTS)
        .json(&serde_json::json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }))
        .send()
        .await?;
    if xsts_res.status() == 401 {
        return Err(Error::Other(
            "XSTS rejected this account — it usually means no Xbox profile exists for it, \
             or the account is a child account without an adult in the family."
                .into(),
        ));
    }
    let xsts: XboxResponse = xsts_res.error_for_status()?.json().await?;

    // 4. XSTS -> Minecraft
    let mc_res = http
        .post(MC_LOGIN)
        .json(&serde_json::json!({
            "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token)
        }))
        .send()
        .await?;
    if mc_res.status() == 403 {
        return Err(Error::Other(
            "Minecraft services returned 403. The Azure app is not approved for the Minecraft \
             API yet — apply at https://aka.ms/mce-reviewappid and try again once it clears."
                .into(),
        ));
    }
    let mc: McToken = mc_res.error_for_status()?.json().await?;

    let account: Account = http
        .get(MC_PROFILE)
        .bearer_auth(&mc.access_token)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| Error::Other(format!("no Minecraft profile on this account: {e}")))?
        .json()
        .await?;

    // Refresh token goes to the OS credential store, never a plaintext file (LAUNCHER.md §4).
    if let Some(rt) = refresh_token {
        match keyring::Entry::new(KEYRING_SERVICE, &account.id) {
            Ok(entry) => {
                if let Err(e) = entry.set_password(&rt) {
                    eprintln!("could not persist refresh token: {e}");
                }
            }
            Err(e) => eprintln!("no OS credential store available: {e}"),
        }
    }

    Ok(LiveSession { account, token: mc.access_token })
}

/// Sign back in without a browser, using the refresh token saved for this account.
///
/// Microsoft can refuse a refresh token — revoked, expired, password changed — and that is a
/// normal outcome rather than a fault: the caller falls back to asking for a real sign-in.
pub async fn refresh(
    http: &reqwest::Client,
    client_id: &str,
    account_id: &str,
) -> Result<LiveSession> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account_id)
        .map_err(|e| Error::Other(format!("no OS credential store available: {e}")))?;
    let stored = entry
        .get_password()
        .map_err(|_| Error::Other("no saved sign-in for this account".into()))?;

    let ms: MsToken = http
        .post(TOKEN)
        .form(&[
            ("client_id", client_id),
            ("refresh_token", stored.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?
        .error_for_status()
        .map_err(|_| Error::Other("saved sign-in has expired — sign in again".into()))?
        .json()
        .await?;

    finish(http, &ms.access_token, ms.refresh_token).await
}

/// The whole chain, starting at a browser. Returns the profile and a live token.
pub async fn sign_in(http: &reqwest::Client, client_id: &str) -> Result<LiveSession> {
    let (verifier, challenge) = pkce();
    let state: String = {
        let mut rng = rand::thread_rng();
        (0..24).map(|_| char::from(b'a' + rng.gen_range(0..26))).collect()
    };

    // Port 0 lets the OS pick, so nothing collides with whatever else is listening.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}");

    let url = format!(
        "{AUTHORIZE}?client_id={client_id}&response_type=code&redirect_uri={redirect}\
         &response_mode=query&scope={scope}&state={state}\
         &code_challenge={challenge}&code_challenge_method=S256",
        scope = urlencode(SCOPE),
        redirect = urlencode(&redirect),
    );
    open_browser(&url)?;

    let expect = state.clone();
    let code = tokio::task::spawn_blocking(move || await_code(listener, &expect))
        .await
        .map_err(|e| Error::Other(format!("sign-in task failed: {e}")))??;

    // 1. code -> Microsoft token
    let ms: MsToken = http
        .post(TOKEN)
        .form(&[
            ("client_id", client_id),
            ("code", &code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", &redirect),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| Error::Other(format!("Microsoft rejected the code exchange: {e}")))?
        .json()
        .await?;

    finish(http, &ms.access_token, ms.refresh_token).await
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
