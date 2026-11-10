//! Warframe's own log (EE.log): finding it, what it says about the account, and when the reward
//! screen opens. Plain file reads - nothing here touches the game process.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{clock, now};

#[cfg(unix)]
const WF_APPID: &str = "230410";
#[cfg(unix)]
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
/// The squad overlay loads a mastery badge per slot. Slot 1 is the local player, slots 2-4 carry
/// the squadmates' badges. Rank 0 has an exact icon, every other badge covers three ranks
/// (Rank1to3 ... Rank49to51).
static MASTERY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"OnMasteryIconReady1\(Loaded:[^)]*?/Rank(\d+)(?:to(\d+))?_").unwrap());
/// A fissure reward screen in the log: `VoidProjections: OpenVoidProjectionRewardScreenRMI`, a 5 s
/// pause showing the relics (`Pause countdown done`, `Relic rewards initialized`,
/// `GetVoidProjectionRewards`), the cards turning over ~0.5 s later (`Got rewards`), and 15 s
/// after that `Relic reward screen shut down`. Some screens have no pause.
///
/// The game writes its log in delayed batches. Over six screens on 2026-11-08 the open line
/// reached the file 0.1-1.8 s after it was written, `Got rewards` up to 15 s late - as the screen
/// closed - and the pause lines once 10.7 s late. So the open line starts the watch, the others
/// only if it was missed, and the scan itself looks at the screen for the cards.
///
/// The 5 s relic display at the start of an endless mission's round shares the interface
/// (`Relic rewards initialized`, `Reward choice force closed`, `Relic reward screen shut down`)
/// but has no cards, and none of these lines.
const REWARD_OPENED: &str = "VoidProjections: OpenVoidProjectionRewardScreen";
const REWARD_STARTED: [&str; 3] = [
    "ProjectionRewardChoice.lua: Pause countdown done",
    "VoidProjections: GetVoidProjectionRewards",
    "ProjectionRewardChoice.lua: Got rewards",
];
const REWARD_CLOSED: &str = "ProjectionRewardChoice.lua: Relic reward screen shut down";
/// Duviri's loadout picks (Teshin's Cave), as the star chart and Duviri roll them: first
/// `MapRedux.lua: RollDuviriOffers - Generating new Duviri offers`, then one line per pick,
/// `MapRedux.lua: /Lotus/Powersuits/Paladin/Paladin generated as an unowned Cave selection for
/// category: 3`, then `RollDuviriOffers - Duviri Seed: <n>`. While the offers stay the same the
/// game logs "Using cached duviri offers" and lists no picks (EE.log, 2026-11-10).
const DUVIRI_NEW: &str = "RollDuviriOffers - Generating new Duviri offers";
const DUVIRI_DONE: &str = "RollDuviriOffers - Duviri Seed:";
/// Lines that suggest the account state changed on the server.
pub const SYNC_HINTS: [&str; 4] =
    ["EndOfMatch.lua: Mission Succeeded", "EndOfMatch.lua: Mission Failed", "ExitLevel", "Inventory updated"];
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

/// The game's interface scale (Options > Interface > Scale; 1.0 = 100%), from `EE.cfg` beside
/// the log. The reward screen's cards are laid out at this scale.
pub fn ui_scale(log: &Path) -> Option<f64> {
    let cfg = fs::read(log.with_file_name("EE.cfg")).ok()?;
    ui_scale_in(&String::from_utf8_lossy(&cfg))
}

/// `Flash.FlashDrawScale=0.85000002`, as the game writes it.
fn ui_scale_in(cfg: &str) -> Option<f64> {
    cfg.lines()
        .find_map(|line| line.trim().strip_prefix("Flash.FlashDrawScale=")?.trim().parse::<f64>().ok())
        .filter(|s| (0.3..=2.0).contains(s))
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

/// A Warframe or weapon Duviri offers, as the log names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DuviriPick {
    pub uid: String,
    /// The game's equipment slot: 3 Warframe, 1 primary, 0 secondary, 5 melee.
    pub category: u8,
    /// The game's own word on whether the account has it.
    pub owned: bool,
}

/// What Duviri offers the account. The game picks anew at the start of every spiral and after a
/// successful extraction (wiki.warframe.com/w/Duviri, "Loadout").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DuviriOffers {
    pub picks: Vec<DuviriPick>,
    /// When the game rolled them, when the log says.
    pub rolled: Option<f64>,
}

