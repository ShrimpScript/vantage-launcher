//! What we installed, and where it came from.
//!
//! Three times now the UI has guessed whether something is already installed by comparing a
//! Modrinth slug against a filename. It fails whenever an author names their jar or zip
//! differently from their project — "Spunky PVP Texture Pack" ships as `Spunky Pack Classic.zip`.
//! Guessing was always wrong; this records the mapping instead.

use crate::error::Result;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// kind -> project id -> installed file name
    #[serde(default)]
    pub entries: BTreeMap<String, BTreeMap<String, String>>,
}

fn path(store: &Store, profile: &str) -> std::path::PathBuf {
    store.profile_sub(profile, "").join("installed.json")
}

pub fn load(store: &Store, profile: &str) -> Manifest {
    std::fs::read(path(store, profile))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn record(store: &Store, profile: &str, kind: &str, project: &str, file: &str) -> Result<()> {
    let mut m = load(store, profile);
    m.entries
        .entry(kind.to_string())
        .or_default()
        .insert(project.to_string(), file.to_string());
    save(store, profile, &m)
}

/// Forget by file name — that is what a Remove button knows.
pub fn forget_file(store: &Store, profile: &str, kind: &str, file: &str) -> Result<()> {
    let mut m = load(store, profile);
    if let Some(map) = m.entries.get_mut(kind) {
        map.retain(|_, v| v != file);
    }
    save(store, profile, &m)
}

pub fn installed_ids(store: &Store, profile: &str, kind: &str) -> Vec<String> {
    load(store, profile)
        .entries
        .get(kind)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn save(store: &Store, profile: &str, m: &Manifest) -> Result<()> {
    let p = path(store, profile);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(m)?)?;
    Ok(())
}
