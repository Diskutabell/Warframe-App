//! LAST RESORT, OFF BY DEFAULT: a read-only scan of the running game for its inventory JSON.
//!
//! This mirrors what Overwolf's Warframe plugin (used by AlecaFrame) does: the game already holds
//! the inventory it downloaded from DE as JSON text; the scan finds that text (anchored on
//! "LastInventorySync") and copies it out. It never writes to the game and never injects. When
//! enabled it runs by itself after login and after each mission (`State::schedule_memscan`).
//! Whether Linux allows the read is the kernel's decision, made when the game's memory is opened.
//!
//! Digital Extremes' position on third-party tools is "use at your own risk".

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::FileExt;
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
const MAX_REGION: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub enum ScanError {
    NotRunning,
    /// Linux refused to let this process read the game's memory.
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
/// the user, so on such a system the read works at scope 1 - it does on the developer's NixOS.
/// Whether it applies elsewhere only shows when the game's memory is opened.
pub fn availability() -> Result<(), String> {
    match ptrace_scope() {
        Some(3) => Err("kernel.yama.ptrace_scope is 3: Linux lets no program read another program's memory.".into()),
        _ => Ok(()),
    }
}

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
            map.into_iter()
                .filter(|(k, _)| allowed(k))
                .map(|(k, v)| (k, sanitise(v)))
                .collect::<Map<String, Value>>(),
        ),
        other => other,
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
            (b > a && b - a <= MAX_REGION).then_some((a, b))
        })
        .collect()
}

fn read(mem: &fs::File, addr: u64, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    match mem.read_at(&mut buf, addr) {
        Ok(n) => {
            buf.truncate(n);
            buf
        }
        Err(_) => Vec::new(),
    }
}

/// Epoch seconds from a Mongo ObjectId: its first four bytes are a unix timestamp.
fn oid_time(v: &Value) -> i64 {
    let oid = v.get("$oid").unwrap_or(v);
    oid.as_str().and_then(|s| s.get(..8)).and_then(|s| i64::from_str_radix(s, 16).ok()).unwrap_or(0)
}

fn sync_time(v: &Value) -> i64 {
    v["LastInventorySync"].as_i64().unwrap_or_else(|| oid_time(&v["LastInventorySync"]))
}