/// `... /Lotus/Powersuits/Berserker/Berserker generated as an owned Cave selection for category: 3`
fn duviri_pick(line: &str) -> Option<DuviriPick> {
    let (head, rest) = line.split_once(" generated as an ")?;
    let uid = head.rsplit(' ').next()?;
    let (owned, rest) = match rest.strip_prefix("owned ") {
        Some(rest) => (true, rest),
        None => (false, rest.strip_prefix("unowned ")?),
    };
    let category = rest.strip_prefix("Cave selection for category: ")?.trim().parse().ok()?;
    uid.starts_with("/Lotus/").then(|| DuviriPick { uid: uid.to_string(), category, owned })
}

/// Gathers the picks of one roll of Duviri offers, line by line.
#[derive(Default)]
struct DuviriRoll(Option<Vec<DuviriPick>>);

impl DuviriRoll {
    /// Takes one line; gives the picks back once the roll's last line has come.
    fn feed(&mut self, line: &str) -> Option<Vec<DuviriPick>> {
        if line.contains(DUVIRI_NEW) {
            self.0 = Some(Vec::new());
        } else if line.contains(DUVIRI_DONE) {
            return self.0.take().filter(|picks| !picks.is_empty());
        } else if let (Some(picks), Some(pick)) = (self.0.as_mut(), duviri_pick(line)) {
            picks.push(pick);
        }
        None
    }
}

/// When the game session began, from the log's header: `Sys [Diag]: Current time: Tue Nov 10
/// 00:07:35 2026 [UTC: Tue Nov 10 00:07:35 2026]`. Every line starts with the seconds since.
fn log_start(line: &str) -> Option<f64> {
    let utc = line.split_once("Current time:")?.1.split_once("[UTC: ")?.1.split(']').next()?;
    clock::parse_ctime(utc)
}

/// The seconds a line begins with: "42.908 Script [Info]: ..." is 42.908.
fn line_seconds(line: &str) -> Option<f64> {
    line.split_whitespace().next()?.parse().ok()
}

/// What an existing log already says, in order, as the live watcher would have reported it:
/// logins, explicit ids, id candidates, the local player's mastery badges and the Duviri offers
/// rolled (timed by the header's clock). Reward screens and
/// syncs are left out (they are over). The order matters: one game session can log out and in
/// with another account, and everything after a login belongs to that account.
fn scan_log(path: &Path, end: u64) -> (Vec<LogEvent>, Vec<u8>) {
    const MAX_BYTES: u64 = 64_000_000;
    let mut events = Vec::new();
    let Ok(mut f) = fs::File::open(path) else { return (events, Vec::new()) };
    let start = end.saturating_sub(MAX_BYTES);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return (events, Vec::new());
    }
    let mut raw = Vec::new();
    if f.take(end - start).read_to_end(&mut raw).is_err() {
        return (events, Vec::new());
    }
    let pending = match raw.iter().rposition(|b| *b == b'\n') {
        Some(last) => raw.split_off(last + 1),
        None => std::mem::take(&mut raw),
    };
    let text = String::from_utf8_lossy(&raw);
    let mut player: Option<String> = None;
    let mut badge = None;
    let mut seen = std::collections::HashSet::new();
    let mut started: Option<f64> = None;
    let mut duviri = DuviriRoll::default();
    for line in text.lines() {
        if let Some(t) = log_start(line) {
            started = Some(t);
        }
        if let Some(picks) = duviri.feed(line) {
            let rolled = started.zip(line_seconds(line)).map(|(start, secs)| start + secs);
            events.push(LogEvent::Duviri(DuviriOffers { picks, rolled }));
        }
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
    (events, pending)
}

fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

// ------------------------------------------------------------------ finding the log

#[cfg(unix)]
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

#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(unix)]
static LIBRARY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""path"\s+"([^"]+)""#).unwrap());

/// Windows: the game writes its log under the user's local app data.
#[cfg(windows)]
pub fn candidate_paths() -> Vec<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    local.map(|dir| dir.join("Warframe").join("EE.log")).into_iter().collect()
}

/// Linux: the log sits inside the Wine prefix Proton, Lutris or Heroic made for the game.
#[cfg(unix)]
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
        .filter_map(|p| fs::metadata(&p).ok().filter(|m| m.is_file()).and_then(|m| Some((m.modified().ok()?, p))))
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

/// The newest EE.log, searched for right now: for a one-off command that cannot wait for the
/// background search.
pub fn find_ee_log_now() -> Option<PathBuf> {
    search();
    FOUND.lock().unwrap_or_else(|e| e.into_inner()).path.clone()
}

pub fn searching() -> bool {
    FOUND.lock().unwrap_or_else(|e| e.into_inner()).searching
}

// ------------------------------------------------------------------ following the log

/// Something that changes when the game deletes its log and writes a new one: the inode on
/// Linux, the creation time on Windows (which has no inode numbers in std).
#[cfg(unix)]
fn file_id(meta: &fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(meta)
}

#[cfg(windows)]
fn file_id(meta: &fs::Metadata) -> u64 {
    std::os::windows::fs::MetadataExt::creation_time(meta)
}

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
    /// What Duviri offers the player, rolled anew.
    Duviri(DuviriOffers),
    RewardScreen,
    RewardScreenClosed,
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
/// The game flushes EE.log in arbitrary chunks, not whole lines; most reads end mid-line,
/// sometimes inside the 3-byte platform glyph. Handling each piece as a line would split
/// triggers in two and read "Logged in tennoplayer" as the name "tenno", so the bytes after the
/// last newline wait for the rest of their line.
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
    reward_active: bool,
    duviri: DuviriRoll,
}

