//! Assembling and starting the game.
//!
//! Mojang's version JSON does not hand you a command line — it hands you two argument arrays
//! full of `${placeholders}` and rule blocks, and expects the launcher to resolve them. Modern
//! versions ship no natives classifiers (LWJGL extracts its own at runtime into the path we
//! give it), so there is no unzip step, just a directory to point at.

use crate::error::{Error, Result};
use crate::{fabric, java, meta, store::Store};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Who is playing. Until Microsoft approves the app, `offline` stands in — single player
/// works, online servers correctly reject it.
#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub uuid: String,
    pub token: String,
    pub kind: &'static str,
}

impl Session {
    pub fn offline(name: &str) -> Self {
        // Deterministic pseudo-UUID so the same name keeps the same singleplayer identity.
        let mut h: u64 = 0xcbf29ce484222325;
        for b in name.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        let uuid = format!(
            "{:08x}-{:04x}-3{:03x}-8{:03x}-{:012x}",
            (h >> 32) as u32, (h >> 16) as u16, (h & 0xfff) as u16,
            ((h >> 12) & 0xfff) as u16, h & 0xffff_ffff_ffff
        );
        Self { name: name.to_string(), uuid, token: "0".into(), kind: "legacy" }
    }
}

#[derive(Debug, Serialize)]
pub struct Plan {
    pub java: String,
    pub main_class: String,
    /// Which loader actually starts the game. Mods only load under Fabric.
    pub loader: String,
    pub mods: usize,
    pub classpath_entries: usize,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub game_dir: String,
}

fn sep() -> char {
    if cfg!(target_os = "windows") { ';' } else { ':' }
}

/// Mojang's rule algorithm again, this time for argument blocks. Feature-gated arguments
/// (demo mode, custom resolution) stay off unless we actually enable the feature.
fn rules_allow(rules: &[serde_json::Value]) -> bool {
    let mut allowed = false;
    for rule in rules {
        let action = rule.get("action").and_then(|a| a.as_str()).unwrap_or("allow");
        if rule.get("features").is_some() {
            return false;
        }
        let matches = match rule.get("os") {
            None => true,
            Some(os) => {
                let name_ok = os.get("name").and_then(|n| n.as_str())
                    .map_or(true, |n| n == meta::os_name());
                let arch_ok = os.get("arch").and_then(|a| a.as_str())
                    .map_or(true, |a| a == std::env::consts::ARCH);
                name_ok && arch_ok
            }
        };
        if matches {
            allowed = action == "allow";
        }
    }
    allowed
}

