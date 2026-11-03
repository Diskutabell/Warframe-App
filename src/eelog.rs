//! Warframe's own log (EE.log): finding it, what it says about the account, and when the reward
//! screen opens. Plain file reads - nothing here touches the game process.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;

use crate::now;

pub const WF_APPID: &str = "230410";
const LOG_SUFFIX: &str = "pfx/drive_c/users/steamuser/AppData/Local/Warframe/EE.log";

/// Explicit "this is me" lines (older builds). Current builds only print "Logged in <name>".
static ACCOUNT_RES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [Regex::new(r"AccountId:\s*([0-9a-f]{24})").unwrap(), Regex::new(r"Logged in \S+ \(([0-9a-f]{24})\)").unwrap()]
});
static NAME_RES: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"Sys \[Info\]: Logged in (\S+)\s*$").unwrap(),
        Regex::new(r"Logged in (\S+) \(").unwrap(),
        Regex::new(r"Player name changed to (\S+)").unwrap(),
    ]
});
/// The squad overlay loads a mastery badge per slot, and slot 1 is the local player: on the user's
/// log it is `Rank0` every time (an MR 0 account, which they confirmed), while slots 2-4 carry the
/// squadmates' badges, Rank7to9 to Rank28to30. Low ranks get an exact icon, higher ones a band.
static MASTERY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"OnMasteryIconReady1\(Loaded:[^)]*?/Rank(\d+)(?:to(\d+))?_").unwrap());
/// Confirmed on real relic cracks: the screen opens on OpenVoidProjectionRewardScreen (logged
/// with an `RMI` suffix, so this stays a substring test). `Created .../ProjectionRewardChoice.swf`
/// follows 2 ms later and `Got rewards` 0.36 s later; the 8 s debounce keeps them from scanning
/// three times. `Pause countdown done` (wfinfo-ng's trigger) never fires on current builds.
pub const REWARD_TRIGGERS: [&str; 4] = [
    "VoidProjections: OpenVoidProjectionRewardScreen",
    "Pause countdown done",
    "Got rewards",
    "Created /Lotus/Interface/ProjectionRewardChoice.swf",
];
/// Lines that suggest the account state changed on the server.
pub const SYNC_HINTS: [&str; 4] =
    ["EndOfMatch.lua: Mission Succeeded", "EndOfMatch.lua: Mission Failed", "ExitLevel", "Inventory updated"];
const REWARD_DEBOUNCE: f64 = 8.0;

/// Display names carry a trailing platform glyph from the private-use area (U+E000 for PC).
/// "Logged in <name>" prints the bare name and every later line the glyph form, so never compare
/// a log name without stripping it.
pub fn strip_glyphs(name: &str) -> String {
    name.chars().filter(|c| !('\u{e000}'..='\u{f8ff}').contains(c)).collect::<String>().trim().to_string()
}

/// Lower-case 24-hex Mongo ObjectIds: a run of exactly 24 of [0-9a-f]. The upper-case
/// matchmaking GUID (mm=...) is deliberately not one.
pub fn object_ids(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let is_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_hex(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_hex(bytes[i]) {
            i += 1;
        }
        let run = &line[start..i];
        if run.len() == 24 && run.bytes().any(|b| b != b'0') {
            out.push(run.to_string());
        }
    }
    out
}

/// (low, high) mastery rank from the local player's badge; high is None when exact.
pub fn mastery_rank(line: &str) -> Option<(u32, Option<u32>)> {
    let m = MASTERY_RE.captures(line)?;
    let lo = m[1].parse().ok()?;
    Some((lo, m.get(2).and_then(|h| h.as_str().parse().ok())))
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `name` appears in `line` as a whole name, not inside a longer one ("Tenno" must not
/// match "Tenno2"). The platform glyph after a name is not a name character, so it bounds the
/// match like a space does; the name itself is compared without it.
pub fn names_player(line: &str, name: &str) -> bool {
    let name = strip_glyphs(name);
    if name.is_empty() {
        return false;
    }
    line.match_indices(&name).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let after = line[i + name.len()..].chars().next();
        !before.is_some_and(name_char) && !after.is_some_and(name_char)
    })
}