/// Grows outward from the marker until NUL bytes bound the text, then decodes the JSON object.
fn extract_blob(mem: &fs::File, (start_lim, end_lim): (u64, u64), hit: u64) -> Option<Value> {
    const STEP: u64 = 1024 * 1024;
    let mut lo = hit;
    while lo > start_lim && hit - lo < MAX_EXTENT {
        let a = start_lim.max(lo.saturating_sub(STEP));
        let buf = read(mem, a, (lo - a) as usize);
        if let Some(z) = memchr::memrchr(0, &buf) {
            lo = a + z as u64 + 1;
            break;
        }
        lo = a;
    }
    let mut hi = hit;
    while hi < end_lim && hi - hit < MAX_EXTENT {
        let buf = read(mem, hi, STEP.min(end_lim - hi) as usize);
        if buf.is_empty() {
            break;
        }
        if let Some(z) = memchr::memchr(0, &buf) {
            hi += z as u64;
            break;
        }
        hi += buf.len() as u64;
    }
    let raw = read(mem, lo, (hi - lo) as usize);
    let text = String::from_utf8_lossy(&raw);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // By the time we look, the head of the buffer is usually already reused, so the run starts
    // mid-object at a key boundary ('"SubscribedToEmails":0,...') and ends with its real closing
    // brace. Prepending a brace recovers everything that survived. Taking the first "{" instead
    // would land on a nested value like {"$date": ...} and decode the wrong object.
    let mut candidates: Vec<String> = Vec::new();
    if text.starts_with('{') {
        candidates.push(text.to_string());
    }
    candidates.push(format!("{{{}", text.trim_start_matches(',')));
    if let Some(brace) = text.find('{').filter(|&b| b > 0) {
        candidates.push(text[brace..].to_string());
    }
    for cand in candidates {
        let Some(Ok(mut obj)) = serde_json::Deserializer::from_str(&cand).into_iter::<Value>().next() else {
            continue;
        };
        let looks_right =
            obj.get("LastInventorySync").is_some() && (obj.get("MiscItems").is_some() || obj.get("Suits").is_some());
        if obj.is_object() && looks_right {
            // Resolve the sync ObjectId while its wrapper is intact: sanitise() drops "$oid" (an
            // id is not inventory), and "keep the newest copy" needs the time.
            let t = oid_time(&obj["LastInventorySync"]);
            obj["LastInventorySync"] = Value::from(t);
            return Some(sanitise(obj));
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
    /// Captures refused in a row.
    #[serde(default)]
    pub refused: u32,
}

impl Sections {
    /// A few captures are needed before anything is expected.
    const MIN_CAPTURES: u32 = 3;
    /// After this many refusals in a row the account itself must have changed: start over.
    const MAX_REFUSED: u32 = 4;

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
        self.refused = 0;
        for key in capture.as_object().into_iter().flat_map(|o| o.keys()) {
            *self.seen.entry(key.clone()).or_insert(0) += 1;
        }
    }

    /// Records a refusal; true when it is time to forget what was learned and accept again.
    pub fn refuse(&mut self) -> bool {
        self.refused += 1;
        if self.refused >= Self::MAX_REFUSED {
            *self = Sections::default();
            return true;
        }
        false
    }
}

pub struct ScanInfo {
    /// How long the read took, to a tenth of a second.
    pub seconds: f64,
}

/// How many different captures a scan hands back at most.
const MAX_CAPTURES: usize = 6;

/// Reads the inventory out of the game process `pid`: every distinct copy found, newest sync
/// first. There can be more than one account's: after logging out and in with another account the
/// game may still hold the first one's text, and its sync can even be the newer one. The caller
/// decides which copy is the logged-in account's (`State::apply_capture`).
pub fn scan_inventory(pid: u32, progress: impl Fn(f64)) -> Result<(Vec<Value>, ScanInfo), ScanError> {
    availability().map_err(ScanError::Denied)?;
    // Opening is where the kernel decides; a refusal costs nothing but a line in its log.
    let mem = fs::File::open(format!("/proc/{pid}/mem")).map_err(|e| match e.kind() {
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
    let started = Instant::now();
    let regions = regions(pid);
    let total: u64 = regions.iter().map(|(a, b)| b - a).sum::<u64>().max(1);
    let finder = memmem::Finder::new(MARKER);
    let (mut found, mut hits, mut done) = (Vec::<Value>::new(), 0usize, 0u64);
    let mut seen = HashSet::new();
    for &(a, b) in &regions {
        let mut off = a;
        while off < b {
            let n = CHUNK.min((b - off) as usize);
            // Read a little past the chunk so a marker straddling the boundary is still found; it
            // is counted only in the chunk it starts in.
            let buf = read(&mem, off, n + MARKER.len());
            for at in finder.find_iter(&buf).filter(|&at| at < n) {
                let addr = off + at as u64;
                if !seen.insert(addr) {
                    continue;
                }
                hits += 1;
                if let Some(obj) = extract_blob(&mem, (a, b), addr) {
                    keep_capture(&mut found, obj);
                }
            }
            off += n as u64;
            done += n as u64;
            progress(done as f64 / total as f64);
        }
    }
    let info = ScanInfo { seconds: (started.elapsed().as_secs_f64() * 10.0).round() / 10.0 };
    if found.is_empty() {
        return Err(ScanError::NotFound { matches: hits });
    }
    Ok((found, info))
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
        // Refused again and again, the account must have changed: learn afresh.
        assert!(!s.refuse() && !s.refuse() && !s.refuse());
        assert!(s.refuse());
        assert!(s.missing(&cut).is_empty());
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
