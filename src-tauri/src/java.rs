//! Java runtime provisioning.
//!
//! The version JSON names a component (26.2 wants `java-runtime-epsilon`, which is Java
//! 25.0.1). We fetch Mojang's own build rather than asking the user to install a JDK — nobody
//! should be shown a question they cannot answer (LAUNCHER.md §5).
//!
//! The component manifest is not a flat file list: for Linux it is 434 entries of which 205
//! are **symlinks** and 82 are directories. Miss the links and the runtime is subtly broken.

use crate::error::{Error, Result};
use crate::{net, store::Store};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

const ALL_RUNTIMES: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Debug, Deserialize)]
struct Available {
    manifest: Link,
}
#[derive(Debug, Deserialize)]
struct Link {
    url: String,
}

#[derive(Debug, Deserialize)]
struct ComponentManifest {
    files: BTreeMap<String, Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Entry {
    Directory,
    File {
        downloads: FileDownloads,
        #[serde(default)]
        executable: bool,
    },
    Link {
        target: String,
    },
}

#[derive(Debug, Deserialize)]
struct FileDownloads {
    raw: Raw,
}
#[derive(Debug, Deserialize)]
struct Raw {
    sha1: String,
    size: u64,
    url: String,
}

/// Mojang's key for this platform.
fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "windows-arm64",
        ("windows", "x86") => "windows-x86",
        ("windows", _) => "windows-x64",
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", _) => "mac-os",
        ("linux", "x86") => "linux-i386",
        _ => "linux",
    }
}

pub fn java_binary(store: &Store, component: &str) -> PathBuf {
    let base = store.root.join("java").join(component);
    if cfg!(target_os = "windows") {
        base.join("bin").join("javaw.exe")
    } else if cfg!(target_os = "macos") {
        base.join("jre.bundle/Contents/Home/bin/java")
    } else {
        base.join("bin").join("java")
    }
}

/// Download the runtime if it is not already present, and return the java executable.
pub async fn provision(http: &reqwest::Client, store: &Store, component: &str) -> Result<PathBuf> {
    let bin = java_binary(store, component);
    if bin.exists() {
        return Ok(bin);
    }

    let all: BTreeMap<String, BTreeMap<String, Vec<Available>>> =
        http.get(ALL_RUNTIMES).send().await?.error_for_status()?.json().await?;
    let url = all
        .get(platform())
        .and_then(|p| p.get(component))
        .and_then(|v| v.first())
        .map(|a| a.manifest.url.clone())
        .ok_or_else(|| {
            Error::Other(format!("Mojang has no {component} runtime for {}", platform()))
        })?;

    let manifest: ComponentManifest =
        http.get(&url).send().await?.error_for_status()?.json().await?;
    let base = store.root.join("java").join(component);

    // Directories first, so files and links always have somewhere to land.
    for (path, entry) in &manifest.files {
        if matches!(entry, Entry::Directory) {
            tokio::fs::create_dir_all(base.join(path)).await?;
        }
    }

    let mut items = Vec::new();
    for (path, entry) in &manifest.files {
        if let Entry::File { downloads, .. } = entry {
            items.push(net::Item {
                url: downloads.raw.url.clone(),
                dest: base.join(path),
                sha1: Some(downloads.raw.sha1.clone()),
                size: downloads.raw.size,
            });
        }
    }
    net::fetch_all(http, items, std::sync::Arc::new(net::Counters::default())).await?;

    // Executable bits and symlinks are what make the difference between a directory of
    // files and a working JRE.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for (path, entry) in &manifest.files {
            match entry {
                Entry::File { executable: true, .. } => {
                    let p = base.join(path);
                    if let Ok(md) = std::fs::metadata(&p) {
                        let mut perm = md.permissions();
                        perm.set_mode(perm.mode() | 0o111);
                        let _ = std::fs::set_permissions(&p, perm);
                    }
                }
                Entry::Link { target } => {
                    let p = base.join(path);
                    let _ = std::fs::remove_file(&p);
                    if let Some(parent) = p.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::os::unix::fs::symlink(target, &p);
                }
                _ => {}
            }
        }
    }

    if !bin.exists() {
        return Err(Error::Other(format!(
            "runtime installed but {} is missing",
            bin.display()
        )));
    }
    Ok(bin)
}
