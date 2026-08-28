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
    /// What this build needs alongside it. Published per version, so a mod that gained a
    /// dependency in a later build is caught rather than assumed away. Not sent to the UI,
    /// which has no use for it.
    #[serde(default, skip_serializing)]
    pub dependencies: Vec<Dependency>,
    /// Modrinth's version name, e.g. "ferritecore-9.0.0-fabric". Not a display title.
    #[allow(dead_code)]
    pub name: String,
    pub version_number: String,
    pub version_type: String,
    pub date_published: String,
    pub files: Vec<ModFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dependency {
    pub project_id: Option<String>,
    /// "required", "optional", "incompatible" or "embedded".
    pub dependency_type: String,
}

impl Ver {
    /// Project ids this build cannot run without.
    pub fn required(&self) -> Vec<&str> {
        self.dependencies
            .iter()
            .filter(|d| d.dependency_type == "required")
            .filter_map(|d| d.project_id.as_deref())
            .collect()
    }

    /// The jar to actually install. Modrinth marks one file primary; sources and
    /// javadoc jars ride along and must not be picked.
    pub fn primary(&self) -> Option<&ModFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }
}

/// What we are looking for. Resource packs and shaders are not Fabric mods and must not be
/// filtered as if they were — packs publish against the `minecraft` loader.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Mod,
    ResourcePack,
    Shader,
}

impl Kind {
    pub fn project_type(self) -> &'static str {
        match self {
            Kind::Mod => "mod",
            Kind::ResourcePack => "resourcepack",
            Kind::Shader => "shader",
        }
    }
    /// Only mods get a loader facet; adding one to a pack search returns nothing.
    pub fn loader_facet(self) -> Option<&'static str> {
        match self {
            Kind::Mod => Some("fabric"),
            _ => None,
        }
    }
    /// Where the game expects this kind to live inside the profile.
    pub fn profile_dir(self) -> &'static str {
        match self {
            Kind::Mod => "mods",
            Kind::ResourcePack => "resourcepacks",
            Kind::Shader => "shaderpacks",
        }
    }
}

/// Projects of one kind compatible with a game version, most-downloaded first.
pub async fn search(
    http: &reqwest::Client,
    query: &str,
    game_version: &str,
    kind: Kind,
    limit: u32,
) -> Result<Vec<Hit>> {
    let mut facets = format!(
        r#"[["project_type:{}"],["versions:{game_version}"]"#,
        kind.project_type()
    );
    if let Some(loader) = kind.loader_facet() {
        facets.push_str(&format!(r#",["categories:{loader}"]"#));
    }
    facets.push(']');
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

/// Every published build of one project for this game version. Mods are additionally
/// filtered to Fabric; packs are not, because they do not publish against a mod loader.
pub async fn versions_of(
    http: &reqwest::Client,
    project: &str,
    game_version: &str,
    kind: Kind,
) -> Result<Vec<Ver>> {
    let gv = format!(r#"["{game_version}"]"#);
    let mut req = http
        .get(format!("{API}/project/{project}/version"))
        .query(&[("game_versions", gv.as_str())]);
    if kind == Kind::Mod {
        req = req.query(&[("loaders", r#"["fabric"]"#)]);
    }
    Ok(req.send().await?.error_for_status()?.json().await?)
}

/// Fabric mods, the common case.
pub async fn versions(
    http: &reqwest::Client,
    project: &str,
    game_version: &str,
) -> Result<Vec<Ver>> {
    versions_of(http, project, game_version, Kind::Mod).await
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Project {
    /// Modrinth's opaque id. Dependencies are declared by id, not slug.
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    /// Modrinth's dominant colour for the icon, as a packed RGB int.
    #[serde(default)]
    pub color: Option<u32>,
}

/// Bulk project lookup. One request for the whole Set rather than five.
pub async fn projects(http: &reqwest::Client, slugs: &[&str]) -> Result<Vec<Project>> {
    let ids = format!(
        "[{}]",
        slugs.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(",")
    );
    Ok(http
        .get(format!("{API}/projects"))
        .query(&[("ids", ids.as_str())])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GalleryItem {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub featured: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct License {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// The full project page. `body` is author-written markdown and must be treated as untrusted.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Detail {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub body: String,
    pub downloads: u64,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub gallery: Vec<GalleryItem>,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub issues_url: Option<String>,
}

pub async fn detail(http: &reqwest::Client, project: &str) -> Result<Detail> {
    Ok(http
        .get(format!("{API}/project/{project}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