fn collect(list: &[serde_json::Value]) -> Vec<String> {
    let mut out = Vec::new();
    for item in list {
        match item {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Object(o) => {
                let ok = o
                    .get("rules")
                    .and_then(|r| r.as_array())
                    .map_or(true, |r| rules_allow(r));
                if !ok {
                    continue;
                }
                match o.get("value") {
                    Some(serde_json::Value::String(s)) => out.push(s.clone()),
                    Some(serde_json::Value::Array(a)) => {
                        out.extend(a.iter().filter_map(|v| v.as_str().map(str::to_string)))
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

fn fill(args: &[String], vars: &BTreeMap<&str, String>) -> Vec<String> {
    args.iter()
        .map(|a| {
            let mut s = a.clone();
            for (k, v) in vars {
                s = s.replace(&format!("${{{k}}}"), v);
            }
            s
        })
        .collect()
}

/// Everything needed to start the game, resolved but not yet spawned.
pub async fn plan(
    http: &reqwest::Client,
    store: &Store,
    id: &str,
    session: &Session,
    memory_mb: u32,
) -> Result<(Plan, PathBuf)> {
    let raw = tokio::fs::read(store.version_json(id)).await.map_err(|_| {
        Error::Other(format!("{id} is not installed — install it first"))
    })?;
    let detail: meta::VersionDetail = serde_json::from_slice(&raw)?;
    let json: serde_json::Value = serde_json::from_slice(&raw)?;

    // Mods live in the profile, but vanilla ignores them entirely. If any are present the
    // game must start through Fabric or the whole mod feature is a no-op.
    let game_dir = store.root.join("profiles").join("main");
    let mods_dir = game_dir.join("mods");
    let mod_count = std::fs::read_dir(&mods_dir)
        .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".jar")).count())
        .unwrap_or(0);

    let component = detail
        .java_version
        .as_ref()
        .map(|j| j.component.clone())
        .unwrap_or_else(|| "jre-legacy".into());
    let java_bin = java::provision(http, store, &component).await?;

    let fab = if mod_count > 0 {
        let loader = fabric::latest_loader(http, id).await?;
        let p = fabric::profile(http, id, &loader).await?;
        let libs = fabric::install(http, store, &p).await?;
        Some((loader, p, libs))
    } else {
        None
    };

    // Classpath: every applicable library, then the client jar last.
    let mut cp: Vec<String> = Vec::new();
    for lib in &detail.libraries {
        if !lib.applies() {
            continue;
        }
        if let Some(path) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()).and_then(|a| a.path.as_ref()) {
            cp.push(store.library(path).display().to_string());
        }
    }
    if let Some((_, _, libs)) = &fab {
        // Fabric's libraries go ahead of the client jar, as its own launcher expects.
        for p in libs {
            cp.push(p.display().to_string());
        }
    }
    cp.push(store.client_jar(id).display().to_string());

    let natives = game_dir.join("natives");
    tokio::fs::create_dir_all(&natives).await?;
    tokio::fs::create_dir_all(game_dir.join("mods")).await?;

    let mut vars: BTreeMap<&str, String> = BTreeMap::new();
    vars.insert("auth_player_name", session.name.clone());
    vars.insert("auth_uuid", session.uuid.clone());
    vars.insert("auth_access_token", session.token.clone());
    vars.insert("auth_xuid", String::new());
    vars.insert("user_type", session.kind.to_string());
    vars.insert("clientid", String::new());
    vars.insert("version_name", detail.id.clone());
    vars.insert("version_type", "release".into());
    vars.insert("game_directory", game_dir.display().to_string());
    vars.insert("assets_root", store.root.join("assets").display().to_string());
    vars.insert("assets_index_name", detail.asset_index.id.clone());
    vars.insert("natives_directory", natives.display().to_string());
    vars.insert("launcher_name", "vantage".into());
    vars.insert("launcher_version", env!("CARGO_PKG_VERSION").to_string());
    vars.insert(
        "classpath",
        cp.join(&sep().to_string()),
    );

    let empty = Vec::new();
    let args = json.get("arguments");
    let jvm_raw = args.and_then(|a| a.get("jvm")).and_then(|v| v.as_array()).unwrap_or(&empty);
    let game_raw = args.and_then(|a| a.get("game")).and_then(|v| v.as_array()).unwrap_or(&empty);

    // Mojang now publishes its own recommended flags in `default-user-jvm`. Use them, with
    // the heap overridden to what the profile asks for.
    let mut jvm: Vec<String> = vec![
        format!("-Xms{}M", (memory_mb / 2).max(512)),
        format!("-Xmx{memory_mb}M"),
        "-XX:+AlwaysPreTouch".into(),
        "-XX:+UseStringDeduplication".into(),
    ];
    jvm.extend(fill(&collect(jvm_raw), &vars));

    // Fabric contributes its own JVM arguments (notably -DFabricMcEmu) and replaces main.
    let (main_class, loader) = match &fab {
        Some((ver, p, _)) => {
            jvm.extend(fill(&p.arguments.jvm, &vars));
            (p.main_class.clone(), format!("Fabric {ver}"))
        }
        None => (detail.main_class.clone(), "vanilla".to_string()),
    };

    Ok((
        Plan {
            java: java_bin.display().to_string(),
            main_class,
            loader,
            mods: mod_count,
            classpath_entries: cp.len(),
            jvm_args: jvm,
            game_args: {
                let mut g = fill(&collect(game_raw), &vars);
                if let Some((_, p, _)) = &fab {
                    g.extend(fill(&p.arguments.game, &vars));
                }
                g
            },
            game_dir: game_dir.display().to_string(),
        },
        game_dir,
    ))
}

/// Start it. Returns the child so the caller can supervise or capture output.
pub fn spawn(plan: &Plan, game_dir: &Path) -> Result<std::process::Child> {
    let child = std::process::Command::new(&plan.java)
        .args(&plan.jvm_args)
        .arg(&plan.main_class)
        .args(&plan.game_args)
        .current_dir(game_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| Error::Other(format!("could not start Java: {e}")))?;
    Ok(child)
}
