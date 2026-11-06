//! Item art for the UI, scaled by the engine.
//!
//! The sources serve 512 px (WFCD, browse.wf) and 128 px (warframe.market) images. Shrunk to a
//! 24-72 px slot by the web view they come out pixelated, so the UI asks
//! `/api/art?u=<url>&s=<px>` instead: the image is fetched once, scaled with a proper filter and
//! kept in the cache directory (`art/`), which also makes art work offline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use image::imageops::FilterType;
use image::{ImageFormat, Rgba32FImage};

use crate::{config, gamedata, market, net};

/// Textures from the game's own interface (mastery badges, faction and syndicate emblems,
/// portraits, location shots), served by path: `GAME_ART` + "Icons/BaroKiTeer.png".
pub const GAME_ART: &str = "https://browse.wf/Lotus/Interface/";
/// The only places art is fetched from: the endpoint must not become a proxy for any URL.
const SOURCES: [&str; 3] = [gamedata::IMG_URL, market::ASSETS, GAME_ART];
const MIN_SIZE: u32 = 16;
/// The sources' pictures are 512 px at most; card backgrounds use them whole.
const MAX_SIZE: u32 = 512;
/// How long a failed download is remembered before the source is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(600);
/// A cached image older than this is fetched again: art does change now and then.
const KEEP_FOR: Duration = Duration::from_secs(30 * 24 * 3600);

