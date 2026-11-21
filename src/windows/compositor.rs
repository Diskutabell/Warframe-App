//! Windows: where the game is, taking its screenshot, and the scan hotkeys.
//!
//! This stands in for `compositor.rs` on Windows (see `main.rs`). There is no compositor to ask:
//! the game's window is found among the desktop's windows by its class and title (never by
//! looking at the game's process), the screen is copied with GDI, and the hotkeys are registered
//! by the app itself while it runs.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, MonitorFromWindow, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetMessageW, GetWindowRect, GetWindowTextW, IsWindowVisible, MSG,
    PostThreadMessageW, WM_HOTKEY, WM_QUIT,
};
use windows_sys::core::BOOL;

use crate::UserError;
use crate::ocr::image::Rgb;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A program on PATH (`tesseract` finds `tesseract.exe`). For tesseract also the copy the exe
/// carries, and the folder its Windows installer uses, which it does not add to PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    let exe = format!("{program}.exe");
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if program == "tesseract" {
        #[cfg(feature = "bundled-tesseract")]
        {
            let bundled = bundled_tesseract_dir().join(&exe);
            if bundled.is_file() {
                return Some(bundled);
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(&base).join("Tesseract-OCR"));
                dirs.push(PathBuf::from(&base).join("Programs").join("Tesseract-OCR"));
            }
        }
    }
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

// ------------------------------------------------------------------ the bundled tesseract

/// The release exe carries tesseract (the MSYS2 build, the DLLs it loads and the English data) as
/// a zip, made by `.github/bundle-tesseract.sh`.
#[cfg(feature = "bundled-tesseract")]
static TESSERACT_ZIP: &[u8] = include_bytes!(env!("WFP_TESSERACT_ZIP"));

/// Where it is unpacked: one folder per app version, so an update brings its own copy.
#[cfg(feature = "bundled-tesseract")]
fn bundled_tesseract_dir() -> PathBuf {
    crate::config::dirs().cache.join(format!("tesseract-{}", env!("CARGO_PKG_VERSION")))
}

