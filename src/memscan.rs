//! Off by default: a read-only scan of the running game for its inventory JSON.
//!
//! This mirrors what Overwolf's Warframe plugin (used by AlecaFrame) does: the game already holds
//! the inventory it downloaded from DE as JSON text; the scan finds that text (anchored on
//! "LastInventorySync") and copies it out. It never writes to the game and never injects. When
//! enabled it runs by itself after login and after each mission (`State::schedule_memscan`).
//! Whether the system allows the read is decided when the game's memory is opened.
//!
//! Digital Extremes' position on third-party tools is "use at your own risk".

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Instant;

use indexmap::IndexMap;
use memchr::memmem;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const MARKER: &[u8] = b"\"LastInventorySync\"";
/// The same process memory holds the login payload, with the live session Token and
/// TwitchAuthToken. Nothing outside this allowlist is ever kept - a leaked session token is worth
/// more to an attacker than the inventory is to anyone. `allowed` adds three families of keys
/// whose names vary: the daily standing pools (`DailyAffiliationCetus`...) and intrinsic ranks
/// and points (`LPS_PILOTING`, `LPP_SPACE`).
const KEEP_KEYS: [&str; 47] = [
    "LastInventorySync",
    "MiscItems",
    "Suits",
    "LongGuns",
    "Pistols",
    "Melee",
    "SpaceSuits",
    "SpaceGuns",
    "SpaceMelee",
    "Sentinels",
    "SentinelWeapons",
    "KubrowPets",
    "MoaPets",
    "Hoverboards",
    "MechSuits",
    "OperatorAmps",
    "SpecialItems",
    "CrewShipWeapons",
    "CrewShipHarnesses",
    "KubrowPetEggs",
    "Recipes",
    "PendingRecipes",
    "Upgrades",
    "RawUpgrades",
    "Consumables",
    "FlavourItems",
    "LevelKeys",
    "ShipDecorations",
    "XPInfo",
    "PlayerLevel",
    "PremiumCredits",
    "RegularCredits",
    "FusionPoints",
    "PrimeTokens",
    "ItemType",
    "XP",
    "ItemCount",
    "CompletionDate",
    // Syndicate standing: [{"Tag", "Standing", "Title"}], and the focus left today.
    "Affiliations",
    "Tag",
    "Standing",
    "Title",
    "DailyFocus",
    // Star chart and intrinsics, for the mastery total: [{"Tag", "Completes", "Tier"}] and
    // {"LPS_...": rank}.
    "Missions",
    "Completes",
    "Tier",
    "PlayerSkills",
];
/// Mongo extended-JSON wrappers. Structural only, and only ever inside a key already allowed.
const BSON_KEYS: [&str; 5] = ["$date", "$numberLong", "$numberInt", "$numberDouble", "$numberDecimal"];
static SECRET_HINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)token|secret|password|auth|session|nonce|cookie").unwrap());

