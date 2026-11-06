//! The tesseract CLI, fed through stdin.
//!
//! Images go in as PGM on stdin and results come back on stdout, so there are no temporary files
//! that two overlapping scans (the F9 hotkey plus the automatic trigger) could share.

use std::process::Command;
use std::time::Duration;

use anyhow::{Result, bail};

use super::image::Gray;
use super::matcher::Word;
use crate::compositor::which;
use crate::process;

/// One word of tesseract's TSV output and its confidence (0-100).
#[derive(Clone, Debug)]
pub struct TsvWord {
    pub word: Word,
    pub conf: f64,
}

fn tesseract(img: &Gray, psm: u8, tsv: bool, timeout: Duration) -> Result<String> {
    // On Windows the installer puts it outside PATH; `which` knows where.
    let program = which("tesseract").unwrap_or_else(|| "tesseract".into());
    let mut cmd = Command::new(&program);
    cmd.args(["stdin", "stdout"]);
    // A copy with its language data beside it (the one the Windows exe carries, or the Windows
    // installer's) is told where that is rather than relying on where it was built to look.
    if let Some(data) = program.parent().map(|dir| dir.join("tessdata")).filter(|d| d.is_dir()) {
        cmd.arg("--tessdata-dir").arg(data);
    }
    cmd.args(["--psm", &psm.to_string(), "-l", "eng"]);
    if tsv {
        cmd.arg("tsv");
    }
    // Several passes run in parallel; one thread each keeps the CPU use predictable while the
    // game is running, instead of every pass spinning up an OpenMP pool.
    cmd.env("OMP_THREAD_LIMIT", "1");
    let out = process::run(&mut cmd, Some(img.to_pgm()), timeout)?;
    if !out.success {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("tesseract failed: {}", err.lines().last().unwrap_or("").trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Every recognised word with its box, in tesseract's reading order.
pub fn words(img: &Gray, psm: u8) -> Result<Vec<TsvWord>> {
    let out = tesseract(img, psm, true, Duration::from_secs(30))?;
    Ok(parse_tsv(&out))
}

pub fn parse_tsv(out: &str) -> Vec<TsvWord> {
    out.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
            let text = f.get(11)?.trim();
            if text.is_empty() {
                return None;
            }
            let conf: f64 = f[10].trim().parse().ok()?;
            let num = |i: usize| f[i].trim().parse::<i32>().ok();
            let (left, top, width, height) = (num(6)?, num(7)?, num(8)?, num(9)?);
            Some(TsvWord { word: Word { text: text.to_string(), left, right: left + width, top, height }, conf })
        })
        .collect()
}

/// Plain text, for the riven card.
pub fn text(img: &Gray, psm: u8) -> Result<String> {
    tesseract(img, psm, false, Duration::from_secs(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_rows() {
        let out = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t4402\t836\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t100\t40\t80\t22\t91.5\tBraton \n\
                   5\t1\t1\t1\t1\t2\t190\t40\t60\t22\tbad\tskip\n";
        let w = parse_tsv(out);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].word, Word { text: "Braton".into(), left: 100, right: 180, top: 40, height: 22 });
        assert_eq!(w[0].conf, 91.5);
    }
}
