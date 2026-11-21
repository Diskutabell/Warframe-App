//! A Windows overlay window: a plain layered popup that shows the pixels `crate::overlay` drew.
//!
//! Made directly with Win32 rather than through tao, so nothing else touches its layered state:
//! `UpdateLayeredWindow` (pixels with their own transparency) stops working the moment anything
//! calls `SetLayeredWindowAttributes` on the same window. Its messages arrive through the event
//! loop of the thread that made it (tao's, like the main window's).
//!
//! The styles: layered (per-pixel alpha), transparent (every click goes through to the game),
//! topmost, a tool window (not in the taskbar or Alt+Tab) and never activated (the game keeps
//! the keyboard).

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, IsWindowVisible, MA_NOACTIVATE, RegisterClassW,
    SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::overlay::Drawn;

/// The extended styles every overlay window has.
pub const STYLES: u32 = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A click (there should be none: the window is transparent to them) never activates it.
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as LRESULT;
    }
    // SAFETY: the arguments are the ones Windows passed in.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Premultiplied RGBA (tiny-skia) as the premultiplied BGRA a 32-bit DIB holds.
pub fn bgra(drawn: &Drawn, out: &mut [u8]) {
    for (o, p) in out.chunks_exact_mut(4).zip(drawn.pixels.pixels()) {
        o.copy_from_slice(&[p.blue(), p.green(), p.red(), p.alpha()]);
    }
}

pub struct LayeredWindow {
    hwnd: HWND,
}

impl LayeredWindow {
    pub fn new() -> Result<LayeredWindow> {
        let class = wide("warframeproject_overlay");
        let title = wide("warframeproject overlay");
        // SAFETY: the class and title strings outlive the calls; registering the class twice
        // fails harmlessly (the first registration stays).
        let hwnd = unsafe {
            let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            CreateWindowExW(
                STYLES,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            bail!("the overlay window could not be made: {}", std::io::Error::last_os_error());
        }
        Ok(LayeredWindow { hwnd })
    }

    #[cfg(test)]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn visible(&self) -> bool {
        // SAFETY: the window lives as long as `self`.
        unsafe { IsWindowVisible(self.hwnd) != 0 }
    }

    /// Puts `drawn` on screen at its place, above everything, without taking focus. On failure
    /// the window is hidden: never the last scan's pixels instead.
    pub fn present(&self, drawn: &Drawn) -> Result<()> {
        let result = self.update(drawn);
        // SAFETY: the window lives as long as `self`.
        unsafe {
            if result.is_ok() {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            } else {
                ShowWindow(self.hwnd, SW_HIDE);
            }
        }
        result
    }

    fn update(&self, drawn: &Drawn) -> Result<()> {
        let (w, h) = (drawn.pixels.width() as i32, drawn.pixels.height() as i32);
        // SAFETY: each handle is checked before use and released on every path below; the DIB
        // is written within its w x h x 4 bytes.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                bail!("CreateCompatibleDC failed");
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: rows top to bottom, as the pixels are.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let result = if bitmap.is_null() || bits.is_null() {
                Err(anyhow::anyhow!("CreateDIBSection failed for {w}x{h}"))
            } else {
                bgra(drawn, std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize));
                let old = SelectObject(dc, bitmap);
                if old.is_null() {
                    Err(anyhow::anyhow!("SelectObject failed"))
                } else {
                    let at = POINT { x: drawn.rect.x, y: drawn.rect.y };
                    let size = SIZE { cx: w, cy: h };
                    let from = POINT { x: 0, y: 0 };
                    let blend = BLENDFUNCTION {
                        BlendOp: AC_SRC_OVER as u8,
                        BlendFlags: 0,
                        SourceConstantAlpha: 255,
                        AlphaFormat: AC_SRC_ALPHA as u8,
                    };
                    // No screen DC: the default palette is all a 32-bit picture needs.
                    let ok = UpdateLayeredWindow(
                        self.hwnd,
                        std::ptr::null_mut(),
                        &at,
                        &size,
                        dc,
                        &from,
                        0,
                        &blend,
                        ULW_ALPHA,
                    );
                    SelectObject(dc, old);
                    if ok == 0 {
                        Err(anyhow::anyhow!("UpdateLayeredWindow failed: {}", std::io::Error::last_os_error()))
                    } else {
                        Ok(())
                    }
                }
            };
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            result
        }
    }

    pub fn hide(&self) {
        // SAFETY: the window lives as long as `self`.
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

impl Drop for LayeredWindow {
    fn drop(&mut self) {
        // SAFETY: the window was made by this thread and is destroyed once.
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Rect;
    use crate::overlay::{self, Screen};
    use crate::services::scans::RewardResult;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindowLongW, GetWindowRect,
    };

    #[test]
    fn pixels_become_premultiplied_bgra() {
        let screen =
            Screen { game: Rect { x: 0, y: 0, width: 1920, height: 1080 }, display: 1.0, dpi: 1.0, ui_scale: 1.0 };
        let drawn = overlay::rewards(&RewardResult::sample(1.0), screen, false);
        let mut out = vec![0u8; drawn.pixels.pixels().len() * 4];
        bgra(&drawn, &mut out);
        let (i, p) =
            drawn.pixels.pixels().iter().enumerate().find(|(_, p)| p.alpha() > 0 && p.red() != p.blue()).unwrap();
        assert_eq!(&out[i * 4..i * 4 + 4], &[p.blue(), p.green(), p.red(), p.alpha()]);
        assert!(out.chunks_exact(4).all(|c| c[0] <= c[3] && c[1] <= c[3] && c[2] <= c[3]));
    }

    /// Makes real windows, so it runs where there is a desktop to make them on: the Windows CI
    /// job runs it (`cargo test overlay_window -- --ignored`); under Wine it would open windows
    /// on whatever X display the shell has.
    #[test]
    #[ignore]
    fn overlay_window_shows_without_focus_where_it_is_put() {
        let w = LayeredWindow::new().unwrap();
        // SAFETY: plain queries of a window this test owns.
        let (style, ex) =
            unsafe { (GetWindowLongW(w.hwnd(), GWL_STYLE) as u32, GetWindowLongW(w.hwnd(), GWL_EXSTYLE) as u32) };
        assert_eq!(style & WS_POPUP, WS_POPUP);
        assert_eq!(ex & STYLES, STYLES, "extended styles {ex:#x}");

        let rect_of = || {
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            // SAFETY: as above.
            unsafe { GetWindowRect(w.hwnd(), &mut r) };
            Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }
        };
        let at = |x, y, ui_scale| {
            let screen = Screen { game: Rect { x, y, width: 1920, height: 1080 }, display: 1.0, dpi: 1.0, ui_scale };
            overlay::rewards(&RewardResult::sample(ui_scale), screen, false)
        };
        for drawn in [at(0, 0, 1.0), at(100, 50, 0.85)] {
            w.present(&drawn).unwrap();
            assert!(w.visible());
            assert_eq!(rect_of(), drawn.rect);
            // SAFETY: a plain query.
            assert_ne!(unsafe { GetForegroundWindow() }, w.hwnd(), "the overlay took the focus");
        }
        w.hide();
        assert!(!w.visible());
    }
}
