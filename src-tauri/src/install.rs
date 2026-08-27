//! Turning a version into a set of downloads, in two phases.
//!
//! Phase 1 (core) is the client jar, the OS-applicable libraries and the asset index —
//! a few dozen items, one of which is 37 MB. Phase 2 (assets) can only be built after
//! phase 1 lands, because the list of 5,057 objects lives inside the index we just fetched.

use crate::error::Result;
use crate::{meta, net, store::Store};

pub const RESOURCES: &str = "https://resources.download.minecraft.net";

pub struct Plan {
    pub detail: meta::VersionDetail,
    pub items: Vec<net::Item>,
    pub bytes: u64,
    pub libs_applicable: usize,
    pub libs_total: usize,
}

pub async fn plan_core(
    http: &reqwest::Client,
    store: &Store,
    entry: &meta::Entry,
) -> Result<Plan> {
    let detail = meta::detail(http, &entry.url).await?;
    let mut items = Vec::new();
    let mut bytes = 0u64;

    // Keep the version JSON beside the jar, vanilla-style, so the install is portable.
    let vj = store.version_json(&detail.id);
    if let Some(parent) = vj.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let raw = http.get(&entry.url).send().await?.error_for_status()?.bytes().await?;
    tokio::fs::write(&vj, &raw).await?;

    let client_jar = &detail.downloads.client;
    bytes += client_jar.size;
    items.push(net::Item {
        url: client_jar.url.clone(),
        dest: store.client_jar(&detail.id),
        sha1: Some(client_jar.sha1.clone()),
        size: client_jar.size,
    });

    let libs_total = detail.libraries.len();
    let mut libs_applicable = 0;
    for lib in &detail.libraries {
        if !lib.applies() {
            continue;
        }
        libs_applicable += 1;
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        let Some(path) = artifact.path.as_ref() else {
            continue;
        };
        bytes += artifact.size;
        items.push(net::Item {
            url: artifact.url.clone(),
            dest: store.library(path),
            sha1: Some(artifact.sha1.clone()),
            size: artifact.size,
        });
    }

    let ai = &detail.asset_index;
    bytes += ai.size;
    items.push(net::Item {
        url: ai.url.clone(),
        dest: store.asset_index(&ai.id),
        sha1: Some(ai.sha1.clone()),
        size: ai.size,
    });

    Ok(Plan { detail, items, bytes, libs_applicable, libs_total })
}

/// Read the freshly-downloaded index off disk and expand it into per-object items.
pub async fn plan_assets(store: &Store, index_id: &str) -> Result<(Vec<net::Item>, u64)> {
    let raw = tokio::fs::read(store.asset_index(index_id)).await?;
    let index: meta::AssetIndex = serde_json::from_slice(&raw)?;

    let mut items = Vec::with_capacity(index.objects.len());
    let mut bytes = 0u64;
    for object in index.objects.values() {
        bytes += object.size;
        items.push(net::Item {
            url: format!("{RESOURCES}/{}/{}", &object.hash[..2], object.hash),
            dest: store.asset_object(&object.hash),
            sha1: Some(object.hash.clone()),
            size: object.size,
        });
    }
    Ok((items, bytes))
}

/// Anywhere progress needs to go. Tauri emits events; the headless installer prints.
/// Keeping this a trait is what lets the install pipeline be exercised without a window.
pub trait ProgressSink: Send + Sync + 'static {
    fn emit(&self, phase: &'static str, done: u64, total: u64, bytes: u64, total_bytes: u64, skipped: u64);
}

#[derive(Debug, serde::Serialize)]
pub struct Report {
    pub id: String,
    pub libs_applicable: usize,
    pub libs_total: usize,
    pub files: u64,
    pub downloaded: u64,
    pub skipped: u64,
    pub bytes: u64,
    pub seconds: f64,
    pub store_root: String,
}

/// The whole install, both phases. This is the function the Tauri command wraps and the
/// one `--install` calls, so the GUI path and the CI path cannot drift apart.
pub async fn run<S: ProgressSink>(
    http: &reqwest::Client,
    store: &Store,
    entry: &meta::Entry,
    sink: std::sync::Arc<S>,
) -> Result<Report> {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    let started = std::time::Instant::now();

    let plan = plan_core(http, store, entry).await?;
    let index_id = plan.detail.asset_index.id.clone();
    let (libs_applicable, libs_total) = (plan.libs_applicable, plan.libs_total);

    let core = Arc::new(net::Counters::default());
    phase(http, plan.items, plan.bytes, core.clone(), sink.clone(), "core").await?;

    let (assets, asset_bytes) = plan_assets(store, &index_id).await?;
    let asset_counters = Arc::new(net::Counters::default());
    phase(http, assets, asset_bytes, asset_counters.clone(), sink.clone(), "assets").await?;

    let files = core.done.load(Ordering::Relaxed) + asset_counters.done.load(Ordering::Relaxed);
    let skipped = core.skipped.load(Ordering::Relaxed) + asset_counters.skipped.load(Ordering::Relaxed);
    let bytes = core.bytes.load(Ordering::Relaxed) + asset_counters.bytes.load(Ordering::Relaxed);

    Ok(Report {
        id: entry.id.clone(),
        libs_applicable,
        libs_total,
        files,
        downloaded: files - skipped,
        skipped,
        bytes,
        seconds: started.elapsed().as_secs_f64(),
        store_root: store.root.display().to_string(),
    })
}

async fn phase<S: ProgressSink>(
    http: &reqwest::Client,
    items: Vec<net::Item>,
    total_bytes: u64,
    counters: std::sync::Arc<net::Counters>,
    sink: std::sync::Arc<S>,
    name: &'static str,
) -> Result<()> {
    use std::sync::atomic::Ordering;
    let total = items.len() as u64;
    let ticker = {
        let counters = counters.clone();
        tokio::spawn(async move {
            loop {
                let done = counters.done.load(Ordering::Relaxed);
                sink.emit(
                    name,
                    done,
                    total,
                    counters.bytes.load(Ordering::Relaxed),
                    total_bytes,
                    counters.skipped.load(Ordering::Relaxed),
                );
                if done >= total {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
        })
    };
    let outcome = net::fetch_all(http, items, counters).await;
    ticker.abort();
    outcome
}