const CHUNK: usize = 16 * 1024 * 1024;
const MAX_EXTENT: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum ScanError {
    NotRunning,
    /// The system refused to let this process read the game's memory.
    Denied(String),
    /// The inventory text is not (completely) in memory right now; it is after the next sync.
    NotFound {
        matches: usize,
    },
    Io(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::NotRunning => f.write_str("Warframe is not running."),
            ScanError::Denied(why) => f.write_str(why),
            ScanError::NotFound { matches: 0 } => f.write_str(
                "The inventory is not in the game's memory right now; it is read again after the next mission.",
            ),
            ScanError::NotFound { matches } => write!(
                f,
                "Only fragments of the inventory are in the game's memory ({matches}); it is read again after the next mission."
            ),
            ScanError::Io(e) => write!(f, "Could not read the game's memory: {e}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Whether a key may be kept: a known inventory field, one of the families named above, or a
/// Mongo wrapper - and never anything that sounds like a credential.
fn allowed(key: &str) -> bool {
    let known = KEEP_KEYS.contains(&key)
        || BSON_KEYS.contains(&key)
        || key.starts_with("DailyAffiliation")
        || key.starts_with("LPS_")
        || key.starts_with("LPP_");
    known && !SECRET_HINT.is_match(key)
}

/// Keeps only known inventory fields. Anything unrecognised is dropped, not inspected.
pub fn sanitise(v: Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.into_iter().map(sanitise).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter().filter(|(k, _)| allowed(k)).map(|(k, v)| (k, sanitise(v))).collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}

// ------------------------------------------------------------------ the operating system's part

/// Linux: the game's memory through /proc. Windows has its own version in `windows/memory.rs`
/// with the same functions.
#[cfg(unix)]
mod os {
    use std::fs;
    use std::os::unix::fs::FileExt;

    use memchr::memmem;

    use super::ScanError;

    /// kernel.yama.ptrace_scope, when Yama is active.
    fn ptrace_scope() -> Option<u8> {
        fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope").ok().and_then(|s| s.trim().parse().ok())
    }

    /// Rules out, without touching the game, only what can never work: ptrace_scope 3 forbids every
    /// program to read another's memory.
    ///
    /// Scopes 1 and 2 are not a "no". Yama lets a process read another one's memory when it holds
    /// CAP_SYS_PTRACE in that process's user namespace (security/yama/yama_lsm.c), and a namespace's
    /// owner holds every capability in it. Steam's sandbox runs the game in a user namespace owned by
    /// the user, so on such a system (NixOS, for one) the read works at scope 1. Whether it applies
    /// elsewhere only shows when the game's memory is opened.
    pub fn availability() -> Result<(), String> {
        match ptrace_scope() {
            Some(3) => {
                Err("kernel.yama.ptrace_scope is 3: Linux lets no program read another program's memory.".into())
            }
            _ => Ok(()),
        }
    }

    pub fn find_game_pid() -> Option<u32> {
        for entry in fs::read_dir("/proc").ok()?.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            if comm.trim().starts_with("Warframe.x64") {
                return Some(pid);
            }
            let cmd = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let has = |needle: &[u8]| memmem::find(&cmd, needle).is_some();
            if has(b"Warframe.x64.exe") && !has(b"Launcher") && !has(b"reaper") {
                return Some(pid);
            }
        }
        None
    }

    /// Writable, anonymous-ish regions: where downloaded JSON lives.
    fn regions(pid: u32) -> Vec<(u64, u64)> {
        let maps = fs::read_to_string(format!("/proc/{pid}/maps")).unwrap_or_default();
        maps.lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let (range, perms) = (parts.next()?, parts.next()?);
                let path = parts.nth(3).unwrap_or("");
                if !perms.starts_with("rw")
                    || path.ends_with(".so")
                    || path.ends_with(".dll")
                    || path.ends_with(".exe")
                    || path.starts_with("/dev/")
                {
                    return None;
                }
                let (a, b) = range.split_once('-')?;
                let (a, b) = (u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?);
                (b > a).then_some((a, b))
            })
            .collect()
    }

    /// The game's memory, opened for reading.
    pub struct Memory {
        file: fs::File,
        pid: u32,
    }

    impl Memory {
        /// Opening is where the kernel decides; a refusal costs nothing but a line in its log.
        pub fn open(pid: u32) -> Result<Memory, ScanError> {
            let file = fs::File::open(format!("/proc/{pid}/mem")).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    let scope = ptrace_scope().map_or_else(|| "not set".to_string(), |s| s.to_string());
                    ScanError::Denied(format!(
                        "Linux refused access to the game's memory (kernel.yama.ptrace_scope is {scope}). It is tried \
                         again when the game restarts; the app never changes that setting."
                    ))
                }
                std::io::ErrorKind::NotFound => ScanError::NotRunning,
                _ => ScanError::Io(e.to_string()),
            })?;
            Ok(Memory { file, pid })
        }

        pub fn regions(&self) -> Vec<(u64, u64)> {
            regions(self.pid)
        }

        /// Up to `len` bytes at `addr`; fewer (or none) where the memory is not readable.
        pub fn read(&self, addr: u64, len: usize) -> Vec<u8> {
            let mut buf = vec![0u8; len];
            match self.file.read_at(&mut buf, addr) {
                Ok(n) => {
                    buf.truncate(n);
                    buf
                }
                Err(_) => Vec::new(),
            }
        }
    }
}

