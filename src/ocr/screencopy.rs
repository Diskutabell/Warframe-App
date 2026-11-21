//! The game's monitor copied straight from the compositor (wlr-screencopy, which Hyprland, Sway
//! and the other wlroots-style compositors offer), as grim does it, without starting grim.
//!
//! A reward scan captures every 120 ms, so the Wayland connection and the shared-memory buffer
//! are kept between captures and only remade after an error or when the frame's size changes.
//! Rows go from the shared memory straight into the RGB frame.
//!
//! What this cannot do is an error, and the caller falls back to the grim program. What can never
//! work in this session (no such protocol, outputs too old to name themselves, a pixel format not
//! handled here) is remembered, so later captures go to grim at once.

use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

use super::image::Rgb;

/// A frame takes one or two monitor refreshes; anything near this means the compositor is stuck.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Something this session's compositor will never do: not worth asking again.
#[derive(Debug)]
struct Unsupported(String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unsupported {}

fn unsupported(why: impl Into<String>) -> anyhow::Error {
    Unsupported(why.into()).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

/// The byte order of a handled pixel format: where red, green and blue sit in each 4 bytes.
fn channels(format: wl_shm::Format) -> Option<[usize; 3]> {
    match format {
        // Little-endian 32-bit pixels: XRGB is B, G, R, X in memory, XBGR is R, G, B, X.
        wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => Some([2, 1, 0]),
        wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => Some([0, 1, 2]),
        _ => None,
    }
}

struct Output {
    global: u32,
    proxy: wl_output::WlOutput,
    version: u32,
    name: Option<String>,
    transform: Option<wl_output::Transform>,
}

#[derive(Default)]
struct State {
    /// The globals: (name, interface, version).
    globals: Vec<(u32, String, u32)>,
    outputs: Vec<Output>,
    /// Answers to `wl_display.sync`, counted.
    synced: u32,
    frame: Frame,
}

/// What the compositor said about the frame being copied.
#[derive(Default)]
struct Frame {
    /// The first shared-memory layout offered in a format handled here.
    layout: Option<Layout>,
    /// Any shared-memory layout was offered (perhaps in another format).
    offered: Option<WEnum<wl_shm::Format>>,
    /// Version 3 lists every buffer type first and says when the list is done.
    listed: bool,
    upside_down: bool,
    ready: bool,
    failed: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                // Outputs are bound as they come, monitors plugged in later too.
                if interface == wl_output::WlOutput::interface().name {
                    let v = version.min(4);
                    let proxy = registry.bind::<wl_output::WlOutput, _, _>(name, v, qh, name);
                    state.outputs.push(Output { global: name, proxy, version: v, name: None, transform: None });
                }
                state.globals.push((name, interface, version));
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.globals.retain(|g| g.0 != name);
                state.outputs.retain(|o| o.global != name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = state.outputs.iter_mut().find(|o| o.global == *global) else { return };
        match event {
            wl_output::Event::Name { name } => o.name = Some(name),
            wl_output::Event::Geometry { transform: WEnum::Value(t), .. } => o.transform = Some(t),
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.synced += 1;
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event;
        let f = &mut state.frame;
        match event {
            Event::Buffer { format, width, height, stride } => {
                f.offered = Some(format);
                if let WEnum::Value(format) = format {
                    if f.layout.is_none() && channels(format).is_some() {
                        f.layout = Some(Layout { format, width, height, stride });
                    }
                }
            }
            Event::BufferDone => f.listed = true,
            Event::Flags { flags: WEnum::Value(flags) } => {
                f.upside_down = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
            }
            Event::Ready { .. } => f.ready = true,
            Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);

/// A shared-memory buffer the compositor copies frames into, kept while the layout stays.
struct Shm {
    layout: Layout,
    file: File,
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
}

impl Drop for Shm {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// A shared-memory file of `size` bytes for the compositor to write into.
fn shared_file(size: u64) -> Result<File> {
    // SAFETY: a constant C string; the result is checked before use.
    let fd = unsafe { libc::memfd_create(c"warframeproject-capture".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("memfd_create");
    }
    // SAFETY: a new descriptor that nothing else owns.
    let file = unsafe { File::from_raw_fd(fd) };
    file.set_len(size)?;
    Ok(file)
}

struct Session {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    shm: wl_shm::WlShm,
    manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    buffer: Option<Shm>,
}

impl Session {
    fn connect(deadline: Instant) -> Result<Session> {
        let conn = Connection::connect_to_env().map_err(|e| unsupported(format!("no Wayland session: {e}")))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut state = State::default();
        // Every global, then every output's first events (name, transform).
        Self::sync(&conn, &mut queue, &mut state, deadline)?;
        Self::sync(&conn, &mut queue, &mut state, deadline)?;
        let global =
            |state: &State, interface: &str| state.globals.iter().find(|g| g.1 == interface).map(|g| (g.0, g.2));
        let (shm_name, _) = global(&state, wl_shm::WlShm::interface().name).ok_or_else(|| unsupported("no wl_shm"))?;
        let (manager_name, manager_version) =
            global(&state, zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1::interface().name)
                .ok_or_else(|| unsupported("the compositor has no wlr-screencopy"))?;
        let shm = registry.bind(shm_name, 1, &qh, ());
        let manager = registry.bind(manager_name, manager_version.min(3), &qh, ());
        Ok(Session { conn, queue, state, shm, manager, buffer: None })
    }

    /// Reads and handles events until `done` says so, or the time is up.
    fn dispatch_until(
        conn: &Connection,
        queue: &mut EventQueue<State>,
        state: &mut State,
        deadline: Instant,
        done: impl Fn(&State) -> bool,
    ) -> Result<()> {
        loop {
            queue.dispatch_pending(state)?;
            if done(state) {
                return Ok(());
            }
            conn.flush()?;
            let Some(guard) = queue.prepare_read() else { continue };
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!("the compositor did not answer in time");
            }
            let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd, and its count.
            let n = unsafe { libc::poll(&mut fd, 1, left.as_millis().clamp(1, i32::MAX as u128) as i32) };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::Interrupted {
                    return Err(e.into());
                }
            } else if n > 0 {
                guard.read()?;
            }
        }
    }

    /// One round trip: everything the compositor had to say before it is handled.
    fn sync(conn: &Connection, queue: &mut EventQueue<State>, state: &mut State, deadline: Instant) -> Result<()> {
        let target = state.synced + 1;
        conn.display().sync(&queue.handle(), ());
        Self::dispatch_until(conn, queue, state, deadline, |s| s.synced >= target)
    }

    fn capture(&mut self, name: Option<&str>, deadline: Instant) -> Result<Rgb> {
        // Monitors plugged in or out since the last capture.
        Self::sync(&self.conn, &mut self.queue, &mut self.state, deadline)?;
        let outputs = &self.state.outputs;
        if outputs.iter().any(|o| o.version < 4) {
            return Err(unsupported("outputs too old to tell their names"));
        }
        let output = match (name, outputs.len()) {
            (_, 0) => bail!("no outputs"),
            (None, 1) => &outputs[0],
            (None, _) => bail!("several outputs and none named"),
            (Some(name), _) => {
                outputs.iter().find(|o| o.name.as_deref() == Some(name)).with_context(|| format!("no output {name}"))?
            }
        };
        // A turned or mirrored monitor's frame comes as it is scanned out; grim turns it back.
        if output.transform.is_some_and(|t| t != wl_output::Transform::Normal) {
            bail!("output {} is rotated", output.name.as_deref().unwrap_or("?"));
        }
        let proxy = output.proxy.clone();

        let qh = self.queue.handle();
        self.state.frame = Frame::default();
        let frame = self.manager.capture_output(0, &proxy, &qh, ());
        let result = self.copy(&frame, deadline);
        frame.destroy();
        let _ = self.conn.flush();
        result
    }

    fn copy(&mut self, frame: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, deadline: Instant) -> Result<Rgb> {
        let v3 = self.manager.version() >= 3;
        Self::dispatch_until(&self.conn, &mut self.queue, &mut self.state, deadline, |s| {
            s.frame.failed || if v3 { s.frame.listed } else { s.frame.offered.is_some() }
        })?;
        let f = &self.state.frame;
        if f.failed {
            bail!("the compositor refused the capture");
        }
        let Some(l) = f.layout else {
            return Err(match f.offered {
                Some(format) => unsupported(format!("pixel format {format:?} is not handled")),
                None => unsupported("the compositor offered no shared-memory buffer"),
            });
        };
        let (w, h, stride) = (u64::from(l.width), u64::from(l.height), u64::from(l.stride));
        let size = stride * h;
        if w == 0 || h == 0 || stride < w * 4 || size > i32::MAX as u64 {
            bail!("odd buffer layout {l:?}");
        }
        if self.buffer.as_ref().is_none_or(|b| b.layout != l) {
            self.buffer = None;
            let file = shared_file(size)?;
            let qh = self.queue.handle();
            let pool = self.shm.create_pool(file.as_fd(), size as i32, &qh, ());
            let buffer = pool.create_buffer(0, l.width as i32, l.height as i32, l.stride as i32, l.format, &qh, ());
            self.buffer = Some(Shm { layout: l, file, pool, buffer });
        }
        let shm = self.buffer.as_ref().ok_or_else(|| anyhow!("no buffer"))?;
        frame.copy(&shm.buffer);
        Self::dispatch_until(&self.conn, &mut self.queue, &mut self.state, deadline, |s| {
            s.frame.ready || s.frame.failed
        })?;
        if self.state.frame.failed {
            bail!("the compositor could not copy the frame");
        }

        let order = channels(l.format).ok_or_else(|| anyhow!("format"))?;
        let (w, h, stride) = (w as usize, h as usize, stride as usize);
        let mut data = Vec::with_capacity(w * h * 3);
        let mut row = vec![0u8; w * 4];
        for y in 0..h {
            let from = if self.state.frame.upside_down { h - 1 - y } else { y };
            shm.file.read_exact_at(&mut row, (from * stride) as u64)?;
            for px in row.chunks_exact(4) {
                data.extend_from_slice(&[px[order[0]], px[order[1]], px[order[2]]]);
            }
        }
        Ok(Rgb { width: w, height: h, data })
    }
}

/// Whether a capture is worth trying: a Wayland session where it has not failed for good.
pub fn possible() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) && !NEVER.load(Ordering::Relaxed)
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static NEVER: AtomicBool = AtomicBool::new(false);

/// The output named `name` (the game's monitor), or the only one there is.
pub fn capture(name: Option<&str>) -> Result<Rgb> {
    if NEVER.load(Ordering::Relaxed) {
        bail!("wlr-screencopy does not work in this session");
    }
    let deadline = Instant::now() + TIMEOUT;
    let mut session = SESSION.lock().map_err(|_| anyhow!("capture lock poisoned"))?;
    let result = match session.as_mut() {
        Some(s) => s.capture(name, deadline),
        None => Session::connect(deadline).and_then(|s| session.insert(s).capture(name, deadline)),
    };
    if let Err(e) = &result {
        if e.downcast_ref::<Unsupported>().is_some() {
            NEVER.store(true, Ordering::Relaxed);
        }
        // A connection that failed once is not trusted again: the next capture starts afresh.
        *session = None;
    }
    result
}
