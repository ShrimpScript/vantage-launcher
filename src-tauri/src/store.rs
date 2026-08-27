//! The content-addressed store.
//!
//! Layout is deliberately Minecraft-standard (`assets/objects/<ab>/<hash>`,
//! `libraries/<maven path>`, `versions/<id>/<id>.jar`) so that other launchers can
//! read it and a user can leave with their install intact. Profiles link into this
//! store rather than owning copies — five profiles on one version cost one copy.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Installed {
    pub id: String,
    pub jar_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn discover() -> Result<Self> {
        let root = dirs::data_dir()
            .ok_or_else(|| Error::Other("no user data directory on this platform".into()))?
            .join("vantage");
        Ok(Self { root })
    }

    pub fn asset_object(&self, hash: &str) -> PathBuf {
        // Two-character shard, same as vanilla, so the tree stays browsable.
        self.root.join("assets").join("objects").join(&hash[..2]).join(hash)
    }
    pub fn asset_index(&self, id: &str) -> PathBuf {
        self.root.join("assets").join("indexes").join(format!("{id}.json"))
    }
    pub fn library(&self, path: &str) -> PathBuf {
        self.root.join("libraries").join(path)
    }
    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.root.join("versions").join(id)
    }
    pub fn client_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }
    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    /// A profile's mods directory. One implicit "main" profile until profiles ship.
    pub fn profile_mods(&self, profile: &str) -> PathBuf {
        self.root.join("profiles").join(profile).join("mods")
    }

    /// Recursive size + file count. Used by the UI to show what the store actually costs.
    pub fn usage(&self) -> (u64, u64) {
        fn walk(dir: &Path, files: &mut u64, bytes: &mut u64) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for entry in rd.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    walk(&entry.path(), files, bytes);
                } else if let Ok(md) = entry.metadata() {
                    *files += 1;
                    *bytes += md.len();
                }
            }
        }
        let (mut f, mut b) = (0, 0);
        walk(&self.root, &mut f, &mut b);
        (f, b)
    }

    /// Installed versions with what each actually costs on disk. The home screen shows
    /// these as real content; there is no placeholder version.
    pub fn installed_versions(&self) -> Vec<Installed> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.root.join("versions")) else {
            return out;
        };
        for entry in rd.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            let jar = self.client_jar(&id);
            let Ok(md) = std::fs::metadata(&jar) else { continue };
            out.push(Installed { id, jar_bytes: md.len() });
        }
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out
    }
}