#[cfg(windows)]
#[path = "windows/memory.rs"]
mod os;

pub use os::{availability, find_game_pid};

/// Epoch seconds from a Mongo ObjectId: its first four bytes are a unix timestamp.
fn oid_time(v: &Value) -> i64 {
    let oid = v.get("$oid").unwrap_or(v);
    oid.as_str().and_then(|s| s.get(..8)).and_then(|s| i64::from_str_radix(s, 16).ok()).unwrap_or(0)
}

fn sync_time(v: &Value) -> i64 {
    v["LastInventorySync"].as_i64().unwrap_or_else(|| oid_time(&v["LastInventorySync"]))
}

/// Grows outward from the marker until NUL bytes bound the text, then decodes the JSON object.
fn extract_blob(mem: &os::Memory, (start_lim, end_lim): (u64, u64), hit: u64) -> Option<Value> {
    extract_blob_from(|addr, len| mem.read(addr, len), (start_lim, end_lim), hit)
}

fn extract_blob_from(
    read: impl Fn(u64, usize) -> Vec<u8>,
    (start_lim, end_lim): (u64, u64),
    hit: u64,
) -> Option<Value> {
    const STEP: u64 = 1024 * 1024;
    let mut lo = hit;
    while lo > start_lim && hit - lo < MAX_EXTENT {
        let a = start_lim.max(lo.saturating_sub(STEP));
        let buf = read(a, (lo - a) as usize);
        if buf.len() != (lo - a) as usize {
            break;
        }
        if let Some(z) = memchr::memrchr(0, &buf) {
            lo = a + z as u64 + 1;
            break;
        }
        lo = a;
    }
    let mut hi = hit;
    while hi < end_lim && hi - hit < MAX_EXTENT {
        let buf = read(hi, STEP.min(end_lim - hi) as usize);
        if buf.is_empty() {
            break;
        }
        if let Some(z) = memchr::memchr(0, &buf) {
            hi += z as u64;
            break;
        }
        hi += buf.len() as u64;
    }
    let raw = read(lo, (hi - lo) as usize);
    if raw.len() != (hi - lo) as usize {
        return None;
    }
    decode_blob(&raw, (hit - lo) as usize)
}

/// Recovers a JSON suffix whose beginning was overwritten.
fn decode_blob(raw: &[u8], hit: usize) -> Option<Value> {
    if raw.get(hit..hit.checked_add(MARKER.len())?)? != MARKER {
        return None;
    }

    let decode = |start: usize, prepend: bool| -> Option<Value> {
        let prefix = if prepend { 1 } else { 0 };
        let mut candidate = Vec::with_capacity(raw.len() - start + prefix);
        if prepend {
            candidate.push(b'{');
        }
        candidate.extend_from_slice(&raw[start..]);
        let mut stream = serde_json::Deserializer::from_slice(&candidate).into_iter::<Value>();
        let mut obj = stream.next()?.ok()?;
        // The parsed object must contain this hit, not an unrelated earlier JSON object.
        if stream.byte_offset() < hit - start + prefix + MARKER.len()
            || obj.get("LastInventorySync").is_none()
            || (obj.get("MiscItems").is_none() && obj.get("Suits").is_none())
        {
            return None;
        }
        let t = sync_time(&obj);
        obj["LastInventorySync"] = Value::from(t);
        Some(sanitise(obj))
    };

    let start = raw.iter().position(|b| !b.is_ascii_whitespace())?;
    if start <= hit {
        if raw[start] == b'{' {
            if let Some(obj) = decode(start, false) {
                return Some(obj);
            }
            let mut stream = serde_json::Deserializer::from_slice(&raw[start..]).into_iter::<Value>();
            if matches!(stream.next(), Some(Ok(_))) && stream.byte_offset() >= hit - start + MARKER.len() {
                return None;
            }
        }
        let start = if raw[start] == b',' { start + 1 } else { start };
        if start <= hit {
            if let Some(obj) = decode(start, true) {
                return Some(obj);
            }
        }
    }

    // The buffer's head may have been overwritten in the middle of a field. Try surviving
    // top-level inventory keys, earliest first, retaining as much of the capture as possible.
    // Nested item keys are excluded, and parsing still requires the root's closing brace.
    let top_level_end = KEEP_KEYS.iter().position(|key| *key == "ItemType")?;
    let mut attempts = 0;
    for pos in memchr::memchr_iter(b'"', &raw[..=hit]) {
        let rest = &raw[pos + 1..];
        let Some(end) = memchr::memchr(b'"', rest) else { continue };
        let Ok(key) = std::str::from_utf8(&rest[..end]) else { continue };
        if !KEEP_KEYS[..top_level_end].contains(&key)
            && !matches!(key, "Affiliations" | "DailyFocus" | "Missions" | "PlayerSkills")
            && !key.starts_with("DailyAffiliation")
        {
            continue;
        }
        if rest[end + 1..].iter().copied().find(|b| !b.is_ascii_whitespace()) != Some(b':') {
            continue;
        }
        if let Some(obj) = decode(pos, true) {
            return Some(obj);
        }
        attempts += 1;
        if attempts >= 64 {
            break;
        }
    }
    None
}