pub struct Art {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

fn allowed(url: &str) -> bool {
    SOURCES.iter().any(|s| url.starts_with(s)) && !url.contains("..")
}

/// The scaled image if it is already on disk. Never touches the network, so the HTTP workers can
/// answer with it straight away.
pub fn cached(url: &str, size: u32) -> Option<Art> {
    if !allowed(url) {
        return None;
    }
    match cached_copy(&cache_path(url, size.clamp(MIN_SIZE, MAX_SIZE))) {
        Some((art, true)) => Some(art),
        _ => None,
    }
}

/// The image at `url`, no larger than `size` pixels on its longer side. Downloads and scales it
/// when the cache has no recent copy, which can take a while.
pub fn get(url: &str, size: u32) -> Result<Art> {
    if !allowed(url) {
        bail!("not an art source: {url}");
    }
    let size = size.clamp(MIN_SIZE, MAX_SIZE);
    let path = cache_path(url, size);
    // An old copy is refreshed, but it will do when the source cannot be reached.
    let old = match cached_copy(&path) {
        Some((art, true)) => return Ok(art),
        Some((art, false)) => Some(art),
        None => None,
    };
    if !recently_failed(url) {
        match download(url).and_then(|body| scale(&body, size)).and_then(typed) {
            Ok(art) => {
                store(&path, &art.bytes);
                return Ok(art);
            }
            Err(e) => {
                FAILED.lock().unwrap().get_or_insert_with(HashMap::new).insert(url.to_string(), Instant::now());
                if old.is_none() {
                    return Err(e);
                }
            }
        }
    }
    old.ok_or_else(|| anyhow!("no art at {url}"))
}

fn download(url: &str) -> Result<Vec<u8>> {
    let resp = net::get(url, &[], Duration::from_secs(20))?;
    if resp.status != 200 {
        return Err(net::HttpStatus(resp.status).into());
    }
    Ok(resp.body)
}

/// Sources that just failed: a missing image (warframe.market answers 200 with an HTML page for
/// art it does not have) or no connection. Asking again for every card on a page would be slow.
static FAILED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn recently_failed(url: &str) -> bool {
    let mut failed = FAILED.lock().unwrap();
    let Some(map) = failed.as_mut() else { return false };
    map.retain(|_, at| at.elapsed() < RETRY_AFTER);
    map.contains_key(url)
}

/// Says what kind of image the bytes are; anything but PNG or WebP is not art.
fn typed(bytes: Vec<u8>) -> Result<Art> {
    let content_type = match image::guess_format(&bytes)? {
        ImageFormat::Png => "image/png",
        ImageFormat::WebP => "image/webp",
        other => bail!("unexpected image format {other:?}"),
    };
    Ok(Art { bytes, content_type })
}

/// A cached copy, and whether it is recent enough to use without asking the source again.
fn cached_copy(path: &Path) -> Option<(Art, bool)> {
    let art = typed(std::fs::read(path).ok()?).ok()?;
    // A file dated in the future (the clock was set back) counts as new.
    let age = std::fs::metadata(path).and_then(|m| m.modified()).ok()?.elapsed().unwrap_or_default();
    Some((art, age < KEEP_FOR))
}

/// Writes under another name first, then renames: a request must never read half a file. The
/// counter keeps two threads storing the same image from sharing a temporary file.
fn store(path: &Path, bytes: &[u8]) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.tmp", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// `<cache>/art/<name>-<hash>-<size>`: the name keeps the directory readable, the hash of the whole
/// URL keeps two sources with the same file name apart.
fn cache_path(url: &str, size: u32) -> PathBuf {
    let name: String = url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(40)
        .collect();
    // FNV-1a, written out because std's hasher may change between Rust versions.
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    config::dirs().cache.join("art").join(format!("{name}-{hash:016x}-{size}"))
}

/// Shrinks an image to fit `size` x `size` and returns it as PNG. An image that already fits is
/// returned untouched: scaling up only blurs it.
fn scale(body: &[u8], size: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory(body)?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    if w <= size && h <= size {
        return Ok(body.to_vec());
    }
    let (nw, nh) = fit(w, h, size);
    // Transparent pixels usually carry black as their colour. Averaging them in as they are
    // draws a dark fringe around every icon, so weight each colour by its alpha first.
    let mut px: Rgba32FImage = img.to_rgba32f();
    for p in px.pixels_mut() {
        let a = p.0[3];
        for c in &mut p.0[..3] {
            *c *= a;
        }
    }
    let mut small = image::imageops::resize(&px, nw, nh, FilterType::Lanczos3);
    for p in small.pixels_mut() {
        let a = p.0[3].clamp(0.0, 1.0);
        for c in &mut p.0[..3] {
            *c = if a > 0.0 { (*c / a).clamp(0.0, 1.0) } else { 0.0 };
        }
        p.0[3] = a;
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba32F(small).to_rgba8().write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

/// The largest size with the same proportions that fits a `size` x `size` box.
fn fit(w: u32, h: u32, size: u32) -> (u32, u32) {
    let long = w.max(h) as f64;
    let side = |v: u32| ((v as f64 * size as f64 / long).round() as u32).max(1);
    (side(w), side(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn fit_keeps_the_proportions() {
        assert_eq!(fit(512, 512, 72), (72, 72));
        assert_eq!(fit(82, 128, 48), (31, 48));
        assert_eq!(fit(128, 75, 48), (48, 28));
        assert_eq!(fit(1000, 1, 10), (10, 1));
    }

    #[test]
    fn a_small_image_is_left_alone() {
        let src = png(20, 10, |_, _| [1, 2, 3, 255]);
        assert_eq!(scale(&src, 24).unwrap(), src);
    }

    #[test]
    fn transparent_black_does_not_darken_the_edge() {
        // Left half opaque white, right half transparent black.
        let src = png(64, 64, |x, _| if x < 32 { [255, 255, 255, 255] } else { [0, 0, 0, 0] });
        let small = image::load_from_memory(&scale(&src, 16).unwrap()).unwrap().to_rgba8();
        assert_eq!(small.dimensions(), (16, 16));
        for p in small.pixels().filter(|p| p.0[3] > 8) {
            assert!(p.0[0] >= 250, "a visible pixel turned grey: {:?}", p.0);
        }
    }

    #[test]
    fn only_the_art_sources_are_fetched() {
        assert!(get("https://example.com/a.png", 64).is_err());
        assert!(get("https://warframe.market/static/assets/../../api/x", 64).is_err());
    }

    #[test]
    fn a_stored_image_is_found_again_and_other_files_are_not() {
        let dir = std::env::temp_dir().join(format!("wfp-art-test-{}", std::process::id()));
        let path = dir.join("icon-0123-48");
        let src = png(8, 8, |_, _| [9, 9, 9, 255]);
        store(&path, &src);
        let (found, fresh) = cached_copy(&path).expect("the stored image");
        assert!(fresh);
        assert_eq!((found.bytes, found.content_type), (src, "image/png"));
        // Nothing but the image is left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::write(&path, b"<html>").unwrap();
        assert!(cached_copy(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_html_answer_is_not_art() {
        assert!(scale(b"<html><body>404</body></html>", 64).is_err());
    }
}
