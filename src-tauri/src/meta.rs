//! Mojang metadata: the version manifest, a version's detail JSON, and the asset index.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub latest: Latest,
    pub versions: Vec<Entry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub sha1: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDetail {
    pub id: String,
    pub main_class: String,
    pub java_version: Option<JavaVersion>,
    pub asset_index: AssetIndexRef,
    pub downloads: Downloads,
    pub libraries: Vec<Library>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub component: String,
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub total_size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Downloads {
    pub client: Artifact,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    pub sha1: String,
    pub size: u64,
    pub url: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    /// Maven coordinate. Not read yet; kept because it is how the format identifies a
    /// library and the mod-conflict work in a later phase needs it.
    #[allow(dead_code)]
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibDownloads>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<Os>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Os {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

pub fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

impl Library {
    /// Mojang's rule algorithm: with rules present the default is deny, and each
    /// matching rule overwrites the verdict in order. 77 of 26.2's 131 libraries
    /// carry rules, so getting this wrong means downloading another OS's natives.
    pub fn applies(&self) -> bool {
        let Some(rules) = &self.rules else {
            return true;
        };
        let mut allowed = false;
        for rule in rules {
            let matches = match &rule.os {
                None => true,
                Some(os) => {
                    let name_ok = os.name.as_deref().map_or(true, |n| n == os_name());
                    let arch_ok = os
                        .arch
                        .as_deref()
                        .map_or(true, |a| a == std::env::consts::ARCH);
                    name_ok && arch_ok
                }
            };
            if matches {
                allowed = rule.action == "allow";
            }
        }
        allowed
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    pub objects: BTreeMap<String, AssetObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

pub async fn manifest(client: &reqwest::Client) -> Result<Manifest> {
    Ok(client.get(MANIFEST_URL).send().await?.error_for_status()?.json().await?)
}

pub async fn detail(client: &reqwest::Client, url: &str) -> Result<VersionDetail> {
    Ok(client.get(url).send().await?.error_for_status()?.json().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(rules: Option<Vec<Rule>>) -> Library {
        Library { name: "test:lib:1".into(), downloads: None, rules }
    }
    fn rule(action: &str, os: Option<&str>) -> Rule {
        Rule {
            action: action.into(),
            os: os.map(|n| Os { name: Some(n.into()), arch: None }),
        }
    }

    #[test]
    fn no_rules_always_applies() {
        assert!(lib(None).applies());
    }

    #[test]
    fn rules_default_to_deny() {
        // A rule block that never matches leaves the verdict at its default, which is deny.
        let other = if os_name() == "linux" { "windows" } else { "linux" };
        assert!(!lib(Some(vec![rule("allow", Some(other))])).applies());
    }

    #[test]
    fn matching_allow_applies() {
        assert!(lib(Some(vec![rule("allow", Some(os_name()))])).applies());
    }

    #[test]
    fn later_rule_overrides_earlier() {
        // The real shape: blanket allow, then disallow this OS. Getting the ordering
        // wrong here means shipping another platform's natives.
        let l = lib(Some(vec![rule("allow", None), rule("disallow", Some(os_name()))]));
        assert!(!l.applies());
    }

    #[test]
    fn blanket_allow_applies() {
        assert!(lib(Some(vec![rule("allow", None)])).applies());
    }

    #[test]
    fn arch_must_match_too() {
        let wrong_arch = Rule {
            action: "allow".into(),
            os: Some(Os { name: Some(os_name().into()), arch: Some("definitely-not-an-arch".into()) }),
        };
        assert!(!lib(Some(vec![wrong_arch])).applies());
    }

    /// Hits the real Mojang endpoints. Run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_manifest_resolves_latest_release() {
        let http = reqwest::Client::builder().user_agent("Vantage/test").build().unwrap();
        let m = manifest(&http).await.expect("manifest");
        assert!(m.versions.len() > 800, "manifest looks truncated");

        let latest = m.versions.iter().find(|v| v.id == m.latest.release).expect("latest release listed");
        let d = detail(&http, &latest.url).await.expect("version detail");

        assert_eq!(d.id, m.latest.release);
        assert!(d.main_class.contains("Main"), "unexpected main class: {}", d.main_class);
        assert!(d.java_version.is_some(), "modern versions declare a java runtime");
        assert!(d.downloads.client.size > 1_000_000);

        let applicable = d.libraries.iter().filter(|l| l.applies()).count();
        assert!(applicable > 0 && applicable <= d.libraries.len());
        // If the filter ever degenerates to "everything", the rules are being ignored.
        assert!(applicable < d.libraries.len(), "rule filtering excluded nothing");
    }
}
