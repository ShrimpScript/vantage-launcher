//! Fabric loader.
//!
//! Without this the launcher installs mods and then starts *vanilla*, which ignores the mods
//! folder entirely — the jars sit on disk doing nothing. Fabric ships a profile that
//! `inheritsFrom` the vanilla version and replaces the main class with its own launcher.

use crate::error::{Error, Result};
use crate::{net, store::Store};
use serde::Deserialize;

const META: &str = "https://meta.fabricmc.net/v2/versions/loader";

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub arguments: Args,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Args {
    #[serde(default)]
    pub jvm: Vec<String>,
    #[serde(default)]
    pub game: Vec<String>,
}

/// Fabric uses the older library shape: a maven coordinate plus a repository base, rather
/// than a resolved download URL.
#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// `group:artifact:version` -> `group/path/artifact/version/artifact-version.jar`
pub fn maven_path(coord: &str) -> Option<String> {
    let mut parts = coord.split(':');
    let group = parts.next()?.replace('.', "/");
    let artifact = parts.next()?;
    let version = parts.next()?;
    Some(format!("{group}/{artifact}/{version}/{artifact}-{version}.jar"))
}

pub async fn profile(http: &reqwest::Client, game: &str, loader: &str) -> Result<Profile> {
    Ok(http
        .get(format!("{META}/{game}/{loader}/profile/json"))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| Error::Other(format!("Fabric has no loader {loader} for {game}: {e}")))?
        .json()
        .await?)
}

/// Newest stable loader for this game version.
pub async fn latest_loader(http: &reqwest::Client, game: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Entry {
        loader: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        version: String,
        stable: bool,
    }
    let list: Vec<Entry> = http
        .get(format!("{META}/{game}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    list.iter()
        .find(|e| e.loader.stable)
        .or_else(|| list.first())
        .map(|e| e.loader.version.clone())
        .ok_or_else(|| Error::Other(format!("Fabric has no loader for {game}")))
}

/// Download Fabric's libraries into the same store vanilla uses, and return their paths so
/// they can go on the classpath.
pub async fn install(
    http: &reqwest::Client,
    store: &Store,
    profile: &Profile,
) -> Result<Vec<std::path::PathBuf>> {
    let mut items = Vec::new();
    let mut paths = Vec::new();
    for lib in &profile.libraries {
        let Some(rel) = maven_path(&lib.name) else { continue };
        let base = lib.url.clone().unwrap_or_else(|| "https://maven.fabricmc.net/".into());
        let dest = store.library(&rel);
        paths.push(dest.clone());
        items.push(net::Item {
            url: format!("{}{rel}", base.trim_end_matches('/').to_string() + "/"),
            dest,
            // Fabric's meta does not publish hashes here, so these are the one set of files
            // we cannot verify. Everything else in the store is sha1-checked.
            sha1: None,
            size: 0,
        });
    }
    net::fetch_all(http, items, std::sync::Arc::new(net::Counters::default())).await?;
    Ok(paths)
}