/// Unpacks the tesseract the exe carries, once per version (a build without it has nothing to do).
/// Runs on its own thread at startup: it takes a few seconds the first time.
pub fn unpack_tesseract() -> Result<()> {
    #[cfg(feature = "bundled-tesseract")]
    {
        let dir = bundled_tesseract_dir();
        if dir.join("tesseract.exe").is_file() {
            return Ok(());
        }
        let cache = &crate::config::dirs().cache;
        // Unpacked under another name and then renamed, so the folder is there whole or not at all.
        let partial = cache.join("tesseract-unpacking");
        let _ = std::fs::remove_dir_all(&partial);
        zip::ZipArchive::new(std::io::Cursor::new(TESSERACT_ZIP))?.extract(&partial)?;
        std::fs::rename(&partial, &dir)?;
        // The copies earlier versions unpacked.
        for entry in std::fs::read_dir(cache)?.flatten() {
            let old = entry.file_name().to_string_lossy().starts_with("tesseract-") && entry.path() != dir;
            if old {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ the game's window

/// A top-level window as the search for the game sees it.
struct Candidate {
    class: String,
    title: String,
    area: i64,
}

/// Which of the visible top-level windows is the game: one whose class is the game's own
/// (DE's engine names it "Warframe..."), else one titled exactly "Warframe" (the launcher is
/// called that too, but it is smaller and gone once the game runs); the largest of them.
fn pick_game(windows: &[Candidate]) -> Option<usize> {
    let largest = |accept: &dyn Fn(&Candidate) -> bool| {
        windows.iter().enumerate().filter(|(_, w)| w.area > 0 && accept(w)).max_by_key(|(_, w)| w.area).map(|(i, _)| i)
    };
    largest(&|w| w.class.starts_with("Warframe")).or_else(|| largest(&|w| w.title == "Warframe"))
}

/// The game's window, if it runs: found by class and title alone, so finding it needs no access to
/// the game's process.
fn game_window() -> Option<HWND> {
    struct Search {
        handles: Vec<HWND>,
        found: Vec<Candidate>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `Search` passed to EnumWindows below, alive for the whole call.
        let search = unsafe { &mut *(data as *mut Search) };
        let mut r: RECT = unsafe { std::mem::zeroed() };
        let mut class = [0u16; 128];
        let mut title = [0u16; 128];
        // SAFETY: plain queries about a window handle Windows just gave us, into buffers of the
        // lengths passed.
        unsafe {
            if IsWindowVisible(hwnd) == 0 || GetWindowRect(hwnd, &mut r) == 0 {
                return 1;
            }
            let class_len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
            search.handles.push(hwnd);
            search.found.push(Candidate {
                class: String::from_utf16_lossy(&class[..class_len.min(class.len())]),
                title: String::from_utf16_lossy(&title[..title_len.min(title.len())]),
                area: (r.right - r.left) as i64 * (r.bottom - r.top) as i64,
            });
        }
        1 // keep going
    }
    let mut search = Search { handles: Vec::new(), found: Vec::new() };
    // SAFETY: the callback only runs during this call, while `search` is alive.
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
    }
    pick_game(&search.found).map(|i| search.handles[i])
}

/// Warframe's client area in screen pixels, without window borders.
pub fn game_rect() -> Option<Rect> {
    let hwnd = game_window()?;
    let mut r: RECT = unsafe { std::mem::zeroed() };
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY: a window handle and writable structures for the client area and screen origin.
    if unsafe { GetClientRect(hwnd, &mut r) == 0 || ClientToScreen(hwnd, &mut origin) == 0 } {
        return None;
    }
    let (width, height) = (r.right - r.left, r.bottom - r.top);
    (width > 0 && height > 0).then_some(Rect { x: origin.x, y: origin.y, width, height })
}

/// The monitor the game is on, or the primary one without a game.
fn monitor() -> HMONITOR {
    // SAFETY: MonitorFromWindow/Point never fail with a DEFAULTTO flag.
    unsafe {
        match game_window() {
            Some(h) => MonitorFromWindow(h, MONITOR_DEFAULTTOPRIMARY),
            None => MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
        }
    }
}

/// The display scale of the game's monitor (1.0 at 100%, 1.5 at 150%), so overlays drawn in
/// CSS pixels come out the same size they do on a 100% screen. The monitor's own scale: the
/// game's window may not be DPI aware, and then Windows reports 100% for it whatever the screen.
pub fn game_scale() -> f64 {
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: a monitor handle Windows gave us and two out-parameters.
    let ok = unsafe { GetDpiForMonitor(monitor(), MDT_EFFECTIVE_DPI, &mut x, &mut y) } == 0;
    if ok && x > 0 { x as f64 / 96.0 } else { 1.0 }
}

/// The monitor the game is on (the primary one without a game), in screen pixels.
pub fn game_monitor() -> Rect {
    // SAFETY: GetMonitorInfoW fills a struct whose size we set.
    unsafe {
        let monitor = monitor();
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Rect { x: 0, y: 0, width: 1920, height: 1080 };
        }
        let r = info.rcMonitor;
        Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }
    }
}

// ------------------------------------------------------------------ the screenshot

/// Copies the game's client area off the screen, or the primary monitor if the game is not up.
///
/// GDI sees what the desktop shows, which includes a game in borderless or windowed mode. A game
/// in exclusive fullscreen bypasses the desktop and comes out black: the README says so.
pub fn capture() -> Result<Rgb> {
    let m = game_rect().unwrap_or_else(game_monitor);
    let (w, h) = (m.width, m.height);
    if w <= 0 || h <= 0 {
        bail!("no monitor to take a screenshot of");
    }
    let (w_usize, h_usize) = (usize::try_from(w)?, usize::try_from(h)?);
    let pixels = w_usize.checked_mul(h_usize).ok_or_else(|| anyhow::anyhow!("capture size overflow"))?;
    let bgra_len = pixels.checked_mul(4).ok_or_else(|| anyhow::anyhow!("capture size overflow"))?;
    let rgb_len = pixels.checked_mul(3).ok_or_else(|| anyhow::anyhow!("capture size overflow"))?;
    // SAFETY: the usual GDI sequence. Every non-null handle created here is released before
    // returning, and `bits` points at `bgra_len` bytes owned by the DIB section until DeleteObject.
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            bail!("could not open the screen device context");
        }
        let dc = CreateCompatibleDC(screen);
        if dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
            bail!("could not create a compatible device context");
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // negative: rows top to bottom, like every other image here
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let ok = !bitmap.is_null() && !bits.is_null() && {
            let old = SelectObject(dc, bitmap);
            if old.is_null() {
                false
            } else {
                let copied = BitBlt(dc, 0, 0, w, h, screen, m.x, m.y, SRCCOPY | CAPTUREBLT) != 0;
                SelectObject(dc, old);
                copied
            }
        };
        let result = if ok {
            // GDI hands out BGRA; the OCR wants RGB.
            let bgra = std::slice::from_raw_parts(bits as *const u8, bgra_len);
            let mut rgb = Vec::with_capacity(rgb_len);
            for px in bgra.chunks_exact(4) {
                rgb.extend_from_slice(&[px[2], px[1], px[0]]);
            }
            Ok(Rgb { width: w_usize, height: h_usize, data: rgb })
        } else {
            Err(anyhow::anyhow!("the screen could not be copied"))
        };
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        result
    }
}

