//! Windows system tray icon (`Shell_NotifyIconW`) with a left-click toggle and right-click menu.
//!
//! Runs a tiny hidden message-only window on its own thread so tray events stay independent of
//! tao's window loop and toggling "Minimize to system tray" in Settings adds or removes the icon
//! live without restarting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW, HICON, HWND_MESSAGE,
    ICONINFO, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_USER, WNDCLASSW,
};

use crate::app::App;
use crate::tray_icon;

const WM_TRAY: u32 = WM_USER + 1;
const CMD_TOGGLE: usize = 1;
const CMD_REWARDS: usize = 2;
const CMD_RIVEN: usize = 3;
const CMD_QUIT: usize = 4;

struct TrayCtx {
    state: Arc<App>,
    visible: Arc<AtomicBool>,
}

struct RunningTray {
    hwnd: usize,
    thread: std::thread::JoinHandle<()>,
}

pub struct Tray {
    state: Arc<App>,
    visible: Arc<AtomicBool>,
    running: Option<RunningTray>,
}

impl Tray {
    pub fn new(state: Arc<App>, enabled: bool, window_visible: bool) -> Self {
        let mut tray = Self { state, visible: Arc::new(AtomicBool::new(window_visible)), running: None };
        tray.set_enabled(enabled, window_visible);
        tray
    }

    pub fn set_enabled(&mut self, enabled: bool, window_visible: bool) {
        self.visible.store(window_visible, Ordering::Release);
        if !enabled {
            self.stop();
            return;
        }
        if self.running.is_some() {
            return;
        }
        let state = Arc::clone(&self.state);
        let visible = Arc::clone(&self.visible);
        let (tx, rx) = std::sync::mpsc::channel::<usize>();
        let Ok(thread) = std::thread::Builder::new().name("tray".into()).spawn(move || {
            run_tray_thread(state, visible, tx);
        }) else {
            return;
        };
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(hwnd) if hwnd != 0 => self.running = Some(RunningTray { hwnd, thread }),
            _ => {
                let _ = thread.join();
            }
        }
    }

    pub fn set_window_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Release);
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            // SAFETY: `running.hwnd` is the message-only window owned by `running.thread`.
            unsafe {
                PostMessageW(running.hwnd as HWND, WM_CLOSE, 0, 0);
            }
            let _ = running.thread.join();
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn create_icon() -> HICON {
    let size = tray_icon::SIZE as i32;
    let bgra = tray_icon::bgra32();
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size,
        biHeight: -size,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { std::mem::zeroed() }
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let color =
        unsafe { CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0) };
    if color.is_null() || bits.is_null() {
        if !color.is_null() {
            unsafe { DeleteObject(color) };
        }
        return std::ptr::null_mut();
    }
    // SAFETY: `bits` points to `size * size * 4` writable bytes owned by `color`.
    unsafe {
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
    }
    let mask_bytes = vec![0u8; ((tray_icon::SIZE * tray_icon::SIZE) / 8) as usize];
    let mask = unsafe { CreateBitmap(size, size, 1, 1, mask_bytes.as_ptr() as *const core::ffi::c_void) };
    let icon = if mask.is_null() {
        std::ptr::null_mut()
    } else {
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let h = unsafe { CreateIconIndirect(&ii) };
        unsafe { DeleteObject(mask) };
        h
    };
    unsafe { DeleteObject(color) };
    icon
}

fn run_tray_thread(state: Arc<App>, visible: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<usize>) {
    let class_name = wide("warframeproject_tray");
    // SAFETY: registers a thread-local message-only window class, creates a hidden HWND_MESSAGE
    // window, attaches our Box<TrayCtx> for the duration of the loop, and cleans up before return.
    unsafe {
        let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), lpszClassName: class_name.as_ptr(), ..std::mem::zeroed() };
        let _ = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if hwnd.is_null() {
            let _ = ready.send(0);
            return;
        }
        let ctx = Box::into_raw(Box::new(TrayCtx { state, visible }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx as isize);

        let hicon = create_icon();
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = hicon;
        let tip = wide("warframeproject");
        let n = tip.len().min(nid.szTip.len());
        nid.szTip[..n].copy_from_slice(&tip[..n]);

        Shell_NotifyIconW(NIM_ADD, &nid);
        let _ = ready.send(hwnd as usize);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Shell_NotifyIconW(NIM_DELETE, &nid);
        if !hicon.is_null() {
            DestroyIcon(hicon);
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        drop(Box::from_raw(ctx));
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lparam as u32) & 0xffff;
            // SAFETY: `GWLP_USERDATA` holds the live `TrayCtx` until after the message loop exits.
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const TrayCtx;
            if !ptr.is_null() {
                let ctx = unsafe { &*ptr };
                if event == WM_LBUTTONUP {
                    ctx.state.toggle_window();
                } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    unsafe { show_menu(hwnd, ctx) };
                }
            }
            0
        }
        WM_CLOSE => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn show_menu(hwnd: HWND, ctx: &TrayCtx) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    let toggle_label = wide(if ctx.visible.load(Ordering::Acquire) { "Hide window" } else { "Show window" });
    let rewards_label = wide("Scan reward screen");
    let riven_label = wide("Grade riven");
    let quit_label = wide("Quit");

    unsafe {
        AppendMenuW(menu, MF_STRING, CMD_TOGGLE, toggle_label.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, CMD_REWARDS, rewards_label.as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_RIVEN, riven_label.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, CMD_QUIT, quit_label.as_ptr());

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Required by Win32 so clicking outside the popup dismisses it reliably.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);

        match cmd {
            CMD_TOGGLE => ctx.state.toggle_window(),
            CMD_REWARDS => ctx.state.scan_rewards_later(),
            CMD_RIVEN => ctx.state.scan_riven_later(),
            CMD_QUIT => ctx.state.quit(),
            _ => {}
        }
    }
}
