//! Reading the game's screen: the relic reward screen and riven cards.
//!
//! Only pixels from a screenshot are read (like WFInfo / wfinfo-ng). Needs the `tesseract` binary
//! and one screenshot tool.

pub mod image;
pub mod matcher;
pub mod riven;
pub mod screenshot;
pub mod tesseract;

use std::fs;
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde_json::Value;

use self::image::Rgb;
use self::matcher::{Names, RewardMatch, Word};
use crate::compositor::which;
use crate::config::{self, ScreenshotTool};

/// The helper programs this machine has.
pub struct Tools {
    pub tesseract: bool,
    pub screenshot: Option<&'static str>,
    pub notify_send: bool,
}

pub fn detect_tools(setting: ScreenshotTool) -> Tools {
    Tools {
        tesseract: which("tesseract").is_some(),
        screenshot: screenshot::resolve(setting).map(screenshot::program),
        notify_send: cfg!(windows) || which("notify-send").is_some(),
    }
}

/// The reward names in a frame, left to right, and the text they were read from.
pub fn read_rewards(frame: &Rgb, names: &Names) -> Result<(Vec<RewardMatch>, String)> {
    if which("tesseract").is_none() {
        bail!("tesseract is not installed, so the reward screen cannot be read.");
    }
    let variants = image::reward_variants(frame);
    // Independent tesseract passes: run them side by side, not one after another.
    let passes: Vec<Result<Vec<tesseract::TsvWord>>> = std::thread::scope(|s| {
        let jobs: Vec<_> = variants.iter().map(|v| s.spawn(move || tesseract::words(v, 11))).collect();
        jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| bail!("tesseract thread panicked"))).collect()
    });
    let mut read: Vec<Vec<tesseract::TsvWord>> = Vec::new();
    let mut first_error = None;
    for pass in passes {
        match pass {
            Ok(words) => read.push(words),
            Err(e) => first_error = first_error.or(Some(e)),
        }
    }
    if read.is_empty() {
        return Err(first_error.unwrap_or_else(|| anyhow::anyhow!("tesseract read nothing")));
    }
    let confident: Vec<Vec<Word>> =
        read.iter().map(|ws| ws.iter().filter(|w| w.conf >= 20.0).map(|w| w.word.clone()).collect()).collect();
    let (matches, text) = matcher::pick_rewards(&confident, names);
    if !matches.is_empty() {
        return Ok((matches, text));
    }
    // Nothing column-shaped: fuzzy-match names anywhere in everything tesseract saw. The words
    // are the same ones its plain-text mode prints, so no second set of passes is needed.
    let all_text = read
        .iter()
        .map(|ws| ws.iter().map(|w| w.word.text.as_str()).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((matcher::match_rewards(&all_text, names, 4), all_text))
}

/// The riven card in a frame: weapon, stats and the raw text.
pub fn read_riven(frame: &Rgb, weapons: &[String]) -> Result<(riven::RivenRead, String)> {
    if which("tesseract").is_none() {
        bail!("tesseract is not installed, so the riven card cannot be read.");
    }
    // A riven card can sit anywhere, unlike the reward band: read the whole frame.
    let text = tesseract::text(&image::riven_frame(frame), 6)?;
    Ok((riven::parse(&text, weapons), text))
}

// ------------------------------------------------------------------ the frame archive

const KEEP_SCANS: usize = 12;

pub fn frames_dir() -> PathBuf {
    config::dirs().cache.join("frames")
}

/// Keeps the frame a scan worked from, with what it read, for replaying later.
///
/// Every scan is kept (the newest 12 of each kind, ~7 MB apiece), not only the bad ones and not
/// only the last: the reward
/// screen is gone 15 s after it opens, and the old single `last_reward.png` was overwritten by
/// the next unrelated scan. A screenshot pasted into a chat is cropped and rescaled, which
/// changes what tesseract reads, so this is the only faithful record. Runs in the background:
/// PNG compression costs about a second and nothing waits on it.
pub fn archive(kind: &'static str, frame: Rgb, text: String, result: Value) {
    std::thread::Builder::new()
        .name("archive".into())
        .spawn(move || {
            let dir = frames_dir();
            if fs::create_dir_all(&dir).is_err() {
                return;
            }
            let stem = dir.join(format!("{kind}-{}", crate::clock::stamp(crate::now())));
            if let Some(img) = ::image::RgbImage::from_raw(frame.width as u32, frame.height as u32, frame.data) {
                let _ = img.save(stem.with_extension("png"));
            }
            let _ = fs::write(stem.with_extension("txt"), text);
            let _ = fs::write(stem.with_extension("json"), serde_json::to_vec_pretty(&result).unwrap_or_default());
            prune(&dir, kind);
        })
        .ok();
}

fn prune(dir: &std::path::Path, kind: &str) {
    let prefix = format!("{kind}-");
    let mut stems: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with(&prefix) && name.ends_with(".png")).then(|| name.trim_end_matches(".png").to_string())
        })
        .collect();
    stems.sort();
    let excess = stems.len().saturating_sub(KEEP_SCANS);
    for stem in &stems[..excess] {
        for ext in ["png", "txt", "json"] {
            let _ = fs::remove_file(dir.join(format!("{stem}.{ext}")));
        }
    }
}