/// Ids in one line, as (id, sure). No network check is ever made on these.
///
/// sure=true only when the line can only be about the local player: it names them and holds
/// exactly one id, or it is the client sending its own fissure reward to the host. Anything else
/// (squadmates' ids appear in the same file) is shown to the user to confirm by eye.
pub fn candidate_ids(line: &str, name: Option<&str>) -> Vec<(String, bool)> {
    let ids = object_ids(line);
    if ids.is_empty() {
        return Vec::new();
    }
    let sure = ids.len() == 1;
    let own_reward = line.contains("VoidProjections: Sending reward info from") && line.contains("to host!");
    if name.is_some_and(|n| names_player(line, n)) || own_reward {
        return ids.into_iter().map(|i| (i, sure)).collect();
    }
    Vec::new()
}

fn player_name(line: &str) -> Option<String> {
    NAME_RES.iter().find_map(|rx| rx.captures(line)).map(|m| strip_glyphs(&m[1]))
}

fn explicit_account(line: &str) -> Option<String> {
    ACCOUNT_RES.iter().find_map(|rx| rx.captures(line)).map(|m| m[1].to_string())
}

/// What an existing log already says, in order, as the live watcher would have reported it:
/// logins, explicit ids, id candidates and the local player's mastery badges. Reward screens and
/// syncs are left out (they are over). The order matters: one game session can log out and in
/// with another account, and everything after a login belongs to that account.
pub fn scan_log(path: &Path) -> Vec<LogEvent> {
    const MAX_BYTES: u64 = 64_000_000;
    let mut events = Vec::new();
    let Ok(mut f) = fs::File::open(path) else { return events };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(size.saturating_sub(MAX_BYTES)));
    let mut raw = Vec::new();
    if f.read_to_end(&mut raw).is_err() {
        return events;
    }
    let text = String::from_utf8_lossy(&raw);
    let mut player: Option<String> = None;
    let mut badge = None;
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        if let Some(n) = player_name(line) {
            if player.as_deref() != Some(n.as_str()) {
                player = Some(n.clone());
                badge = None;
                events.push(LogEvent::Name(n, false));
            }
        }
        if let Some(acc) = explicit_account(line) {
            events.push(LogEvent::Account(acc, player.clone()));
        }
        if let Some(mr) = mastery_rank(line) {
            // The squad overlay redraws the badge many times a session; one report is enough.
            if badge != Some(mr) {
                badge = Some(mr);
                events.push(LogEvent::Mastery(mr.0, mr.1));
            }
        }
        for (id, sure) in candidate_ids(line, player.as_deref()) {
            if seen.insert(id.clone()) {
                events.push(LogEvent::Candidate(id, sure, tail(line.trim(), 200)));
            }
        }
    }
    events
}

fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

// ------------------------------------------------------------------ finding the log

const SKIP_DIRS: [&str; 12] = [
    "dosdevices",
    "windows",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "shadercache",
    "steamapps",
    "node_modules",
    ".git",
    "proc",
    "sys",
    "nix",
];

fn warframe_logs_in_drive_c(drive_c: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for user in fs::read_dir(drive_c.join("users")).into_iter().flatten().flatten() {
        let p = user.path().join("AppData/Local/Warframe/EE.log");
        if p.is_file() {
            out.push(p);
        }
    }
    out
}

/// Wine prefixes under `root`, searched without following symlinks: every prefix has
/// dosdevices/z: -> /, so a symlink-following search would crawl the whole disk.
fn prefix_logs(root: &Path) -> Vec<PathBuf> {
    const MAX_DEPTH: usize = 5;
    const MAX_DIRS: usize = 20_000;
    let (mut found, mut stack, mut seen) = (Vec::new(), vec![(root.to_path_buf(), 0usize)], 0usize);
    while let Some((dir, depth)) = stack.pop() {
        if seen >= MAX_DIRS {
            break;
        }
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue; // file_type() does not follow symlinks
            }
            seen += 1;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "drive_c" {
                found.extend(warframe_logs_in_drive_c(&entry.path()));
            } else if !SKIP_DIRS.contains(&name.as_ref()) && depth < MAX_DEPTH {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    found
}

static LIBRARY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""path"\s+"([^"]+)""#).unwrap());

pub fn candidate_paths() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let roots = [
        ".local/share/Steam",
        ".steam/steam",
        ".steam/root",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
    ];
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in roots.map(|r| home.join(r)) {
        if let Ok(vdf) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            libraries.extend(LIBRARY_RE.captures_iter(&vdf).map(|m| PathBuf::from(&m[1])));
        }
        libraries.push(root);
    }
    libraries.sort();
    libraries.dedup();
    let mut out: Vec<PathBuf> =
        libraries.iter().map(|lib| lib.join("steamapps/compatdata").join(WF_APPID).join(LOG_SUFFIX)).collect();
    // Lutris / umu / Heroic / plain Wine prefixes.
    for root in ["Games", ".local/share/umu", ".var/app/com.heroicgameslauncher.hgl"].map(|r| home.join(r)) {
        if root.is_dir() {
            out.extend(prefix_logs(&root));
        }
    }
    for entry in fs::read_dir(&home).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(".wine") {
            out.extend(warframe_logs_in_drive_c(&entry.path().join("drive_c")));
        }
    }
    out
}

