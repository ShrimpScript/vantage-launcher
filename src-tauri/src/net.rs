//! The parallel, hash-verified downloader.
//!
//! Why this shape: Minecraft 26.2's asset index is 5,057 objects totalling 458 MB, with a
//! median object of 10 KB and 73% under 16 KB. Per-request latency dominates completely —
//! at 50 ms RTT, serial fetching spends four minutes doing nothing but waiting. Concurrency
//! is the entire performance story, not bandwidth.

use crate::error::{Error, Result};
use futures::stream::{self, StreamExt};
use sha1::{Digest, Sha1};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const CONCURRENCY: usize = 20;

#[derive(Debug, Clone)]
pub struct Item {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct Counters {
    pub done: AtomicU64,
    pub bytes: AtomicU64,
    pub skipped: AtomicU64,
}

pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("Vantage/", env!("CARGO_PKG_VERSION")))
        // Tiny files, many of them: keep connections hot rather than reopening.
        .pool_max_idle_per_host(CONCURRENCY)
        .build()?)
}

async fn fetch_one(client: &reqwest::Client, item: &Item, c: &Counters) -> Result<()> {
    // Already in the store at the right size? Then it is the right file — the path is
    // its hash. This is what makes a second profile on the same version nearly free.
    if let Ok(md) = tokio::fs::metadata(&item.dest).await {
        if item.size == 0 || md.len() == item.size {
            c.skipped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
    }
    if let Some(parent) = item.dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let body = client.get(&item.url).send().await?.error_for_status()?.bytes().await?;

    if let Some(expected) = &item.sha1 {
        let mut hasher = Sha1::new();
        hasher.update(&body);
        let actual = hex::encode(hasher.finalize());
        if &actual != expected {
            return Err(Error::Hash {
                path: item.dest.display().to_string(),
                expected: expected.clone(),
                actual,
            });
        }
    }

    // Write to a sibling temp then rename, so a killed download never leaves a
    // truncated file that the size check above would later accept as valid.
    let mut tmp = item.dest.clone().into_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    tokio::fs::write(&tmp, &body).await?;
    tokio::fs::rename(&tmp, &item.dest).await?;

    c.bytes.fetch_add(body.len() as u64, Ordering::Relaxed);
    Ok(())
}

pub async fn fetch_all(
    client: &reqwest::Client,
    items: Vec<Item>,
    counters: Arc<Counters>,
) -> Result<()> {
    let results = stream::iter(items.into_iter().map(|item| {
        let client = client.clone();
        let counters = counters.clone();
        async move {
            let outcome = fetch_one(&client, &item, &counters).await;
            counters.done.fetch_add(1, Ordering::Relaxed);
            outcome
        }
    }))
    .buffer_unordered(CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    for r in results {
        r?;
    }
    Ok(())
}
