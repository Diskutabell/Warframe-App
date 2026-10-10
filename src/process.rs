//! Running helper programs (tesseract, grim, hyprctl...) with a deadline.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `cmd`, feeding `input` on stdin, and collects its output. The process is killed once
/// `timeout` passes, so a hung tool cannot hang the scan or request that called it.
pub fn run(cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration) -> Result<Output> {
    let mut child = cmd
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("could not run {:?}: {e}", cmd.get_program()))?;
    // Separate threads for stdin and both pipes: a tool that writes a lot before it has read all
    // of its input would otherwise deadlock against us.
    let writer = input.zip(child.stdin.take()).map(|(data, mut stdin)| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        })
    });
    let mut out_pipe = child.stdout.take().expect("piped");
    let mut err_pipe = child.stderr.take().expect("piped");
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{:?} did not finish within {} s", cmd.get_program(), timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if let Some(w) = writer {
        let _ = w.join();
    }
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    Ok(Output { success: status.success(), stdout, stderr })
}

#[cfg(test)]
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
        assert!(run(Command::new("sleep").arg("5"), None, Duration::from_millis(100)).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
