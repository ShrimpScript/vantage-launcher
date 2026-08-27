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

/// What we know about one installed thing. Recorded at install time so the installed lists
/// can show a title and icon without a network round trip, and without guessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Entry {
    /// Older manifests stored just the file name.
    Legacy(String),
    Full {
        file: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        icon: Option<String>,
    },
}

impl Entry {
    pub fn file(&self) -> &str {
        match self {
            Entry::Legacy(f) => f,
            Entry::Full { file, .. } => file,
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Entry::Legacy(f) => f,
            Entry::Full { title, file, .. } => if title.is_empty() { file } else { title },
        }
    }
    pub fn icon(&self) -> Option<&str> {
        match self {
            Entry::Legacy(_) => None,
            Entry::Full { icon, .. } => icon.as_deref(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// kind -> project id -> what we installed
    #[serde(default)]
    pub entries: BTreeMap<String, BTreeMap<String, Entry>>,
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

/// The file we previously recorded for this project, if it differs from the new one.
/// Installing a newer build must remove the old jar — two versions of the same mod in the
/// folder is a crash, not a choice.
pub fn superseded(
    store: &Store,
    profile: &str,
    kind: &str,
    project: &str,
    new_file: &str,
) -> Option<String> {
    load(store, profile)
        .entries
        .get(kind)
        .and_then(|m| m.get(project))
        .map(|e| e.file().to_string())
        .filter(|old| old != new_file)
}

pub fn record(
    store: &Store,
    profile: &str,
    kind: &str,
    project: &str,
    file: &str,
    title: &str,
    icon: Option<String>,
) -> Result<()> {
    let mut m = load(store, profile);
    m.entries.entry(kind.to_string()).or_default().insert(
        project.to_string(),
        Entry::Full { file: file.to_string(), title: title.to_string(), icon },
    );
    save(store, profile, &m)
}

/// Everything installed for one kind, for the installed lists.
pub fn list(store: &Store, profile: &str, kind: &str) -> Vec<(String, Entry)> {
    load(store, profile)
        .entries
        .get(kind)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// Forget by file name — that is what a Remove button knows.
pub fn forget_file(store: &Store, profile: &str, kind: &str, file: &str) -> Result<()> {
    let mut m = load(store, profile);
    if let Some(map) = m.entries.get_mut(kind) {
        map.retain(|_, v| v.file() != file);
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