impl LogWatcher {
    pub fn spawn(path_of: PathGetter, emit: Handler) -> Arc<Mutex<LogStatus>> {
        let status = Arc::new(Mutex::new(LogStatus::default()));
        let mut watcher = LogWatcher {
            path_of,
            emit,
            status: Arc::clone(&status),
            player: None,
            reward_active: false,
            duviri: DuviriRoll::default(),
        };
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
                self.reward_active = false;
                self.duviri = DuviriRoll::default();
                self.status().path = Some(path.clone());
                let meta = fs::metadata(&path).ok();
                pos = meta.as_ref().map_or(0, |m| m.len());
                inode = meta.as_ref().map_or(0, file_id);
                lines.clear();
                let pending = self.replay(&path, pos);
                lines.push(&pending);
                current = Some(path.clone());
            }
            if let Ok(meta) = fs::metadata(&path) {
                if file_id(&meta) != inode || meta.len() < pos {
                    // The game restarted and recreated its log.
                    (pos, inode) = (0, file_id(&meta));
                    lines.clear();
                    self.reward_active = false;
                    self.duviri = DuviriRoll::default();
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
            // The reward screen is read the moment its cards are up and its prices hidden the
            // moment it closes, so the log is looked at often; a look is one stat call.
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Reads what an existing log already says (logins, ids, mastery) when it is first found.
    fn replay(&mut self, path: &Path, end: u64) -> Vec<u8> {
        let (events, pending) = scan_log(path, end);
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
        pending
    }

    fn handle(&mut self, line: &str) {
        if let Some(name) = player_name(line) {
            if self.player.as_deref() != Some(name.as_str()) || line.contains("Logged in ") {
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
        if let Some(picks) = self.duviri.feed(line) {
            (self.emit)(LogEvent::Duviri(DuviriOffers { picks, rolled: Some(now()) }));
        }
        if line.contains(REWARD_OPENED) {
            // A new screen, even should the last one's shut-down line have been missed.
            self.status().note(line);
            self.reward_active = true;
            (self.emit)(LogEvent::RewardScreen);
        } else if REWARD_STARTED.iter().any(|l| line.contains(l)) {
            self.status().note(line);
            if !self.reward_active {
                self.reward_active = true;
                (self.emit)(LogEvent::RewardScreen);
            }
        } else if line.contains(REWARD_CLOSED) {
            self.status().note(line);
            if self.reward_active {
                self.reward_active = false;
                (self.emit)(LogEvent::RewardScreenClosed);
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
    fn reward_watch_starts_at_the_open_line_once_per_screen() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut watcher = LogWatcher {
            path_of: Box::new(|| None),
            emit: Box::new(move |event| match event {
                LogEvent::RewardScreen => captured.lock().unwrap().push("watch"),
                LogEvent::RewardScreenClosed => captured.lock().unwrap().push("closed"),
                _ => {}
            }),
            status: Arc::new(Mutex::new(LogStatus::default())),
            player: None,
            reward_active: false,
            duviri: DuviriRoll::default(),
        };
        for line in [
            // A reward screen, as on 2026-11-08: one watch from the open line on.
            "3843.388 Sys [Info]: VoidProjections: OpenVoidProjectionRewardScreenRMI",
            "3848.396 Script [Info]: ProjectionRewardChoice.lua: Pause countdown done",
            "3848.419 Script [Info]: ProjectionRewardChoice.lua: Relic rewards initialized",
            "3848.419 Sys [Info]: VoidProjections: GetVoidProjectionRewards",
            "3848.883 Script [Info]: ProjectionRewardChoice.lua: Got rewards",
            "3863.885 Script [Info]: ProjectionRewardChoice.lua: Relic reward screen shut down",
            // The next round's relic display: not a reward screen.
            "3869.161 Script [Info]: ProjectionRewardChoice.lua: Relic rewards initialized",
            "3874.153 Script [Info]: ProjectionRewardChoice.lua: Reward choice force closed",
            "3874.153 Script [Info]: ProjectionRewardChoice.lua: Relic reward screen shut down",
            // A screen whose open line was missed: its first card line starts the watch.
            "4010.290 Script [Info]: ProjectionRewardChoice.lua: Relic rewards initialized",
            "4010.290 Sys [Info]: VoidProjections: GetVoidProjectionRewards",
            "4010.793 Script [Info]: ProjectionRewardChoice.lua: Got rewards",
            // A screen whose shut-down line never came: the next open line still starts a watch.
            "4255.999 Sys [Info]: VoidProjections: OpenVoidProjectionRewardScreenRMI",
            "4261.477 Script [Info]: ProjectionRewardChoice.lua: Got rewards",
            "4276.476 Script [Info]: ProjectionRewardChoice.lua: Relic reward screen shut down",
        ] {
            watcher.handle(line);
        }
        assert_eq!(*events.lock().unwrap(), ["watch", "closed", "watch", "watch", "closed"]);
    }

    #[test]
    fn interface_scale_comes_from_the_config() {
        let cfg = "Graphics.WindowMode=MODE_BORDERLESS\r\nFlash.FlashDrawScale=0.85000002\r\nFlash.FlashDrawScaleMode=DSM_CUSTOM\r\n";
        assert_eq!(ui_scale_in(cfg), Some(0.85000002));
        assert_eq!(ui_scale_in("Flash.FlashDrawScaleMode=DSM_CUSTOM\n"), None);
        assert_eq!(ui_scale_in("Flash.FlashDrawScale=0\n"), None);
    }

    #[test]
    fn logging_back_into_the_same_account_emits_another_login() {
        let names = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&names);
        let mut watcher = LogWatcher {
            path_of: Box::new(|| None),
            emit: Box::new(move |event| {
                if let LogEvent::Name(name, live) = event {
                    received.lock().unwrap().push((name, live));
                }
            }),
            status: Arc::new(Mutex::new(LogStatus::default())),
            player: None,
            reward_active: false,
            duviri: DuviriRoll::default(),
        };
        watcher.handle("Sys [Info]: Logged in Tenno");
        watcher.handle("Sys [Info]: Player name changed to Tenno");
        watcher.handle("Sys [Info]: Logged in Tenno");
        assert_eq!(*names.lock().unwrap(), [("Tenno".into(), true), ("Tenno".into(), true)]);
    }

    #[test]
    fn a_line_cut_in_two_is_read_once_whole() {
        let mut buf = LineBuffer::default();
        assert!(buf.push(b"13.945 Sys [Info]: Logged in tenno").is_empty());
        let lines = buf.push(b"player\r\n1.0 next\r\n");
        assert_eq!(lines, ["13.945 Sys [Info]: Logged in tennoplayer", "1.0 next"]);
        assert_eq!(player_name(&lines[0]).as_deref(), Some("tennoplayer"));
    }

    #[test]
    fn replay_snapshot_keeps_a_partial_login_for_the_live_watcher() {
        let path = std::env::temp_dir().join(format!("wfp-eelog-snapshot-{}.log", std::process::id()));
        let partial = b"1.0 Sys [Info]: Logged in Ten";
        fs::write(&path, [partial.as_slice(), b"no\n"].concat()).unwrap();
        let (events, pending) = scan_log(&path, partial.len() as u64);
        fs::remove_file(&path).unwrap();
        assert!(events.is_empty());
        let mut lines = LineBuffer::default();
        assert!(lines.push(&pending).is_empty());
        let followed = lines.push(b"no\n");
        assert_eq!(player_name(&followed[0]).as_deref(), Some("Tenno"));
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
        fs::write(&path, &log).unwrap();
        let (events, pending) = scan_log(&path, log.len() as u64);
        assert!(pending.is_empty());
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

    /// A roll of Duviri offers as EE.log wrote one on 2026-11-10 (the seed made up), cut to one
    /// pick per slot.
    const DUVIRI_LOG: &str = "0.072 Sys [Diag]: Current time: Tue Nov 10 00:07:35 2026 [UTC: Tue Nov 10 00:07:35 2026]\n\
        14.454 Sys [Info]: Logged in Tenno\n\
        42.903 Script [Info]: MapRedux.lua: RollDuviriOffers - Generating new Duviri offers\n\
        42.908 Script [Info]: MapRedux.lua: /Lotus/Powersuits/Paladin/Paladin generated as an unowned Cave selection for category: 3\n\
        42.908 Script [Info]: MapRedux.lua: /Lotus/Weapons/Grineer/LongGuns/BurstRifle/GrnBurstRifle generated as an owned Cave selection for category: 1\n\
        42.908 Script [Info]: MapRedux.lua: /Lotus/Weapons/Infested/Pistols/InfProximityStars/InfProximityStars generated as an unowned Cave selection for category: 0\n\
        42.909 Script [Info]: MapRedux.lua: /Lotus/Weapons/Tenno/Melee/Hammer/HammerWeapon generated as an owned Cave selection for category: 5\n\
        42.909 Script [Info]: MapRedux.lua: Found 0 overrides.\n\
        42.909 Script [Info]: MapRedux.lua: RollDuviriOffers - Duviri Seed: 1234567890\n\
        42.910 Script [Info]: MapRedux.lua: Generated weapons:\n\
        42.910 Script [Info]: MapRedux.lua: 1> 1> /Lotus/Weapons/Grineer/LongGuns/BurstRifle/GrnBurstRifle\n\
        112.745 Script [Info]: MapRedux.lua: RollDuviriOffers - Using cached duviri offers\n\
        112.745 Script [Info]: MapRedux.lua: RollDuviriOffers - Duviri Seed: 1234567890\n";

    #[test]
    fn duviri_offers_come_from_the_log() {
        let picks = |v: &[DuviriPick]| {
            v.iter().map(|p| (p.uid.rsplit('/').next().unwrap().to_string(), p.category, p.owned)).collect::<Vec<_>>()
        };
        let expected = [
            ("Paladin".to_string(), 3, false),
            ("GrnBurstRifle".to_string(), 1, true),
            ("InfProximityStars".to_string(), 0, false),
            ("HammerWeapon".to_string(), 5, true),
        ];
        // Read back: one roll, timed by the header's clock (00:07:35 UTC + 42.909 s).
        let path = std::env::temp_dir().join(format!("wfp-eelog-duviri-{}.log", std::process::id()));
        fs::write(&path, DUVIRI_LOG).unwrap();
        let (events, _) = scan_log(&path, DUVIRI_LOG.len() as u64);
        let _ = fs::remove_file(&path);
        let rolls: Vec<&DuviriOffers> =
            events.iter().filter_map(|e| if let LogEvent::Duviri(d) = e { Some(d) } else { None }).collect();
        assert_eq!(rolls.len(), 1, "the cached offers are no new roll");
        assert_eq!(picks(&rolls[0].picks), expected);
        let rolled = rolls[0].rolled.unwrap();
        assert!((rolled - (clock::parse_iso("2026-11-10T00:07:35Z").unwrap() + 42.909)).abs() < 0.001);
        assert!(matches!(events[0], LogEvent::Name(..)), "the login comes first, so the roll is that account's");
        // Followed live: the same roll, timed now.
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&got);
        let mut watcher = LogWatcher {
            path_of: Box::new(|| None),
            emit: Box::new(move |event| {
                if let LogEvent::Duviri(d) = event {
                    sink.lock().unwrap().push(d);
                }
            }),
            status: Arc::new(Mutex::new(LogStatus::default())),
            player: None,
            reward_active: false,
            duviri: DuviriRoll::default(),
        };
        for line in DUVIRI_LOG.lines() {
            watcher.handle(line);
        }
        let live = got.lock().unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(picks(&live[0].picks), expected);
        assert!(live[0].rolled.is_some_and(|t| (t - now()).abs() < 60.0));
        // Pick lines outside a roll, or of another shape, are not picks.
        assert_eq!(DuviriRoll::default().feed(DUVIRI_LOG.lines().nth(3).unwrap()), None);
        assert_eq!(duviri_pick("x /Lotus/A generated as an owned Cave selection for category: x"), None);
        assert_eq!(duviri_pick("x Temp/A generated as an owned Cave selection for category: 3"), None);
    }

    #[test]
    fn mastery_badges() {
        // Lines from an EE.log (MR 0); the band form is what squadmates' slots show.
        let mine = "Script [Info]: ThemedSquadOverlay.lua: OnMasteryIconReady1(Loaded: /Lotus/Interface/Icons/MasteryRanks/NewMasteryRanks/Rank0_d.png)";
        assert_eq!(mastery_rank(mine), Some((0, None)));
        assert_eq!(mastery_rank(&mine.replace("Rank0", "Rank16to18")), Some((16, Some(18))));
        let squadmate = mine.replace("Ready1", "Ready2").replace("Rank0", "Rank13to15");
        assert_eq!(mastery_rank(&squadmate), None, "slots 2-4 are squadmates");
    }
}