struct Found {
    checked: f64,
    path: Option<PathBuf>,
    searching: bool,
}

static FOUND: Mutex<Found> = Mutex::new(Found { checked: f64::NEG_INFINITY, path: None, searching: false });

fn search() {
    let newest = candidate_paths()
        .into_iter()
        .filter_map(|p| fs::metadata(&p).ok().filter(|m| m.is_file()).map(|m| (m.mtime(), p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p);
    let mut f = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    f.path = newest;
    f.checked = now();
    f.searching = false;
}

/// The newest EE.log on this machine. Cached for a minute and refreshed in the background, so
/// callers (like the web UI) never wait on a disk search.
pub fn find_ee_log() -> Option<PathBuf> {
    let mut f = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    if now() - f.checked > 60.0 && !f.searching {
        f.searching = true;
        std::thread::Builder::new().name("find-eelog".into()).spawn(search).ok();
    }
    f.path.clone()
}

pub fn searching() -> bool {
    FOUND.lock().unwrap_or_else(|e| e.into_inner()).searching
}

// ------------------------------------------------------------------ following the log

pub enum LogEvent {
    /// The player name from a "Logged in" or rename line; true when the line was just written,
    /// false when it was read back from a log that existed when the app started.
    Name(String, bool),
    /// An explicit account id line (older builds), with the player name known at the time.
    Account(String, Option<String>),
    /// An id that may be the player's: (id, sure, the line it came from).
    Candidate(String, bool, String),
    /// The local player's mastery badge: (low, high), high None when exact.
    Mastery(u32, Option<u32>),
    RewardScreen,
    Sync,
    /// The log as it was when the watcher found it has been read back.
    Replayed,
}

/// What the UI shows about the log.
#[derive(Default)]
pub struct LogStatus {
    pub path: Option<PathBuf>,
    pub last_activity: f64,
    pub recent: VecDeque<String>,
}

impl LogStatus {
    pub fn game_active(&self) -> bool {
        now() - self.last_activity < 600.0
    }

    fn note(&mut self, line: &str) {
        self.recent.push_back(tail(line, 300));
        while self.recent.len() > 40 {
            self.recent.pop_front();
        }
    }
}

/// Splits followed bytes into complete lines.
///
/// The game flushes EE.log in arbitrary chunks, not whole lines: watching a live log, 9 of 10
/// reads ended mid-line, one of them inside the 3-byte platform glyph. Handling each piece as a
/// line split triggers in two and once read "Logged in tennoplayer" as the name "tenno". So the
/// bytes after the last newline wait for the rest of their line.
#[derive(Default)]
pub struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    /// Never hold more than this waiting for a newline.
    const MAX_PENDING: usize = 1 << 20;

    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        if let Some(last) = self.pending.iter().rposition(|&b| b == b'\n') {
            let rest = self.pending.split_off(last + 1);
            for raw in self.pending.split(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(raw);
                let line = line.trim_end_matches('\r');
                if !line.is_empty() {
                    lines.push(line.to_string());
                }
            }
            self.pending = rest;
        }
        if self.pending.len() > Self::MAX_PENDING {
            lines.push(String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned());
        }
        lines
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

type PathGetter = Box<dyn Fn() -> Option<PathBuf> + Send>;
type Handler = Box<dyn Fn(LogEvent) + Send>;

pub struct LogWatcher {
    path_of: PathGetter,
    emit: Handler,
    status: Arc<Mutex<LogStatus>>,
    player: Option<String>,
    last_reward: f64,
}

impl LogWatcher {
    pub fn spawn(path_of: PathGetter, emit: Handler) -> Arc<Mutex<LogStatus>> {
        let status = Arc::new(Mutex::new(LogStatus::default()));
        let mut watcher =
            LogWatcher { path_of, emit, status: Arc::clone(&status), player: None, last_reward: f64::NEG_INFINITY };
        std::thread::Builder::new().name("eelog".into()).spawn(move || watcher.run()).expect("spawning eelog");
        status
    }

    fn status(&self) -> std::sync::MutexGuard<'_, LogStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn run(&mut self) {
        let mut current: Option<PathBuf> = None;
        let (mut pos, mut inode) = (0u64, 0u64);
        let mut lines = LineBuffer::default();
        loop {
            let path = (self.path_of)().filter(|p| p.is_file());
            let Some(path) = path else {
                current = None;
                self.status().path = None;
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            if current.as_ref() != Some(&path) {
                self.status().path = Some(path.clone());
                self.replay(&path);
                let meta = fs::metadata(&path).ok();
                pos = meta.as_ref().map_or(0, |m| m.len()); // only follow new lines
                inode = meta.as_ref().map_or(0, |m| m.ino());
                lines.clear();
                current = Some(path.clone());
            }
            if let Ok(meta) = fs::metadata(&path) {
                if meta.ino() != inode || meta.len() < pos {
                    // The game restarted and recreated its log.
                    (pos, inode) = (0, meta.ino());
                    lines.clear();
                }
                if meta.len() > pos {
                    self.status().last_activity = now();
                    let mut chunk = Vec::new();
                    if let Ok(mut f) = fs::File::open(&path) {
                        if f.seek(SeekFrom::Start(pos)).is_ok() {
                            let _ = f.take(4_000_000).read_to_end(&mut chunk);
                        }
                    }
                    pos += chunk.len() as u64;
                    for line in lines.push(&chunk) {
                        self.handle(&line);
                    }
                }
            }
            // The reward choice timer is ~10 s; don't spend half a second of it here.
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Reads what an existing log already says (logins, ids, mastery) when it is first found.
    fn replay(&mut self, path: &Path) {
        let events = scan_log(path);
        // Following the live log goes on with the name the existing part ended on.
        for ev in &events {
            if let LogEvent::Name(name, _) = ev {
                self.player = Some(name.clone());
            }
        }
        for ev in events {
            (self.emit)(ev);
        }
        (self.emit)(LogEvent::Replayed);
    }

    fn handle(&mut self, line: &str) {
        if let Some(name) = player_name(line) {
            if self.player.as_deref() != Some(name.as_str()) {
                self.player = Some(name.clone());
                (self.emit)(LogEvent::Name(name, true));
            }
            self.status().note(line);
        }
        if let Some(acc) = explicit_account(line) {
            (self.emit)(LogEvent::Account(acc, self.player.clone()));
        }
        if let Some((lo, hi)) = mastery_rank(line) {
            (self.emit)(LogEvent::Mastery(lo, hi));
        }
        for (id, sure) in candidate_ids(line, self.player.as_deref()) {
            (self.emit)(LogEvent::Candidate(id, sure, tail(line.trim(), 200)));
        }
        if REWARD_TRIGGERS.iter().any(|t| line.contains(t)) {
            self.status().note(line);
            if now() - self.last_reward > REWARD_DEBOUNCE {
                self.last_reward = now();
                (self.emit)(LogEvent::RewardScreen);
            }
        } else if SYNC_HINTS.iter().any(|h| line.contains(h)) {
            self.status().note(line);
            (self.emit)(LogEvent::Sync);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_cut_in_two_is_read_once_whole() {
        let mut buf = LineBuffer::default();
        assert!(buf.push(b"13.945 Sys [Info]: Logged in tenno").is_empty());
        let lines = buf.push(b"player\r\n1.0 next\r\n");
        assert_eq!(lines, ["13.945 Sys [Info]: Logged in tennoplayer", "1.0 next"]);
        assert_eq!(player_name(&lines[0]).as_deref(), Some("tennoplayer"));
    }

    #[test]
    fn a_glyph_split_across_reads_survives() {
        let mut buf = LineBuffer::default();
        let line = "Sys [Info]: Player name changed to Tenno\u{e000}\n".as_bytes();
        let cut = line.len() - 3; // inside the 3-byte glyph
        assert!(buf.push(&line[..cut]).is_empty());
        let lines = buf.push(&line[cut..]);
        assert_eq!(player_name(&lines[0]).as_deref(), Some("Tenno"));
    }

    #[test]
    fn object_ids_are_exact_lowercase_runs() {
        let line = "a 5f1e2d3c4b5a69788796a5b4 b mm=5F1E2D3C4B5A69788796A5B4 c 000000000000000000000000 \
                    d 5f1e2d3c4b5a69788796a5b4ff";
        assert_eq!(object_ids(line), ["5f1e2d3c4b5a69788796a5b4"]);
    }

    #[test]
    fn names_must_be_whole() {
        assert!(names_player("Race started for Tenno\u{e000} ID: x", "Tenno"));
        assert!(names_player("Race started for Tenno\u{e000}. ID: x", "Tenno\u{e000}"));
        assert!(names_player("Logged in Tenno", "Tenno"));
        assert!(!names_player("Race started for Tenno2 ID: x", "Tenno"));
        assert!(!names_player("xTenno joined", "Tenno"));
    }

    #[test]
    fn candidates() {
        let id = "5f1e2d3c4b5a69788796a5b4";
        let own = format!("VoidProjections: Sending reward info from {id} to host!");
        assert_eq!(candidate_ids(&own, None), [(id.to_string(), true)]);
        let other = format!("/Temp/HubPlayers/{id}");
        assert!(candidate_ids(&other, Some("Tenno")).is_empty());
        let two = format!("Tenno {id} {}", "1234567890abcdef12345678");
        assert!(candidate_ids(&two, Some("Tenno")).iter().all(|(_, sure)| !sure));
    }

    #[test]
    fn a_replayed_log_reports_each_login_with_what_followed_it() {
        let id = "5f1e2d3c4b5a69788796a5b4";
        let log = format!(
            "1.0 Sys [Info]: Logged in Alpha\n\
             2.0 Script [Info]: ThemedSquadOverlay.lua: OnMasteryIconReady1(Loaded: /Lotus/Interface/Icons/MasteryRanks/NewMasteryRanks/Rank0_d.png)\n\
             2.1 Script [Info]: ThemedSquadOverlay.lua: OnMasteryIconReady1(Loaded: /Lotus/Interface/Icons/MasteryRanks/NewMasteryRanks/Rank0_d.png)\n\
             3.0 Net [Info]: VoidProjections: Sending reward info from {id} to host!\n\
             9.0 Sys [Info]: Logged in Beta\n\
             9.5 Script [Info]: ThemedSquadOverlay.lua: OnMasteryIconReady1(Loaded: /Lotus/Interface/Icons/MasteryRanks/NewMasteryRanks/Rank34to36_d.png)\n"
        );
        let path = std::env::temp_dir().join(format!("wfp-eelog-test-{}.log", std::process::id()));
        fs::write(&path, log).unwrap();
        let events = scan_log(&path);
        let _ = fs::remove_file(&path);
        let shown: Vec<String> = events
            .iter()
            .map(|e| match e {
                LogEvent::Name(n, live) => format!("name {n} {live}"),
                LogEvent::Mastery(lo, hi) => format!("badge {lo} {hi:?}"),
                LogEvent::Candidate(i, sure, _) => format!("id {i} {sure}"),
                _ => "other".to_string(),
            })
            .collect();
        assert_eq!(
            shown,
            [
                "name Alpha false".to_string(),
                "badge 0 None".to_string(),
                format!("id {id} true"),
                "name Beta false".to_string(),
                "badge 34 Some(36)".to_string()
            ]
        );
    }

    #[test]
    fn mastery_badges() {
        // Lines from the user's EE.log (MR 0); the band form is what squadmates' slots show.
        let mine = "Script [Info]: ThemedSquadOverlay.lua: OnMasteryIconReady1(Loaded: /Lotus/Interface/Icons/MasteryRanks/NewMasteryRanks/Rank0_d.png)";
        assert_eq!(mastery_rank(mine), Some((0, None)));
        assert_eq!(mastery_rank(&mine.replace("Rank0", "Rank16to18")), Some((16, Some(18))));
        let squadmate = mine.replace("Ready1", "Ready2").replace("Rank0", "Rank13to15");
        assert_eq!(mastery_rank(&squadmate), None, "slots 2-4 are squadmates");
    }
}
