//! The Vantage Set, as a real Modrinth modpack index.
//!
//! The bundled performance stack is not a hardcoded list inside the launcher — it is a
//! `.mrpack` we generate, hash-pin and can hand to anyone. That single decision buys four
//! things at once (LAUNCHER.md §2): it is auditable, it credits authors structurally because
//! the format carries them, it imports into Prism or Modrinth App so nobody is locked in, and
//! it keeps us clear of Sodium's PolyForm noncompete because we redistribute nothing — the
//! manifest points at cdn.modrinth.com and files are fetched upstream unmodified.

use crate::error::{Error, Result};
use crate::{modrinth, net, store::Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Pinned members, with the role each plays. Performance only — quality-of-life mods are the
/// user's choice, not ours (DESIGN.md: the Set is a toggle, not a cage).
pub const MEMBERS: &[(&str, &str, &str)] = &[
    // slug, display name, role
    ("sodium", "Sodium", "renderer"),
    ("lithium", "Lithium", "tick engine"),
    ("immediatelyfast", "ImmediatelyFast", "draw calls"),
    ("ferrite-core", "FerriteCore", "memory"),
    ("fabric-api", "Fabric API", "dependency"),
];

const FABRIC_META: &str = "https://meta.fabricmc.net/v2/versions/loader";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub game: String,
    #[serde(rename = "versionId")]
    pub version_id: String,
    pub name: String,
    pub summary: String,
    pub files: Vec<PackFile>,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackFile {
    pub path: String,
    pub hashes: modrinth::Hashes,
    pub downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
    pub env: Env,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Env {
    pub client: String,
    pub server: String,
}

/// What the UI shows about each member. Not part of the .mrpack format — the format has no
/// room for a human role — so it rides alongside.
#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub slug: String,
    pub title: String,
    pub role: &'static str,
    /// From Modrinth, so the UI shows the author's real icon rather than a blank square.
    pub icon_url: Option<String>,
    pub color: Option<u32>,
    pub version_number: String,
    pub version_type: String,
    pub filename: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Resolved {
    pub index: Index,
    pub members: Vec<Member>,
    pub total_bytes: u64,
}

#[derive(Deserialize)]
struct FabricLoaderEntry {
    loader: FabricLoader,
}
#[derive(Deserialize)]
struct FabricLoader {
    version: String,
    stable: bool,
}

async fn fabric_loader(http: &reqwest::Client, game_version: &str) -> Result<String> {
    let list: Vec<FabricLoaderEntry> = http
        .get(format!("{FABRIC_META}/{game_version}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    list.iter()
        .find(|e| e.loader.stable)
        .or_else(|| list.first())
        .map(|e| e.loader.version.clone())
        .ok_or_else(|| Error::Other(format!("Fabric has no loader for {game_version}")))
}

/// Ask Modrinth for the newest build of every member and pin it by hash.
///
/// All six lookups run concurrently. Doing them in sequence cost several seconds of dead
/// UI, which is not defensible in a launcher whose pitch is speed.
pub async fn resolve(http: &reqwest::Client, game_version: &str) -> Result<Resolved> {
    let loader_fut = fabric_loader(http, game_version);
    let member_futs = MEMBERS.iter().map(|(slug, title, role)| async move {
        let versions = modrinth::versions(http, slug, game_version).await?;
        let ver = versions.into_iter().next().ok_or_else(|| {
            Error::Other(format!("{slug} has no Fabric build for {game_version}"))
        })?;
        let file = ver
            .primary()
            .ok_or_else(|| Error::Other(format!("{slug} {} has no file", ver.version_number)))?
            .clone();
        Ok::<_, Error>((
            Member {
                slug: (*slug).to_string(),
                title: (*title).to_string(),
                role,
                icon_url: None,
                color: None,
                version_number: ver.version_number.clone(),
                version_type: ver.version_type.clone(),
                filename: file.filename.clone(),
                bytes: file.size,
            },
            PackFile {
                path: format!("mods/{}", file.filename),
                hashes: file.hashes.clone(),
                downloads: vec![file.url.clone()],
                file_size: file.size,
                env: Env { client: "required".into(), server: "unsupported".into() },
            },
        ))
    });

    let slugs: Vec<&str> = MEMBERS.iter().map(|(s, _, _)| *s).collect();
    let (loader, resolved, projects) = futures::try_join!(
        loader_fut,
        futures::future::try_join_all(member_futs),
        modrinth::projects(http, &slugs)
    )?;

    let mut files = Vec::with_capacity(resolved.len());
    let mut members = Vec::with_capacity(resolved.len());
    let mut total_bytes = 0u64;
    for (mut m, f) in resolved {
        if let Some(p) = projects.iter().find(|p| p.slug == m.slug) {
            m.icon_url = p.icon_url.clone();
            m.color = p.color;
        }
        total_bytes += m.bytes;
        members.push(m);
        files.push(f);
    }

    let mut dependencies = BTreeMap::new();
    dependencies.insert("minecraft".to_string(), game_version.to_string());
    dependencies.insert("fabric-loader".to_string(), loader);

    Ok(Resolved {
        index: Index {
            format_version: 1,
            game: "minecraft".into(),
            version_id: format!("{game_version}-{}", chrono_stamp()),
            name: "The Vantage Set".into(),
            summary: "Vantage's pinned performance stack. Every file is fetched from its \
                      author's official Modrinth release, unmodified."
                .into(),
            files,
            dependencies,
        },
        members,
        total_bytes,
    })
}

/// Date stamp without pulling in a date crate for one string.
fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    format!("{days}")
}

/// Install every file in the index into the profile, sha1-verified.
pub async fn apply(http: &reqwest::Client, store: &Store, index: &Index) -> Result<u64> {
    let mods = store.profile_mods("main");
    let items: Vec<net::Item> = index
        .files
        .iter()
        .map(|f| net::Item {
            url: f.downloads.first().cloned().unwrap_or_default(),
            dest: mods.join(f.path.trim_start_matches("mods/")),
            sha1: Some(f.hashes.sha1.clone()),
            size: f.file_size,
        })
        .collect();
    let bytes = items.iter().map(|i| i.size).sum();
    net::fetch_all(http, items, std::sync::Arc::new(net::Counters::default())).await?;
    Ok(bytes)
}

/// Write a genuine `.mrpack` — a zip with `modrinth.index.json` at the root. This is the
/// artifact anyone can unzip, verify, and import into another launcher.
pub fn export(store: &Store, index: &Index) -> Result<std::path::PathBuf> {
    use std::io::Write;
    let out = store.root.join(format!("vantage-set-{}.mrpack", index.dependencies["minecraft"]));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(&out)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("modrinth.index.json", opts)
        .map_err(|e| Error::Other(format!("mrpack write failed: {e}")))?;
    zip.write_all(serde_json::to_string_pretty(index)?.as_bytes())?;
    zip.finish()
        .map_err(|e| Error::Other(format!("mrpack finalise failed: {e}")))?;
    Ok(out)
}
