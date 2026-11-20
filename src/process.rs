//! Running helper programs (tesseract, grim, hyprctl...) with a deadline.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

/// Generous cap for raw screenshots (`grim` PPM) while preventing unbounded OOM from a runaway tool.
const MAX_STDOUT: u64 = 128 * 1024 * 1024;
const MAX_STDERR: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Ensures a spawned child is killed and reaped on any early return (timeout, `try_wait` error,
/// or panic) so no orphan or zombie is left behind.
struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn fmt_timeout(d: Duration) -> String {
    if d.subsec_nanos() == 0 { format!("{} s", d.as_secs()) } else { format!("{:.2} s", d.as_secs_f64()) }
}

/// Runs `cmd`, feeding `input` on stdin, and collects its output. The process is killed once
/// `timeout` passes, so a hung tool cannot hang the scan or request that called it.
pub fn run(cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration) -> Result<Output> {
    #[cfg(windows)]
    {
        // Without this every tesseract run opens a console window for a moment.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("could not run {:?}: {e}", cmd.get_program()))?;
    let mut guard = ChildGuard(Some(child));
    let child = guard.0.as_mut().expect("child present");
    // Separate threads for stdin and both pipes: a tool that writes a lot before it has read all
    // of its input would otherwise deadlock against us. Channels let us bound waiting if a
    // grandchild inherits stdout/stderr and keeps the pipe open after the child exits.
    let writer = input.zip(child.stdin.take()).map(|(data, mut stdin)| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
            drop(stdin);
            let _ = tx.send(());
        });
        rx
    });
    let out_pipe = child.stdout.take().expect("piped");
    let err_pipe = child.stderr.take().expect("piped");
    let (out_tx, out_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.take(MAX_STDOUT).read_to_end(&mut buf);
        let _ = out_tx.send(buf);
    });
    let (err_tx, err_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.take(MAX_STDERR).read_to_end(&mut buf);
        let _ = err_tx.send(buf);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = guard.0.as_mut().expect("child present").try_wait()? {
            guard.0 = None;
            break status;
        }
        if Instant::now() > deadline {
            drop(guard);
            bail!("{:?} did not finish within {}", cmd.get_program(), fmt_timeout(timeout));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let wait_budget = || deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(50));
    if let Some(w) = writer {
        let _ = w.recv_timeout(wait_budget());
    }
    let stdout = match out_rx.recv_timeout(wait_budget()) {
        Ok(buf) => buf,
        Err(_) => bail!("{:?} did not close stdout within {}", cmd.get_program(), fmt_timeout(timeout)),
    };
    let stderr = match err_rx.recv_timeout(wait_budget()) {
        Ok(buf) => buf,
        Err(_) => bail!("{:?} did not close stderr within {}", cmd.get_program(), fmt_timeout(timeout)),
    };
    Ok(Output { success: status.success(), stdout, stderr })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn feeds_stdin_and_reads_stdout() {
        let out = run(Command::new("cat").arg("-"), Some(b"hello".to_vec()), Duration::from_secs(5)).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, b"hello");
    }

    #[test]
    fn kills_on_timeout() {
        let started = Instant::now();
        let err = run(Command::new("sleep").arg("5"), None, Duration::from_millis(100)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(err.to_string().contains("0.10 s"), "got: {err}");
    }

    #[test]
    fn does_not_hang_when_grandchild_holds_stdout_open() {
        let started = Instant::now();
        let err = run(Command::new("sh").args(["-c", "sleep 5 &"]), None, Duration::from_millis(150)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2), "hung waiting on grandchild pipe: {err}");
    }
}