/// Which top-level sections this account's captures normally hold.
///
/// The game reuses the head of the inventory text, so a capture can start anywhere inside it and
/// simply lack the sections before that point. Applying such a capture makes everything in those
/// sections drop to zero until the next good one. A section seen in nearly every capture so far is
/// expected; a capture without one is refused.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sections {
    /// Captures applied so far.
    pub captures: u32,
    /// Section -> how many of those captures had it.
    pub seen: IndexMap<String, u32>,
}

impl Sections {
    /// A few captures are needed before anything is expected.
    const MIN_CAPTURES: u32 = 3;

    /// The expected sections `capture` lacks. Empty means it can be applied.
    pub fn missing(&self, capture: &Value) -> Vec<String> {
        if self.captures < Self::MIN_CAPTURES {
            return Vec::new();
        }
        self.seen
            .iter()
            // Expected: present in at least nine captures out of ten.
            .filter(|(_, n)| **n * 10 >= self.captures * 9)
            .filter(|(key, _)| capture.get(key.as_str()).is_none())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Records an applied capture.
    pub fn learn(&mut self, capture: &Value) {
        self.captures += 1;
        for key in capture.as_object().into_iter().flat_map(|o| o.keys()) {
            *self.seen.entry(key.clone()).or_insert(0) += 1;
        }
    }
}

pub struct ScanInfo {
    /// How long the read took, to a tenth of a second.
    pub seconds: f64,
}

/// How many different captures a scan hands back at most.
const MAX_CAPTURES: usize = 6;

/// JSON buffers can span adjacent readable mappings, even when the OS lists them separately.
fn join_regions(regions: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    let mut joined: Vec<(u64, u64)> = Vec::new();
    for (start, end) in regions {
        if let Some(last) = joined.last_mut().filter(|last| last.1 == start) {
            last.1 = end;
        } else {
            joined.push((start, end));
        }
    }
    joined
}

/// Reads the inventory out of the game process `pid`: every distinct copy found, newest sync
/// first. There can be more than one account's: after logging out and in with another account the
/// game may still hold the first one's text, and its sync can even be the newer one. The caller
/// decides which copy is the logged-in account's (`State::apply_capture`).
pub fn scan_inventory(pid: u32, progress: impl Fn(f64)) -> Result<(Vec<Value>, ScanInfo), ScanError> {
    availability().map_err(ScanError::Denied)?;
    let mem = os::Memory::open(pid)?;
    let started = Instant::now();
    let regions = join_regions(mem.regions());
    let total: u64 = regions.iter().map(|(a, b)| b - a).sum::<u64>().max(1);
    let (mut found, mut hits, mut done) = (Vec::<Value>::new(), 0usize, 0u64);
    let mut seen = HashSet::new();
    for &(a, b) in &regions {
        scan_region(
            (a, b),
            |addr, len| mem.read(addr, len),
            |addr| {
                if !seen.insert(addr) {
                    return;
                }
                hits += 1;
                if let Some(obj) = extract_blob(&mem, (a, b), addr) {
                    keep_capture(&mut found, obj);
                }
            },
            |advanced| {
                done += advanced;
                progress(done as f64 / total as f64);
            },
        );
    }
    let info = ScanInfo { seconds: (started.elapsed().as_secs_f64() * 10.0).round() / 10.0 };
    if found.is_empty() {
        return Err(ScanError::NotFound { matches: hits });
    }
    Ok((found, info))
}

/// Reads within region boundaries, retaining overlap even after a short read. If a large read
/// fails because memory changed since enumeration, retry smaller pieces before skipping a page.
fn scan_region(
    (start, end): (u64, u64),
    read: impl Fn(u64, usize) -> Vec<u8>,
    mut hit: impl FnMut(u64),
    mut progress: impl FnMut(u64),
) {
    let finder = memmem::Finder::new(MARKER);
    let mut off = start;
    while off < end {
        let remaining = end - off;
        let n = remaining.min(CHUNK as u64) as usize;
        let request = remaining.min((n + MARKER.len() - 1) as u64) as usize;
        let mut buf = read(off, request);
        if buf.is_empty() && request > 64 * 1024 {
            buf = read(off, 64 * 1024);
        }
        if buf.is_empty() && request > 4096 {
            buf = read(off, 4096);
        }
        let advance = if buf.is_empty() {
            remaining.min(4096 - (off - start) % 4096) as usize
        } else if buf.len() < request {
            buf.len().saturating_sub(MARKER.len() - 1).max(1)
        } else {
            n
        };
        for at in finder.find_iter(&buf).filter(|&at| at < advance) {
            hit(off + at as u64);
        }
        off += advance as u64;
        progress(advance as u64);
    }
}

/// Adds a capture to `found`, newest sync first. Copies of one sync are one capture: the most
/// complete copy stands for it.
fn keep_capture(found: &mut Vec<Value>, obj: Value) {
    let size = |v: &Value| v.as_object().map_or(0, |o| o.len());
    if let Some(same) = found.iter_mut().find(|c| sync_time(c) == sync_time(&obj)) {
        if size(&obj) > size(same) {
            *same = obj;
        }
        return;
    }
    found.push(obj);
    found.sort_by_key(|c| std::cmp::Reverse(sync_time(c)));
    found.truncate(MAX_CAPTURES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode(text: &str) -> Option<Value> {
        decode_blob(text.as_bytes(), memmem::find(text.as_bytes(), MARKER).unwrap())
    }

    #[test]
    fn adjacent_readable_regions_are_joined_without_crossing_gaps() {
        assert_eq!(join_regions(vec![(0, 100), (100, 200), (201, 300), (300, 400)]), vec![(0, 200), (201, 400)]);
    }

    #[test]
    fn extracts_inventory_spanning_adjacent_regions() {
        let raw = b"\0{\"Suits\":[{\"ItemType\":\"/A\"}],\"MiscItems\":[],\"LastInventorySync\":123}\0";
        let hit = memmem::find(raw, MARKER).unwrap();
        let regions = join_regions(vec![(0, 20), (20, 50), (50, raw.len() as u64)]);
        let read = |addr: u64, len: usize| raw[addr as usize..addr as usize + len].to_vec();
        let mut captures = Vec::new();
        scan_region(regions[0], read, |addr| captures.push(extract_blob_from(read, regions[0], addr)), |_| {});
        assert_eq!(captures, [Some(json!({"Suits": [{"ItemType": "/A"}], "MiscItems": [], "LastInventorySync": 123}))]);
        assert!(extract_blob_from(read, ((hit - 3) as u64, raw.len() as u64), hit as u64).is_none());
    }

    #[test]
    fn decodes_complete_inventory_and_preserves_numeric_sync() {
        let obj = decode(
            r#"{"Token":"secret","MiscItems":[{"ItemType":"/A","ItemCount":2}],"LastInventorySync":123,"Suits":[]}"#,
        )
        .unwrap();
        assert_eq!(
            obj,
            json!({"MiscItems": [{"ItemType": "/A", "ItemCount": 2}], "LastInventorySync": 123, "Suits": []})
        );
    }

    #[test]
    fn recovers_inventory_after_overwritten_prefix() {
        let obj = decode(
            r#"garbage damaged":[1,2],"MiscItems":[{"ItemType":"/A","ItemCount":2}],"Suits":[],"LastInventorySync":{"$oid":"5f1e2d3c4b5a69788796a5b4"},"Token":"secret"}"#,
        )
        .unwrap();
        assert_eq!(
            obj,
            json!({"MiscItems": [{"ItemType": "/A", "ItemCount": 2}], "Suits": [], "LastInventorySync": 0x5f1e2d3c})
        );
    }

    #[test]
    fn refuses_truncated_or_nested_inventory() {
        assert!(decode(r#"broken,"MiscItems":[],"LastInventorySync":123"#).is_none());
        assert!(
            decode(r#"{"MiscItems":[],"Nested":{"LastInventorySync":123}}"#).is_none(),
            "a sync marker in a nested object is not the top-level sync"
        );
        assert!(decode(r#"{"Nested":{"MiscItems":[],"LastInventorySync":123}}"#).is_none());
        assert!(decode(r#"{"MiscItems":[],"LastInventorySync":123}{"LastInventorySync":456}"#).is_some());
        let raw = br#"{"MiscItems":[],"LastInventorySync":123}{"LastInventorySync":456}"#;
        let second = memmem::find_iter(raw, MARKER).nth(1).unwrap();
        assert!(decode_blob(raw, second).is_none(), "an earlier object must not stand in for the hit");
    }

    #[test]
    fn scans_region_boundaries_and_partial_reads() {
        let mut bytes = vec![b'x'; CHUNK + 200];
        let positions = [0, CHUNK - 5, CHUNK + 100];
        for &pos in &positions {
            bytes[pos..pos + MARKER.len()].copy_from_slice(MARKER);
        }
        let base = 1000;
        let end = base + bytes.len() as u64;
        for limit in [CHUNK + MARKER.len(), 700_000] {
            let mut hits = Vec::new();
            let mut progress = 0;
            scan_region(
                (base, end),
                |addr, len| {
                    assert!(addr + len as u64 <= end, "reads must not cross the region boundary");
                    let offset = (addr - base) as usize;
                    bytes[offset..offset + len.min(limit)].to_vec()
                },
                |addr| hits.push((addr - base) as usize),
                |n| progress += n,
            );
            assert_eq!(hits, positions);
            assert_eq!(progress, bytes.len() as u64);
        }
    }

    #[test]
    fn retries_failed_large_reads_and_skips_unreadable_pages() {
        let mut bytes = vec![b'x'; 150_000];
        let positions = [100, 10_000, 100_000];
        for &pos in &positions {
            bytes[pos..pos + MARKER.len()].copy_from_slice(MARKER);
        }
        let mut hits = Vec::new();
        let mut progress = 0;
        scan_region(
            (0, bytes.len() as u64),
            |addr, len| {
                let start = addr as usize;
                let end = start + len;
                if len > 4096 || (start < 8192 && end > 4096) { Vec::new() } else { bytes[start..end].to_vec() }
            },
            |addr| hits.push(addr as usize),
            |n| progress += n,
        );
        assert_eq!(hits, positions);
        assert_eq!(progress, bytes.len() as u64);
    }

    #[test]
    fn a_capture_that_lost_a_usual_section_is_refused() {
        let whole = json!({"MiscItems": [], "Recipes": [], "Suits": [], "LastInventorySync": 1});
        let mut s = Sections::default();
        assert!(s.missing(&json!({})).is_empty(), "nothing is expected before a few captures");
        for _ in 0..3 {
            s.learn(&whole);
        }
        // The head went missing: Suits and what came before it are gone.
        let cut = json!({"Recipes": [], "LastInventorySync": 2});
        assert_eq!(s.missing(&cut), ["MiscItems", "Suits"]);
        assert!(s.missing(&whole).is_empty());
        // A section seen once (the rarely surviving PlayerLevel) is not expected.
        let mut with_level = whole.clone();
        with_level["PlayerLevel"] = json!(0);
        s.learn(&with_level);
        assert!(s.missing(&whole).is_empty());
        // Repeated partial captures do not erase the learned expectations.
        for _ in 0..5 {
            assert_eq!(s.missing(&cut), ["MiscItems", "Suits"]);
        }
    }

    #[test]
    fn older_section_statistics_with_refusals_still_load() {
        let s: Sections = serde_json::from_value(json!({
            "captures": 3, "seen": {"MiscItems": 3}, "refused": 3
        }))
        .unwrap();
        assert_eq!(s.missing(&json!({"Suits": []})), ["MiscItems"]);
    }

    #[test]
    fn sanitise_keeps_inventory_and_drops_secrets() {
        let v = json!({"Token": "s3cret", "TwitchAuthToken": "x", "Nonce": 1, "MiscItems": [{"ItemType": "/A",
            "ItemCount": 2, "Fingerprint": "f"}], "PendingRecipes": [{"ItemType": "/R",
            "CompletionDate": {"$date": {"$numberLong": "1"}}}], "PremiumCredits": 5});
        let s = sanitise(v);
        assert_eq!(
            s,
            json!({"MiscItems": [{"ItemType": "/A", "ItemCount": 2}], "PendingRecipes": [{"ItemType": "/R",
            "CompletionDate": {"$date": {"$numberLong": "1"}}}], "PremiumCredits": 5})
        );
    }

    #[test]
    fn standing_and_intrinsics_are_kept() {
        let v = json!({"Affiliations": [{"Tag": "CetusSyndicate", "Standing": 23143, "Title": 1, "FreeFavorsUsed": []}],
            "DailyAffiliationCetus": 16000, "DailyFocus": 250000, "AuthToken": "x",
            "PlayerSkills": {"LPS_PILOTING": 7, "LPP_SPACE": 29267, "SessionKey": "s"},
            "Missions": [{"Tag": "SolNode27", "Completes": 2, "Tier": 1, "BestRating": 3}]});
        assert_eq!(
            sanitise(v),
            json!({"Affiliations": [{"Tag": "CetusSyndicate", "Standing": 23143, "Title": 1}],
                "DailyAffiliationCetus": 16000, "DailyFocus": 250000,
                "PlayerSkills": {"LPS_PILOTING": 7, "LPP_SPACE": 29267},
                "Missions": [{"Tag": "SolNode27", "Completes": 2, "Tier": 1}]})
        );
    }

    #[test]
    fn captures_are_kept_newest_first_one_per_sync() {
        let cap = |sync: i64, keys: usize| {
            let mut v = json!({"LastInventorySync": sync});
            for k in 0..keys {
                v[format!("K{k}")] = json!(1);
            }
            v
        };
        let mut found = Vec::new();
        for c in [cap(10, 1), cap(30, 1), cap(20, 1), cap(30, 4), cap(30, 2)] {
            keep_capture(&mut found, c);
        }
        let syncs: Vec<i64> = found.iter().map(sync_time).collect();
        assert_eq!(syncs, [30, 20, 10]);
        assert_eq!(found[0].as_object().unwrap().len(), 5, "the most complete copy of a sync stands for it");
        for s in 40..50 {
            keep_capture(&mut found, cap(s, 1));
        }
        assert_eq!(found.len(), MAX_CAPTURES);
        assert_eq!(sync_time(&found[0]), 49);
    }

    #[test]
    fn object_id_times() {
        assert_eq!(oid_time(&json!({"$oid": "5f1e2d3c4b5a69788796a5b4"})), 0x5f1e2d3c);
        assert_eq!(oid_time(&json!(null)), 0);
    }
}
