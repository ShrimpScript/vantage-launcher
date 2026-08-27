//! Reading assets out of the client jar we already manage.
//!
//! The point: the launcher does not ship stock Minecraft art. It reads the real textures
//! out of the user's own copy of the game, for the exact version they selected. A 26.2
//! grass block in the UI is *26.2's* grass block, straight from the jar on disk.

use crate::error::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::Read;
use std::path::Path;

pub fn read_entry(jar: &Path, name: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Other(format!("client jar unreadable: {e}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|_| Error::Other(format!("{name} is not in this version's jar")))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn read_png_data_uri(jar: &Path, name: &str) -> Result<String> {
    let bytes = read_entry(jar, name)?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

/// Every `textures/block/*.png` in the jar, without the path or extension.
pub fn block_textures(jar: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Other(format!("client jar unreadable: {e}")))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(i) else { continue };
        let name = entry.name();
        if let Some(rest) = name.strip_prefix("assets/minecraft/textures/block/") {
            if let Some(stem) = rest.strip_suffix(".png") {
                if !stem.contains('/') {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    Ok(out)
}
