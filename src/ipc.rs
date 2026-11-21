//! Talking to the compositor over its own socket, as hyprctl and swaymsg do, without starting
//! either: a query takes a connect and a read instead of a new process.
//!
//! Hyprland: `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` (`/tmp/hypr/...`
//! before 0.40). A request is `<flags>/<command> <args>` - `j/clients` for JSON, `/eval <lua>` -
//! and the answer is everything read until Hyprland closes the connection (checked against
//! hyprctl 0.57.2 with strace).
//!
//! Sway: `$SWAYSOCK`, the i3 IPC protocol: `i3-ipc`, the payload's length and the message type
//! (both u32, native byte order), then the payload; the answer comes framed the same way.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};

const TIMEOUT: Duration = Duration::from_secs(5);
/// No answer of either compositor comes near this; a larger one is not one of theirs.
const MAX_ANSWER: u64 = 64 * 1024 * 1024;

fn connect(path: &PathBuf) -> Result<UnixStream> {
    let stream = UnixStream::connect(path).with_context(|| format!("connecting to {}", path.display()))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    Ok(stream)
}

/// Hyprland's request socket for this session.
fn hyprland_socket() -> Result<PathBuf> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|s| !s.is_empty()).context("not in a Hyprland session")?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let candidates = runtime.into_iter().map(|r| r.join("hypr")).chain([PathBuf::from("/tmp/hypr")]);
    candidates
        .map(|dir| dir.join(&sig).join(".socket.sock"))
        .find(|p| p.exists())
        .context("Hyprland's socket was not found")
}

/// One request to Hyprland, as hyprctl sends it, and its answer.
pub fn hyprland(request: &str) -> Result<String> {
    let mut stream = connect(&hyprland_socket()?)?;
    stream.write_all(request.as_bytes())?;
    let mut answer = Vec::new();
    stream.take(MAX_ANSWER).read_to_end(&mut answer).context("reading Hyprland's answer")?;
    Ok(String::from_utf8_lossy(&answer).into_owned())
}

/// Hyprland's answer to a JSON request (`j/clients`, `j/monitors`).
pub fn hyprland_json(request: &str) -> Result<serde_json::Value> {
    serde_json::from_str(&hyprland(request)?).with_context(|| format!("Hyprland's answer to {request} is not JSON"))
}

/// The i3 IPC message types used here.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum SwayMessage {
    RunCommand = 0,
    GetOutputs = 3,
    GetTree = 4,
}

const MAGIC: &[u8; 6] = b"i3-ipc";

fn sway_frame(kind: SwayMessage, payload: &str) -> Vec<u8> {
    let mut msg = Vec::with_capacity(14 + payload.len());
    msg.extend_from_slice(MAGIC);
    msg.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    msg.extend_from_slice(&(kind as u32).to_ne_bytes());
    msg.extend_from_slice(payload.as_bytes());
    msg
}

/// Reads one framed answer.
fn read_sway_answer(r: &mut impl Read) -> Result<Vec<u8>> {
    let mut head = [0u8; 14];
    r.read_exact(&mut head).context("reading Sway's answer")?;
    if &head[..6] != MAGIC {
        bail!("Sway answered in an unknown format");
    }
    let len = u32::from_ne_bytes([head[6], head[7], head[8], head[9]]) as u64;
    if len > MAX_ANSWER {
        bail!("Sway's answer is too large");
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).context("reading Sway's answer")?;
    Ok(body)
}

/// One message to Sway and its answer (JSON).
pub fn sway(kind: SwayMessage, payload: &str) -> Result<serde_json::Value> {
    let path = std::env::var_os("SWAYSOCK").filter(|s| !s.is_empty()).map(PathBuf::from).context("not in a Sway session")?;
    let mut stream = connect(&path)?;
    stream.write_all(&sway_frame(kind, payload))?;
    let body = read_sway_answer(&mut stream)?;
    serde_json::from_slice(&body).context("Sway's answer is not JSON")
}

/// Runs Sway commands; an error names the first one Sway refused.
pub fn sway_command(command: &str) -> Result<()> {
    let answer = sway(SwayMessage::RunCommand, command)?;
    for result in answer.as_array().into_iter().flatten() {
        if result["success"] != true {
            bail!("Sway refused \"{command}\": {}", result["error"].as_str().unwrap_or("no reason given"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sway_messages_are_framed_as_i3_ipc() {
        let frame = sway_frame(SwayMessage::GetTree, "");
        assert_eq!(&frame[..6], b"i3-ipc");
        assert_eq!(u32::from_ne_bytes(frame[6..10].try_into().unwrap()), 0);
        assert_eq!(u32::from_ne_bytes(frame[10..14].try_into().unwrap()), 4);
        let mut answer = sway_frame(SwayMessage::RunCommand, r#"[{"success":true}]"#);
        let body = read_sway_answer(&mut answer.as_slice()).unwrap();
        assert_eq!(body, br#"[{"success":true}]"#);
        answer[0] = b'x';
        assert!(read_sway_answer(&mut answer.as_slice()).is_err());
    }

    #[test]
    fn a_hyprland_answer_is_read_until_the_socket_closes() {
        let dir = std::env::temp_dir().join(format!("wfp-ipc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".socket.sock");
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 9];
            s.read_exact(&mut req).unwrap();
            assert_eq!(&req, b"j/clients");
            s.write_all(b"[{\"class\":\"steam_app_230410\"}]").unwrap();
        });
        let mut stream = connect(&path).unwrap();
        stream.write_all(b"j/clients").unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        server.join().unwrap();
        assert_eq!(answer, "[{\"class\":\"steam_app_230410\"}]");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
