//! Who is signed in, and which of them is playing.
//!
//! Accounts are kept in a plain JSON file next to the store. Nothing secret goes in it: the
//! refresh token lives in the OS credential store, keyed by account id, and the access token is
//! short-lived enough that it is never written down at all.
//!
//! Only Microsoft accounts are listed. There is deliberately no way to add an offline account
//! as a peer of a real one — that is the shape a launcher takes when it is being used to play
//! without owning the game. The offline *session* that already exists is a different thing: it
//! is what runs before sign-in works, it is singleplayer-only because servers reject it, and
//! all that is stored for it is the name to show.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "gg.vantage.launcher";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    /// The Minecraft profile UUID, which is also the keychain key for the refresh token.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Accounts {
    #[serde(default)]
    pub list: Vec<StoredAccount>,
    /// Id of the account that plays. None when nobody is signed in.
    #[serde(default)]
    pub active: Option<String>,
    /// Name shown by the offline session, used until sign-in works.
    #[serde(default = "default_offline_name")]
    pub offline_name: String,
}

fn default_offline_name() -> String {
    "Player".to_string()
}

fn path(root: &Path) -> PathBuf {
    root.join("accounts.json")
}

pub fn load(root: &Path) -> Accounts {
    std::fs::read_to_string(path(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Accounts {
            list: Vec::new(),
            active: None,
            offline_name: default_offline_name(),
        })
}

fn save(root: &Path, a: &Accounts) -> Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::write(path(root), serde_json::to_string_pretty(a)?)?;
    Ok(())
}

/// Record an account and make it the one that plays.
///
/// Signing in again with an account already on the list updates the name rather than adding a
/// second row — gamertags change, ids do not.
pub fn add(root: &Path, id: &str, name: &str) -> Result<Accounts> {
    let mut a = load(root);
    match a.list.iter_mut().find(|x| x.id == id) {
        Some(existing) => existing.name = name.to_string(),
        None => a.list.push(StoredAccount { id: id.to_string(), name: name.to_string() }),
    }
    a.active = Some(id.to_string());
    save(root, &a)?;
    Ok(a)
}

pub fn select(root: &Path, id: &str) -> Result<Accounts> {
    let mut a = load(root);
    if !a.list.iter().any(|x| x.id == id) {
        return Err(Error::Other("no such account".into()));
    }
    a.active = Some(id.to_string());
    save(root, &a)?;
    Ok(a)
}

/// Forget an account, including its refresh token.
///
/// The keychain entry goes first. A stale credential outliving the row that explains it is the
/// one failure here that a player cannot see or clean up themselves.
pub fn remove(root: &Path, id: &str) -> Result<Accounts> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, id) {
        let _ = entry.delete_credential();
    }
    let mut a = load(root);
    a.list.retain(|x| x.id != id);
    if a.active.as_deref() == Some(id) {
        a.active = a.list.first().map(|x| x.id.clone());
    }
    save(root, &a)?;
    Ok(a)
}

pub fn set_offline_name(root: &Path, name: &str) -> Result<Accounts> {
    let trimmed = name.trim();
    // Minecraft's own rule for a legacy name. Rejecting here rather than letting the game fail
    // to start on a name it will not accept.
    if trimmed.len() < 3 || trimmed.len() > 16
        || !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(Error::Other(
            "3 to 16 characters, letters, numbers and underscore".into(),
        ));
    }
    let mut a = load(root);
    a.offline_name = trimmed.to_string();
    save(root, &a)?;
    Ok(a)
}

/// The name the game should launch under right now.
pub fn play_name(root: &Path) -> String {
    let a = load(root);
    a.active
        .as_ref()
        .and_then(|id| a.list.iter().find(|x| &x.id == id))
        .map(|x| x.name.clone())
        .unwrap_or(a.offline_name)
}
