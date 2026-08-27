//! Modrinth, the first-class library (LAUNCHER.md §3.1).
//!
//! Open API, no key, permissive terms — so search results can be cached and the UI can feel
//! instant. Modelled against live responses, not assumptions: every field below was verified
//! against api.modrinth.com on 2026-08-27.

use crate::error::Result;
use serde::{Deserialize, Serialize};

const API: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Hit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub downloads: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<Hit>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModFile {
    pub filename: String,
    pub url: String,
    pub size: u64,
    #[serde(default)]
    pub primary: bool,
    pub hashes: Hashes,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Hashes {
    pub sha1: String,
    /// The .mrpack format requires both, so both are carried through.
    pub sha512: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ver {
    pub id: String,
    /// Modrinth's version name, e.g. "ferritecore-9.0.0-fabric". Not a display title.
    #[allow(dead_code)]
    pub name: String,
    pub version_number: String,
    pub version_type: String,
    pub date_published: String,
    pub files: Vec<ModFile>,
}

impl Ver {
    /// The jar to actually install. Modrinth marks one file primary; sources and
    /// javadoc jars ride along and must not be picked.
    pub fn primary(&self) -> Option<&ModFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }
}

/// Fabric mods compatible with one game version, most-downloaded first.
pub async fn search(
    http: &reqwest::Client,
    query: &str,
    game_version: &str,
    limit: u32,
) -> Result<Vec<Hit>> {
    let facets = format!(
        r#"[["project_type:mod"],["versions:{game_version}"],["categories:fabric"]]"#
    );
    let r: SearchResponse = http
        .get(format!("{API}/search"))
        .query(&[("query", query), ("facets", &facets), ("limit", &limit.to_string())])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(r.hits)
}

/// Every published build of one project for this game version and loader.
pub async fn versions(
    http: &reqwest::Client,
    project: &str,
    game_version: &str,
) -> Result<Vec<Ver>> {
    let gv = format!(r#"["{game_version}"]"#);
    Ok(http
        .get(format!("{API}/project/{project}/version"))
        .query(&[("game_versions", gv.as_str()), ("loaders", r#"["fabric"]"#)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