// ------------------------------------------------------------------ hotkeys

/// A key name the hotkey parser understands: letters, digits, '_', '+', '-' and spaces only,
/// as on Linux, so the same settings file works on both.
pub fn validate_key(key: &str) -> Result<()> {
    let ok = key.len() <= 40 && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | ' '));
    if ok { Ok(()) } else { Err(UserError::new(format!("\"{key}\" is not a key name a hotkey can use.")).into()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavour {
    Windows,
}

impl Flavour {
    pub fn label(self) -> &'static str {
        "windows"
    }
}

pub fn detect_flavour() -> Option<Flavour> {
    Some(Flavour::Windows)
}

/// What the Settings page lists: each key and what it does.
pub fn hotkey_lines(_flavour: Option<Flavour>, _port: u16, keys: &[(&str, &str)]) -> Vec<String> {
    keys.iter()
        .filter(|(k, _)| !k.trim().is_empty() && validate_key(k).is_ok())
        .map(|(k, p)| {
            let desc = match *p {
                "riven/scan" => "grade the riven on screen",
                "window/toggle" => "show or hide the window",
                _ => "scan the reward screen",
            };
            format!("{k} -> {desc}")
        })
        .collect()
}

pub fn hotkey_note(_flavour: Option<Flavour>) -> String {
    "The app registers these keys itself while it runs. After changing a key, press Install.".into()
}

/// "ctrl + shift + F9" -> (modifiers, virtual key code).
fn parse_key(key: &str) -> Option<(u32, u32)> {
    let mut mods = MOD_NOREPEAT;
    let mut vk = None;
    for part in key.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let up = part.to_ascii_uppercase();
        match up.as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CONTROL,
            "ALT" => mods |= MOD_ALT,
            "SHIFT" => mods |= MOD_SHIFT,
            "SUPER" | "WIN" | "WINDOWS" | "META" => mods |= MOD_WIN,
            _ if vk.is_some() => return None, // two keys
            _ => {
                vk = Some(match up.as_str() {
                    f if f.len() >= 2
                        && f.starts_with('F')
                        && f[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) =>
                    {
                        0x70 + f[1..].parse::<u32>().ok()? - 1 // VK_F1 ...
                    }
                    c if c.len() == 1 && c.as_bytes()[0].is_ascii_alphanumeric() => c.as_bytes()[0] as u32,
                    "INSERT" => 0x2D,
                    "DELETE" => 0x2E,
                    "HOME" => 0x24,
                    "END" => 0x23,
                    "PAGEUP" | "PRIOR" => 0x21,
                    "PAGEDOWN" | "NEXT" => 0x22,
                    "PAUSE" => 0x13,
                    "SCROLLLOCK" | "SCROLL" => 0x91,
                    _ => return None,
                });
            }
        }
    }
    vk.map(|vk| (mods, vk))
}

/// The thread that owns the registered hotkeys. Windows sends WM_HOTKEY to the thread that
/// registered the key, so that thread waits for messages and nothing else.
struct HotkeyThread {
    id: u32,
    handle: std::thread::JoinHandle<()>,
}

static HOTKEY_THREAD: Mutex<Option<HotkeyThread>> = Mutex::new(None);

