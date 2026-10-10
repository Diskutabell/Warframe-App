//! Makes the bundled UI fonts available to GTK, so the overlays match the main window.

use std::ffi::{CString, c_int, c_uchar, c_void};

use crate::config;
use crate::server::Web;

#[link(name = "fontconfig")]
unsafe extern "C" {
    /// Adds a font file to the application's fontconfig configuration (NULL = the current one).
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const c_uchar) -> c_int;
}

/// Writes the embedded fonts to the cache and registers them with fontconfig. They are TrueType on
/// purpose: fontconfig accepts WOFF2, but Pango silently skips any .woff/.woff2 file (HarfBuzz would
/// get the compressed data and no glyphs). Must run before GTK starts: Pango caches the font list.
pub fn register() {
    let dir = config::dirs().cache.join("fonts");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for name in Web::iter().filter(|n| n.starts_with("fonts/") && n.ends_with(".ttf")) {
        let Some(file) = Web::get(&name) else { continue };
        let path = dir.join(name.trim_start_matches("fonts/"));
        if std::fs::read(&path).ok().as_deref() != Some(file.data.as_ref())
            && std::fs::write(&path, &file.data).is_err()
        {
            continue;
        }
        let Ok(c_path) = CString::new(path.to_string_lossy().as_bytes()) else { continue };
        // SAFETY: a valid NUL-terminated path; a null config means fontconfig's current one.
        unsafe {
            FcConfigAppFontAddFile(std::ptr::null_mut(), c_path.as_ptr().cast());
        }
    }
}
