//! The Vantage client mod: whether one is in the profile, and putting one there.
//!
//! The launcher and the in-game client are two halves of the same product, and until now only
//! one of them was managed. A jar copied in by hand is invisible to the launcher, cannot be
//! updated, and silently ends up in whatever state the last copy left it.
//!
//! Detection reads the mod id out of each jar's `fabric.mod.json` rather than matching a
//! filename. A file called `vantage-core-0.1.0.jar` is only a convention; the id inside it is
//! what Fabric actually loads, so that is what this trusts.

use crate::error::{Error, Result};
use crate::jar;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The mod id declared in the client's `fabric.mod.json`.
const MOD_ID: &str = "vantage";

#[derive(Serialize)]
pub struct ClientStatus {
    /// Version string from the installed jar, if one is there.
    pub version: Option<String>,
    /// The file it was found in, so the UI can name what it would replace.
    pub file: Option<String>,
}

#[derive(Deserialize)]
struct ModJson {
    id: String,
    version: String,
}

/// Read a jar's Fabric metadata. Anything unreadable is simply not the client.
fn mod_meta(path: &Path) -> Option<ModJson> {
    let bytes = jar::read_entry(path, "fabric.mod.json").ok()?;
    serde_json::from_slice::<ModJson>(&bytes).ok()
}

/// Find the installed client jar, if any.
fn find(mods_dir: &Path) -> Option<(PathBuf, String)> {
    let entries = std::fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jar") {
            continue;
        }
        if let Some(meta) = mod_meta(&path) {
            if meta.id == MOD_ID {
                return Some((path, meta.version));
            }
        }
    }
    None
}

pub fn status(store: &Store) -> ClientStatus {
    match find(&store.profile_mods("main")) {
        Some((path, version)) => ClientStatus {
            version: Some(version),
            file: path.file_name().map(|f| f.to_string_lossy().into_owned()),
        },
        None => ClientStatus { version: None, file: None },
    }
}

/// Put a client jar into the profile, replacing whatever version is already there.
///
/// Takes a local path or an http(s) URL. Two jars declaring the same mod id make Fabric refuse
/// to start, so the old one is removed rather than left beside the new one — the same reason
/// mod installs drop superseded files.
pub async fn install(http: &reqwest::Client, store: &Store, source: &str) -> Result<String> {
    let mods_dir = store.profile_mods("main");
    std::fs::create_dir_all(&mods_dir)?;

    let staged = if source.starts_with("http://") || source.starts_with("https://") {
        let bytes = http
            .get(source)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let tmp = mods_dir.join(".vantage-client.part");
        std::fs::write(&tmp, &bytes)?;
        tmp
    } else {
        let from = PathBuf::from(source);
        if !from.exists() {
            return Err(Error::Other(format!("no such file: {source}")));
        }
        let tmp = mods_dir.join(".vantage-client.part");
        std::fs::copy(&from, &tmp)?;
        tmp
    };

    // Verify before displacing anything: a truncated download or the wrong jar entirely would
    // otherwise remove a working client and leave nothing in its place.
    let Some(meta) = mod_meta(&staged) else {
        let _ = std::fs::remove_file(&staged);
        return Err(Error::Other("that jar has no fabric.mod.json".into()));
    };
    if meta.id != MOD_ID {
        let _ = std::fs::remove_file(&staged);
        return Err(Error::Other(format!(
            "that jar is '{}', not the Vantage client",
            meta.id
        )));
    }

    if let Some((old, _)) = find(&mods_dir) {
        std::fs::remove_file(&old)?;
    }
    let dest = mods_dir.join(format!("vantage-core-{}.jar", meta.version));
    std::fs::rename(&staged, &dest)?;
    Ok(meta.version)
}