/// Registers the scan hotkeys for this session, replacing any registered before. A press calls
/// the same local endpoint as the Linux binds: `POST /api/<path>`.
pub fn register_hotkeys(port: u16, keys: &[(&str, &str)]) -> Result<Vec<String>> {
    let mut wanted: Vec<(u32, u32, String, String)> = Vec::new();
    for (k, p) in keys.iter().filter(|(k, _)| !k.trim().is_empty()) {
        let Some((mods, vk)) = parse_key(k) else {
            return Err(
                UserError::new(format!("\"{k}\" is not a key Windows can register (try F9 or ctrl+F9).")).into()
            );
        };
        wanted.push((mods, vk, k.to_string(), p.to_string()));
    }
    if wanted.is_empty() {
        return Err(UserError::new("Set at least one key first.").into());
    }
    let mut owner = HOTKEY_THREAD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(old) = owner.take() {
        // Wait for the old thread to release its keys before registering replacements.
        if !old.handle.is_finished() && unsafe { PostThreadMessageW(old.id, WM_QUIT, 0, 0) } == 0 {
            *owner = Some(old);
            bail!("could not stop the previous hotkey registration");
        }
        old.handle.join().map_err(|_| anyhow::anyhow!("the previous hotkey thread panicked"))?;
    }
    let (tx, rx) = std::sync::mpsc::channel::<(u32, Vec<String>)>();
    let handle = std::thread::Builder::new().name("hotkeys".into()).spawn(move || {
        // SAFETY: registers keys for this thread and pumps its messages until WM_QUIT; the keys
        // are unregistered before the thread ends.
        unsafe {
            let mut failed = Vec::new();
            for (id, (mods, vk, name, _)) in wanted.iter().enumerate() {
                if RegisterHotKey(std::ptr::null_mut(), id as i32 + 1, *mods, *vk) == 0 {
                    failed.push(name.clone());
                }
            }
            if tx.send((GetCurrentThreadId(), failed)).is_err() {
                for id in 0..wanted.len() {
                    UnregisterHotKey(std::ptr::null_mut(), id as i32 + 1);
                }
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_HOTKEY {
                    if let Some((_, _, _, path)) = wanted.get(msg.wParam.wrapping_sub(1)) {
                        let path = path.clone();
                        // Off this thread: a scan takes a second, and keys must stay responsive.
                        std::thread::spawn(move || {
                            if let Err(e) = crate::net::local::call(port, &path, true, Duration::from_secs(60)) {
                                eprintln!("hotkey request failed: {e:#}");
                            }
                        });
                    }
                }
            }
            for id in 0..wanted.len() {
                UnregisterHotKey(std::ptr::null_mut(), id as i32 + 1);
            }
        }
    })?;
    let (tid, failed) = rx.recv_timeout(Duration::from_secs(5))?;
    *owner = Some(HotkeyThread { id: tid, handle });
    if !failed.is_empty() {
        return Err(UserError::new(format!(
            "Another program already uses {}. Pick another key and press Install again.",
            failed.join(" and ")
        ))
        .into());
    }
    Ok(keys.iter().filter(|(k, _)| !k.trim().is_empty()).map(|(k, p)| format!("{k} -> {p}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_is_found_by_its_window_alone() {
        let w = |class: &str, title: &str, area| Candidate { class: class.into(), title: title.into(), area };
        let windows = [
            w("Chrome_WidgetWin_1", "Warframe - wiki - Browser", 2_000_000),
            w("LauncherWindow", "Warframe", 400_000),
            w("WarframePublicEvolutionGfxD3D12", "Warframe", 3_686_400),
        ];
        assert_eq!(pick_game(&windows), Some(2), "the game's own class wins");
        assert_eq!(pick_game(&windows[..2]), Some(1), "else a window titled exactly Warframe");
        assert_eq!(pick_game(&windows[..1]), None);
    }

    #[test]
    fn key_names() {
        assert_eq!(parse_key("F9"), Some((MOD_NOREPEAT, 0x78)));
        assert_eq!(parse_key("ctrl + shift + f10"), Some((MOD_NOREPEAT | MOD_CONTROL | MOD_SHIFT, 0x79)));
        assert_eq!(parse_key("SUPER+R"), Some((MOD_NOREPEAT | MOD_WIN, b'R' as u32)));
        assert_eq!(parse_key("F9+F10"), None);
        assert_eq!(parse_key("F25"), None);
    }
}
