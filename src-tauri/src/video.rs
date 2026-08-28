//! Sensible video settings on a first launch.
//!
//! A new install drops the player into Minecraft's defaults, which are tuned for a machine that
//! has to run the game at all rather than one playing PvP. VSync in particular caps the frame
//! rate to the display and adds a frame of latency, which is the opposite of what this client
//! is for.
//!
//! Applied once, and only when the profile has no `options.txt` at all. Anything else would
//! mean overwriting settings the player has since chosen, which a launcher has no business
//! doing — the whole point of picking good defaults is that they are *defaults*.

use crate::error::Result;
use std::path::Path;

/// Key/value pairs written into a fresh `options.txt`, in Minecraft's own encoding.
const DEFAULTS: &[(&str, &str)] = &[
    // Uncapped frames and no waiting on the display: the two settings that decide input latency.
    ("enableVsync", "false"),
    // 260 is not a frame cap. Options.UNLIMITED_FRAMERATE_CUTOFF is 260, and the slider reads
    // "Unlimited" at that value.
    ("maxFps", "260"),
    // Stored normalised rather than in degrees: 0.0 is 30 and 1.0 is 110, the maximum.
    ("fov", "1.0"),
    // 0 means "auto", which lands on 4 or more at high resolutions and makes the HUD enormous.
    ("guiScale", "3"),
];

pub struct Applied {
    pub wrote: bool,
    pub settings: Vec<(String, String)>,
}

/// Write the defaults if this profile has never been launched.
///
/// A partial `options.txt` is fine: Minecraft fills in every key it does not find and rewrites
/// the complete file when it next saves.
pub fn seed(game_dir: &Path) -> Result<Applied> {
    let path = game_dir.join("options.txt");
    if path.exists() {
        return Ok(Applied { wrote: false, settings: Vec::new() });
    }
    std::fs::create_dir_all(game_dir)?;
    let body: String = DEFAULTS.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    std::fs::write(&path, body)?;
    Ok(Applied { wrote: true, settings: owned() })
}

/// Force the defaults onto an existing `options.txt`, keeping every other line.
///
/// Only ever runs when the player asks for it. Rewrites the four keys in place rather than
/// truncating the file, so key bindings, sound levels and everything else survive.
pub fn apply(game_dir: &Path) -> Result<Applied> {
    let path = game_dir.join("options.txt");
    if !path.exists() {
        return seed(game_dir);
    }
    let existing = std::fs::read_to_string(&path)?;
    let mut out = String::with_capacity(existing.len() + 64);
    let mut seen = vec![false; DEFAULTS.len()];

    for line in existing.lines() {
        let key = line.split(':').next().unwrap_or("");
        match DEFAULTS.iter().position(|(k, _)| *k == key) {
            Some(i) => {
                seen[i] = true;
                out.push_str(&format!("{}:{}\n", DEFAULTS[i].0, DEFAULTS[i].1));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    for (i, (k, v)) in DEFAULTS.iter().enumerate() {
        if !seen[i] {
            out.push_str(&format!("{k}:{v}\n"));
        }
    }
    std::fs::write(&path, out)?;
    Ok(Applied { wrote: true, settings: owned() })
}

fn owned() -> Vec<(String, String)> {
    DEFAULTS.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
}

/// How each default reads in the game's own settings screen, for reporting.
pub fn describe(key: &str) -> &'static str {
    match key {
        "enableVsync" => "VSync: Off",
        "maxFps" => "Max Framerate: Unlimited",
        "fov" => "FOV: 110 (Quake Pro)",
        "guiScale" => "GUI Scale: 3",
        _ => "",
    }
}
