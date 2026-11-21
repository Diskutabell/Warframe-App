//! The game's monitor copied straight from the compositor (wlr-screencopy, which Hyprland, Sway
//! and the other wlroots-style compositors offer), as grim does it, without starting grim: one
//! Wayland connection, one shared-memory buffer, no PPM to encode and parse.
//!
//! Anything this cannot do (no such protocol, a pixel format not handled here, several monitors
//! and none named) is an error, and the caller falls back to the grim program.

use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

use super::image::Rgb;

/// A frame takes one or two monitor refreshes; anything near this means the compositor is stuck.
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
struct Layout {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(Default)]
struct State {
    /// Each output and its name (`wl_output` version 4 sends it).
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    /// The shared-memory layout the compositor offers for the frame.
    layout: Option<Layout>,
    /// Version 3 lists every buffer type first and says when the list is done.
    listed: bool,
    upside_down: bool,
    ready: bool,
    failed: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = event {
            if let Some(o) = state.outputs.get_mut(*index) {
                o.1 = Some(name);
            }
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
        match event {
            Event::Buffer { format: WEnum::Value(format), width, height, stride } => {
                state.layout = Some(Layout { format, width, height, stride });
            }
            Event::BufferDone => state.listed = true,
            Event::Flags { flags: WEnum::Value(f) } => {
                state.upside_down = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
            }
            Event::Ready { .. } => state.ready = true,
            Event::Failed => state.failed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);

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
        let n = unsafe { libc::poll(&mut fd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
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

/// The output named `name` (the game's monitor), or the only one there is.
pub fn capture(name: Option<&str>) -> Result<Rgb> {
    let deadline = Instant::now() + TIMEOUT;
    let conn = Connection::connect_to_env().context("no Wayland session")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let mut state = State::default();
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 =
        globals.bind(&qh, 1..=3, ()).context("the compositor has no wlr-screencopy")?;
    for g in globals.contents().clone_list().into_iter().filter(|g| g.interface == "wl_output") {
        let index = state.outputs.len();
        let output = globals.registry().bind::<wl_output::WlOutput, _, _>(g.name, g.version.min(4), &qh, index);
        state.outputs.push((output, None));
    }
    let output = match (name, state.outputs.len()) {
        (_, 0) => bail!("no outputs"),
        (None, 1) => state.outputs[0].0.clone(),
        (None, _) => bail!("several outputs and none named"),
        (Some(name), _) => {
            // The names arrive with the outputs' first events.
            let n = state.outputs.len();
            dispatch_until(&conn, &mut queue, &mut state, deadline, |s| s.outputs.iter().filter(|o| o.1.is_some()).count() == n)
                .context("the outputs' names")?;
            state.outputs.iter().find(|o| o.1.as_deref() == Some(name)).map(|o| o.0.clone()).with_context(|| format!("no output {name}"))?
        }
    };

    let frame = manager.capture_output(0, &output, &qh, ());
    let v3 = manager.version() >= 3;
    dispatch_until(&conn, &mut queue, &mut state, deadline, |s| s.failed || (s.layout.is_some() && (!v3 || s.listed)))?;
    if state.failed {
        bail!("the compositor refused the capture");
    }
    let l = state.layout.context("the compositor offered no shared-memory buffer")?;
    let order = match l.format {
        // Little-endian 32-bit pixels: XRGB is B, G, R, X in memory, XBGR is R, G, B, X.
        wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => [2, 1, 0],
        wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => [0, 1, 2],
        other => bail!("pixel format {other:?} is not handled"),
    };
    if l.stride < l.width * 4 || l.width == 0 || l.height == 0 {
        bail!("odd buffer layout {l:?}");
    }
    let size = l.stride as u64 * l.height as u64;
    let file = shared_file(size)?;
    let pool = shm.create_pool(file.as_fd(), i32::try_from(size)?, &qh, ());
    let buffer = pool.create_buffer(0, l.width as i32, l.height as i32, l.stride as i32, l.format, &qh, ());
    frame.copy(&buffer);
    let copied = dispatch_until(&conn, &mut queue, &mut state, deadline, |s| s.ready || s.failed);
    frame.destroy();
    buffer.destroy();
    pool.destroy();
    let _ = conn.flush();
    copied?;
    if state.failed {
        bail!("the compositor could not copy the frame");
    }

    let mut raw = vec![0u8; size as usize];
    file.read_exact_at(&mut raw, 0)?;
    let (w, h, stride) = (l.width as usize, l.height as usize, l.stride as usize);
    let mut data = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = if state.upside_down { h - 1 - y } else { y };
        for px in raw[row * stride..row * stride + w * 4].chunks_exact(4) {
            data.extend_from_slice(&[px[order[0]], px[order[1]], px[order[2]]]);
        }
    }
    Ok(Rgb { width: w, height: h, data })
}
