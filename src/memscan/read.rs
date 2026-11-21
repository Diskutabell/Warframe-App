//! Reading the inventory text out of the game's memory: the regions to read, the marker to look
//! for, and copying out (and sanitising) the JSON around each one. Only compiled with the
//! `memscan` feature.

use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Read as _;
use std::sync::LazyLock;
use std::time::Instant;

use memchr::memmem;
use regex::Regex;
use serde_json::{Map, Value};

use super::{ScanError, ScanInfo};

const MARKER: &[u8] = b"\"LastInventorySync\"";
/// The same process memory holds the login payload, with the live session Token and
/// TwitchAuthToken. Nothing outside this allowlist is ever kept - a leaked session token is worth
/// more to an attacker than the inventory is to anyone. `allowed` adds three families of keys
/// whose names vary: the daily standing pools (`DailyAffiliationCetus`...) and intrinsic ranks
/// and points (`LPS_PILOTING`, `LPP_SPACE`).
const KEEP_KEYS: [&str; 48] = [
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
    // A ranked mod's or arcane's rank, as a JSON string inside its `Upgrades` entry: {"lvl": 5}.
    "UpgradeFingerprint",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadError {
    /// Localized unreadable or unmapped page(s): skip and continue scanning.
    Unreadable,
    /// The target process exited mid-scan: abort immediately.
    ProcessGone,
    /// Permission to read the target process was revoked: abort immediately.
    AccessDenied,
}

/// Overwrites sensitive memory using volatile writes and a compiler fence so the optimizer cannot
/// elide the wipe before the buffer is freed.
fn zeroize_bytes(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` points to a valid, aligned byte inside `buf`.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn zeroize_string(mut s: String) {
    // SAFETY: zero bytes are valid UTF-8, and `s` is dropped immediately after.
    unsafe { zeroize_bytes(s.as_bytes_mut()) };
    drop(s);
}

#[cfg(test)]
fn zeroize_value(v: Value) {
    match v {
        Value::String(s) => zeroize_string(s),
        Value::Array(items) => {
            for item in items {
                zeroize_value(item);
            }
        }
        Value::Object(map) => {
            for (k, val) in map {
                zeroize_string(k);
                zeroize_value(val);
            }
        }
        _ => {}
    }
}

/// Byte buffer holding raw game memory (which may contain session tokens): wiped on drop.
struct SensitiveBuf(Vec<u8>);

impl SensitiveBuf {
    fn new(buf: Vec<u8>) -> Self {
        Self(buf)
    }

    fn with_capacity(cap: usize) -> Self {
        Self(Vec::with_capacity(cap))
    }

    fn zeroed(len: usize) -> Self {
        Self(vec![0u8; len])
    }
}

impl std::ops::Deref for SensitiveBuf {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SensitiveBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for SensitiveBuf {
    fn drop(&mut self) {
        zeroize_bytes(&mut self.0);
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

/// Keeps only known inventory fields; dropped strings are wiped before freeing.
#[cfg(test)]
pub fn sanitise(v: Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.into_iter().map(sanitise).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, val) in map {
                if allowed(&k) {
                    out.insert(k, sanitise(val));
                } else {
                    zeroize_string(k);
                    zeroize_value(val);
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

struct CowKey<'de>(Cow<'de, str>);

impl<'de> serde::Deserialize<'de> for CowKey<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Cow<'de, str>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON key")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Cow::Borrowed(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Cow::Owned(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(Cow::Owned(v))
            }
        }
        deserializer.deserialize_str(V).map(CowKey)
    }
}

fn parse_oid_prefix(s: &str) -> i64 {
    s.get(..8).and_then(|h| i64::from_str_radix(h, 16).ok()).unwrap_or(0)
}

struct OidTime(i64);

impl<'de> serde::Deserialize<'de> for OidTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = i64;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an ObjectId string")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<i64, E> {
                Ok(parse_oid_prefix(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<i64, E> {
                Ok(parse_oid_prefix(v))
            }
            fn visit_string<E>(self, v: String) -> Result<i64, E> {
                let ts = parse_oid_prefix(&v);
                zeroize_string(v);
                Ok(ts)
            }
            fn visit_bool<E>(self, _: bool) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_i64<E>(self, _: i64) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_u64<E>(self, _: u64) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_f64<E>(self, _: f64) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_unit<E>(self) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_none<E>(self) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<i64, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(0)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<i64, A::Error> {
                while map.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?.is_some() {}
                Ok(0)
            }
        }
        deserializer.deserialize_any(V).map(OidTime)
    }
}

/// Parses `LastInventorySync` (an integer timestamp or `{"$oid": "..."}`) straight to `i64`.
struct SyncValue(i64);

impl<'de> serde::Deserialize<'de> for SyncValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = i64;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a sync timestamp or Mongo ObjectId")
            }
            fn visit_i64<E>(self, v: i64) -> Result<i64, E> {
                Ok(v)
            }
            fn visit_u64<E>(self, v: u64) -> Result<i64, E> {
                Ok(v as i64)
            }
            fn visit_f64<E>(self, _: f64) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_bool<E>(self, _: bool) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_unit<E>(self) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_none<E>(self) -> Result<i64, E> {
                Ok(0)
            }
            fn visit_borrowed_str<E>(self, s: &'de str) -> Result<i64, E> {
                Ok(parse_oid_prefix(s))
            }
            fn visit_str<E>(self, s: &str) -> Result<i64, E> {
                Ok(parse_oid_prefix(s))
            }
            fn visit_string<E>(self, s: String) -> Result<i64, E> {
                let ts = parse_oid_prefix(&s);
                zeroize_string(s);
                Ok(ts)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<i64, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(0)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<i64, A::Error> {
                let mut ts = 0i64;
                while let Some(CowKey(k)) = map.next_key::<CowKey<'de>>()? {
                    if k.as_ref() == "$oid" {
                        ts = map.next_value::<OidTime>()?.0;
                    } else {
                        let _ = map.next_value::<serde::de::IgnoredAny>()?;
                    }
                    if let Cow::Owned(s) = k {
                        zeroize_string(s);
                    }
                }
                Ok(ts)
            }
        }
        deserializer.deserialize_any(V).map(SyncValue)
    }
}

/// Deserializes JSON directly into a sanitised `Value`: disallowed fields (such as `Token` or
/// `TwitchAuthToken`) are skipped via `IgnoredAny` without ever allocating a `String` on the heap.
struct SanitisedValue(Value);

impl<'de> serde::Deserialize<'de> for SanitisedValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null))
            }
            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
                let mut items = Vec::new();
                while let Some(SanitisedValue(v)) = seq.next_element()? {
                    items.push(v);
                }
                Ok(Value::Array(items))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
                let mut out = Map::new();
                while let Some(CowKey(k)) = map.next_key::<CowKey<'de>>()? {
                    if k.as_ref() == "LastInventorySync" {
                        let SyncValue(t) = map.next_value()?;
                        out.insert(k.into_owned(), Value::from(t));
                    } else if allowed(k.as_ref()) {
                        let SanitisedValue(v) = map.next_value()?;
                        out.insert(k.into_owned(), v);
                    } else {
                        let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        if let Cow::Owned(s) = k {
                            zeroize_string(s);
                        }
                    }
                }
                Ok(Value::Object(out))
            }
        }
        deserializer.deserialize_any(V).map(SanitisedValue)
    }
}

// ------------------------------------------------------------------ the operating system's part

/// Linux: the game's memory through /proc. Windows has its own version in `windows/memory.rs`
/// with the same functions.
#[cfg(unix)]
mod os {
    use std::ffi::CString;
    use std::fs;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::FileExt;

    use super::{ReadError, ScanError};

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

    fn is_warframe_exe_arg(arg: &str) -> bool {
        let base = arg.rsplit(['/', '\\']).next().unwrap_or(arg);
        base.eq_ignore_ascii_case("Warframe.x64.exe")
    }

    /// Matches only argv[0] = `Warframe.x64.exe` or Wine invoking `Warframe.x64.exe` as its first
    /// `.exe` argument, so wrappers (`bwrap`, `pressure-vessel`, `reaper`, `umu`) and unrelated
    /// tools (editors, log viewers) are never mistaken for the game.
    pub(super) fn cmdline_is_game(cmd: &[u8]) -> bool {
        let args: Vec<String> =
            cmd.split(|&b| b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
        let Some(first) = args.first() else { return false };
        if is_warframe_exe_arg(first) {
            return true;
        }
        let prog = first.rsplit('/').next().unwrap_or(first);
        let is_wine = prog == "wine"
            || prog == "wine64"
            || prog.starts_with("wine-preloader")
            || prog.starts_with("wine64-preloader");
        if !is_wine {
            return false;
        }
        let first_exe = args.iter().skip(1).find(|a| a.to_ascii_lowercase().ends_with(".exe"));
        first_exe.is_some_and(|a| is_warframe_exe_arg(a))
    }

    /// The game process: the one named after its exe (`comm`, which the kernel cuts to 15
    /// characters, so "Warframe.x64.ex"). Only when no process is named so does the command line
    /// decide, for a Wine that names its processes differently.
    pub fn find_game_pid() -> Option<u32> {
        let mut by_command_line = None;
        for entry in fs::read_dir("/proc").ok()?.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            if comm.trim().starts_with("Warframe.x64") {
                return Some(pid);
            }
            if by_command_line.is_some() {
                continue;
            }
            let cmd = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if cmdline_is_game(&cmd) {
                by_command_line = Some(pid);
            }
        }
        by_command_line
    }

    /// Private, writable, non-library regions from `/proc/<pid>/maps`: where downloaded JSON lives.
    pub(super) fn parse_maps(maps: &str) -> Vec<(u64, u64)> {
        maps.lines()
            .filter_map(|line| {
                let mut parts = line.split_ascii_whitespace();
                let range = parts.next()?;
                let perms = parts.next()?;
                let _offset = parts.next()?;
                let _dev = parts.next()?;
                let inode = parts.next()?;
                if !perms.starts_with("rw") || !perms.ends_with('p') {
                    return None;
                }
                // Everything after the inode column is the pathname, which may contain spaces
                // (common under Steam/Proton libraries).
                let inode_end = inode.as_ptr() as usize - line.as_ptr() as usize + inode.len();
                let path = line[inode_end..].trim();
                let clean = path.strip_suffix(" (deleted)").unwrap_or(path);
                let lower = clean.to_ascii_lowercase();
                if lower.ends_with(".so")
                    || lower.contains(".so.")
                    || lower.ends_with(".dll")
                    || lower.ends_with(".exe")
                    || clean.starts_with("/dev/")
                {
                    return None;
                }
                let (a, b) = range.split_once('-')?;
                let (a, b) = (u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?);
                (b > a).then_some((a, b))
            })
            .collect()
    }

    fn open_at(dir: &OwnedFd, name: &str) -> std::io::Result<fs::File> {
        let cname = CString::new(name).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        // SAFETY: `dir` is a valid open directory FD and `cname` is NUL-terminated.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), cname.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `openat` returned a fresh file descriptor owned by us.
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }

    /// The game's memory, opened through a pinned `/proc/<pid>` directory FD so `maps`, `mem`,
    /// and liveness checks always refer to the exact same process even if the PID is recycled.
    pub struct Memory {
        proc_dir: OwnedFd,
        file: fs::File,
    }

    impl Memory {
        /// Opening is where the kernel decides; a refusal costs nothing but a line in its log.
        pub fn open(pid: u32) -> Result<Memory, ScanError> {
            let map_err = |e: std::io::Error| match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    let scope = ptrace_scope().map_or_else(|| "not set".to_string(), |s| s.to_string());
                    ScanError::Denied(format!(
                        "Linux refused access to the game's memory (kernel.yama.ptrace_scope is {scope}). It is tried \
                         again when the game restarts; the app never changes that setting."
                    ))
                }
                std::io::ErrorKind::NotFound => ScanError::NotRunning,
                _ => ScanError::Io(e.to_string()),
            };
            let dir_path = CString::new(format!("/proc/{pid}")).map_err(|e| ScanError::Io(e.to_string()))?;
            // SAFETY: `dir_path` is NUL-terminated; O_DIRECTORY ensures it is the proc directory.
            let dir_fd = unsafe { libc::open(dir_path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
            if dir_fd < 0 {
                return Err(map_err(std::io::Error::last_os_error()));
            }
            // SAFETY: `open` returned a fresh descriptor owned by us.
            let proc_dir = unsafe { OwnedFd::from_raw_fd(dir_fd) };
            let file = open_at(&proc_dir, "mem").map_err(map_err)?;
            Ok(Memory { proc_dir, file })
        }

        pub fn regions(&self) -> Vec<(u64, u64)> {
            let Ok(mut f) = open_at(&self.proc_dir, "maps") else { return Vec::new() };
            let mut maps = String::new();
            if f.read_to_string(&mut maps).is_err() {
                return Vec::new();
            }
            parse_maps(&maps)
        }

        fn alive(&self) -> bool {
            let Ok(mut f) = open_at(&self.proc_dir, "stat") else { return false };
            let mut stat = String::new();
            if f.read_to_string(&mut stat).is_err() {
                return false;
            }
            // State character sits right after the closing ')' of comm: 'Z' / 'X' means exited.
            stat.rsplit_once(')')
                .and_then(|(_, rest)| rest.split_ascii_whitespace().next())
                .is_some_and(|state| !matches!(state, "Z" | "X" | "x"))
        }

        /// Copies up to `buf.len()` bytes starting at `addr` into `buf`, distinguishing fatal
        /// process exit / permission loss from localized unreadable pages.
        pub fn read_into(&self, addr: u64, buf: &mut [u8]) -> Result<usize, ReadError> {
            if buf.is_empty() {
                return Ok(0);
            }
            match self.file.read_at(buf, addr) {
                Ok(0) => {
                    if !self.alive() {
                        Err(ReadError::ProcessGone)
                    } else {
                        Ok(0)
                    }
                }
                Ok(n) => Ok(n),
                Err(e) => match e.raw_os_error() {
                    Some(libc::ESRCH) => Err(ReadError::ProcessGone),
                    Some(libc::EACCES | libc::EPERM) => Err(ReadError::AccessDenied),
                    _ if !self.alive() => Err(ReadError::ProcessGone),
                    _ => Err(ReadError::Unreadable),
                },
            }
        }
    }
}

#[cfg(windows)]
#[path = "../windows/memory.rs"]
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

/// Reads the exact range `[addr, addr + buf.len())`, looping on short reads (such as crossing a
/// sub-region boundary on Windows) until filled or an unreadable page / fatal error is reached.
fn read_exact_range(
    read: &impl Fn(u64, &mut [u8]) -> Result<usize, ReadError>,
    addr: u64,
    buf: &mut [u8],
) -> Result<bool, ReadError> {
    let mut filled = 0;
    while filled < buf.len() {
        match read(addr + filled as u64, &mut buf[filled..]) {
            Ok(0) => return Ok(false),
            Ok(n) => filled += n,
            Err(ReadError::Unreadable) => return Ok(false),
            Err(fatal) => return Err(fatal),
        }
    }
    Ok(true)
}

/// Grows outward from the marker until NUL bytes bound the text, then decodes the JSON object.
fn extract_blob(mem: &os::Memory, (start_lim, end_lim): (u64, u64), hit: u64) -> Result<Option<Value>, ReadError> {
    extract_blob_from(|addr, buf| mem.read_into(addr, buf), (start_lim, end_lim), hit)
}

/// Retains every chunk read during the backward and forward scans and stitches them once (never
/// re-reading live memory a second time), clamping strictly to `MAX_EXTENT` in each direction.
fn extract_blob_from(
    read: impl Fn(u64, &mut [u8]) -> Result<usize, ReadError>,
    (start_lim, end_lim): (u64, u64),
    hit: u64,
) -> Result<Option<Value>, ReadError> {
    const STEP: u64 = 1024 * 1024;
    let mut lo = hit;
    let mut prefix_chunks: Vec<SensitiveBuf> = Vec::new();
    let mut scratch = SensitiveBuf::zeroed(STEP as usize);

    while lo > start_lim && hit - lo < MAX_EXTENT {
        let budget = (lo - start_lim).min(MAX_EXTENT - (hit - lo));
        // Try 1 MiB, then 64 KiB, 4 KiB, and finally any partial tail up to a 4 KiB page boundary
        // so one unreadable page inside a 1 MiB step does not discard the whole readable prefix.
        let page_rem = lo % 4096;
        let candidates = [STEP.min(budget), (64 * 1024).min(budget), 4096u64.min(budget), page_rem.min(budget)];
        let mut step_got = None;
        let mut last_tried = 0u64;
        for &want in &candidates {
            if want == 0 || want == last_tried {
                continue;
            }
            last_tried = want;
            let a = lo - want;
            let slice = &mut scratch[..want as usize];
            if read_exact_range(&read, a, slice)? {
                step_got = Some((a, want as usize));
                break;
            }
        }
        let Some((a, want)) = step_got else {
            break;
        };
        let slice = &scratch[..want];
        if let Some(z) = memchr::memrchr(0, slice) {
            if z + 1 < want {
                prefix_chunks.push(SensitiveBuf::new(slice[z + 1..].to_vec()));
            }
            lo = a + z as u64 + 1;
            break;
        }
        prefix_chunks.push(SensitiveBuf::new(slice.to_vec()));
        lo = a;
    }

    let mut hi = hit;
    let mut suffix = SensitiveBuf::with_capacity(64 * 1024);
    while hi < end_lim && hi - hit < MAX_EXTENT {
        let budget = (end_lim - hi).min(MAX_EXTENT - (hi - hit));
        let want = STEP.min(budget) as usize;
        let mut got = match read(hi, &mut scratch[..want]) {
            Ok(n) => n,
            Err(ReadError::Unreadable) => 0,
            Err(fatal) => return Err(fatal),
        };
        if got == 0 && want > 64 * 1024 {
            got = match read(hi, &mut scratch[..64 * 1024]) {
                Ok(n) => n,
                Err(ReadError::Unreadable) => 0,
                Err(fatal) => return Err(fatal),
            };
        }
        if got == 0 && want > 4096 {
            got = match read(hi, &mut scratch[..4096]) {
                Ok(n) => n,
                Err(ReadError::Unreadable) => 0,
                Err(fatal) => return Err(fatal),
            };
        }
        if got == 0 {
            break;
        }
        let slice = &scratch[..got];
        if let Some(z) = memchr::memchr(0, slice) {
            suffix.extend_from_slice(&slice[..z]);
            break;
        }
        suffix.extend_from_slice(slice);
        hi += got as u64;
    }

    let prefix_len: usize = prefix_chunks.iter().map(|c| c.len()).sum();
    if prefix_len as u64 != hit - lo {
        return Ok(None);
    }
    let mut raw = SensitiveBuf::with_capacity(prefix_len + suffix.len());
    for chunk in prefix_chunks.iter().rev() {
        raw.extend_from_slice(chunk);
    }
    raw.extend_from_slice(&suffix);
    drop(prefix_chunks);
    drop(suffix);
    drop(scratch);
    Ok(decode_blob(&raw, prefix_len))
}

/// Recovers a JSON suffix whose beginning was overwritten, streaming directly into a sanitised
/// `Value` without copying the buffer on retry or allocating heap strings for skipped secrets.
fn decode_blob(raw: &[u8], hit: usize) -> Option<Value> {
    if raw.get(hit..hit.checked_add(MARKER.len())?)? != MARKER {
        return None;
    }

    let decode = |start: usize, prepend: bool| -> Option<Value> {
        let prefix = usize::from(prepend);
        let (obj, consumed) = if prepend {
            let mut stream = serde_json::Deserializer::from_reader(b"{".as_slice().chain(&raw[start..]))
                .into_iter::<SanitisedValue>();
            let SanitisedValue(obj) = stream.next()?.ok()?;
            (obj, stream.byte_offset())
        } else {
            let mut stream = serde_json::Deserializer::from_slice(&raw[start..]).into_iter::<SanitisedValue>();
            let SanitisedValue(obj) = stream.next()?.ok()?;
            (obj, stream.byte_offset())
        };
        // The parsed object must contain this hit, not an unrelated earlier JSON object.
        if consumed < hit - start + prefix + MARKER.len()
            || obj.get("LastInventorySync").is_none()
            || (obj.get("MiscItems").is_none() && obj.get("Suits").is_none())
        {
            return None;
        }
        Some(obj)
    };

    let start = raw.iter().position(|b| !b.is_ascii_whitespace())?;
    if start <= hit {
        if raw[start] == b'{' {
            if let Some(obj) = decode(start, false) {
                return Some(obj);
            }
            let mut stream = serde_json::Deserializer::from_slice(&raw[start..]).into_iter::<serde::de::IgnoredAny>();
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
    // Single reusable scratch buffer across the entire scan; wiped on drop.
    let mut scratch = SensitiveBuf::zeroed(CHUNK + MARKER.len() - 1);
    let mut fatal: Option<ReadError> = None;
    for &(a, b) in &regions {
        let mut hit_fatal: Option<ReadError> = None;
        let res = scan_region(
            (a, b),
            &mut scratch,
            |addr, buf| mem.read_into(addr, buf),
            |addr| {
                if !seen.insert(addr) || hit_fatal.is_some() {
                    return;
                }
                hits += 1;
                match extract_blob(&mem, (a, b), addr) {
                    Ok(Some(obj)) => keep_capture(&mut found, obj),
                    Ok(None) => {}
                    Err(e) => hit_fatal = Some(e),
                }
            },
            |advanced| {
                done += advanced;
                progress(done as f64 / total as f64);
            },
        );
        if let Err(e) = res {
            fatal = Some(e);
            break;
        }
        if let Some(e) = hit_fatal {
            fatal = Some(e);
            break;
        }
    }
    drop(scratch);
    let info = ScanInfo { seconds: (started.elapsed().as_secs_f64() * 10.0).round() / 10.0 };
    if !found.is_empty() {
        return Ok((found, info));
    }
    match fatal {
        Some(ReadError::ProcessGone) => Err(ScanError::NotRunning),
        Some(ReadError::AccessDenied) => Err(ScanError::Denied("Access to the game's memory was revoked.".into())),
        _ => Err(ScanError::NotFound { matches: hits }),
    }
}

/// Reads within region boundaries into a reused scratch buffer, retaining overlap even after a
/// short read. Aborts immediately on fatal process errors (`ProcessGone` / `AccessDenied`), and
/// retries smaller sizes on localized unreadable pages before skipping to the next 4 KiB page.
fn scan_region(
    (start, end): (u64, u64),
    scratch: &mut [u8],
    read: impl Fn(u64, &mut [u8]) -> Result<usize, ReadError>,
    mut hit: impl FnMut(u64),
    mut progress: impl FnMut(u64),
) -> Result<(), ReadError> {
    let finder = memmem::Finder::new(MARKER);
    let mut off = start;
    let max_chunk = CHUNK.min(scratch.len().saturating_sub(MARKER.len() - 1)).max(1);
    while off < end {
        let remaining = end - off;
        let n = remaining.min(max_chunk as u64) as usize;
        let request = remaining.min((n + MARKER.len() - 1) as u64).min(scratch.len() as u64) as usize;
        let mut got = match read(off, &mut scratch[..request]) {
            Ok(len) => len,
            Err(ReadError::Unreadable) => 0,
            Err(fatal) => return Err(fatal),
        };
        if got == 0 && request > 64 * 1024 {
            got = match read(off, &mut scratch[..64 * 1024]) {
                Ok(len) => len,
                Err(ReadError::Unreadable) => 0,
                Err(fatal) => return Err(fatal),
            };
        }
        if got == 0 && request > 4096 {
            got = match read(off, &mut scratch[..4096]) {
                Ok(len) => len,
                Err(ReadError::Unreadable) => 0,
                Err(fatal) => return Err(fatal),
            };
        }
        // A short read may only have stopped where one of the system's memory regions ends (a
        // Windows read never crosses one), with the next one readable right after it: read on, or
        // a marker lying across that border would never be whole in the buffer.
        while got > 0 && got < request {
            match read(off + got as u64, &mut scratch[got..request]) {
                Ok(0) | Err(ReadError::Unreadable) => break,
                Ok(more) => got += more,
                Err(fatal) => return Err(fatal),
            }
        }
        let next_page = || remaining.min(4096 - (off % 4096)) as usize;
        let advance = if got == 0 {
            next_page()
        } else if got < request {
            if got < MARKER.len() {
                // Too few bytes before an unreadable boundary to hold the marker: jump straight to
                // the next 4 KiB page instead of crawling byte by byte.
                next_page().max(got)
            } else {
                got - (MARKER.len() - 1)
            }
        } else {
            n
        };
        for at in finder.find_iter(&scratch[..got]).filter(|&at| at < advance) {
            hit(off + at as u64);
        }
        off += advance as u64;
        progress(advance as u64);
    }
    Ok(())
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
        let read = |addr: u64, buf: &mut [u8]| -> Result<usize, ReadError> {
            let start = addr as usize;
            let n = buf.len().min(raw.len().saturating_sub(start));
            buf[..n].copy_from_slice(&raw[start..start + n]);
            Ok(n)
        };
        let mut scratch = vec![0u8; 4096];
        let mut captures = Vec::new();
        scan_region(
            regions[0],
            &mut scratch,
            read,
            |addr| captures.push(extract_blob_from(read, regions[0], addr).unwrap()),
            |_| {},
        )
        .unwrap();
        assert_eq!(captures, [Some(json!({"Suits": [{"ItemType": "/A"}], "MiscItems": [], "LastInventorySync": 123}))]);
        assert!(extract_blob_from(read, ((hit - 3) as u64, raw.len() as u64), hit as u64).unwrap().is_none());
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
        let mut scratch = vec![0u8; CHUNK + MARKER.len() - 1];
        for limit in [CHUNK + MARKER.len(), 700_000] {
            let mut hits = Vec::new();
            let mut progress = 0;
            scan_region(
                (base, end),
                &mut scratch,
                |addr, buf| {
                    assert!(addr + buf.len() as u64 <= end, "reads must not cross the region boundary");
                    let offset = (addr - base) as usize;
                    let n = buf.len().min(limit);
                    buf[..n].copy_from_slice(&bytes[offset..offset + n]);
                    Ok(n)
                },
                |addr| hits.push((addr - base) as usize),
                |n| progress += n,
            )
            .unwrap();
            assert_eq!(hits, positions);
            assert_eq!(progress, bytes.len() as u64);
        }
    }

    #[test]
    fn a_marker_across_the_border_of_two_regions_is_found() {
        // Two readable regions side by side; a read stops at the border between them, as Windows'
        // ReadProcessMemory does at the end of a VirtualQueryEx region.
        let border = 4096 * 3;
        let mut bytes = vec![b'x'; border * 2];
        let at = border - 7;
        bytes[at..at + MARKER.len()].copy_from_slice(MARKER);
        let mut hits = Vec::new();
        let mut scratch = vec![0u8; CHUNK + MARKER.len() - 1];
        scan_region(
            (0, bytes.len() as u64),
            &mut scratch,
            |addr, buf| {
                let start = addr as usize;
                let stop = if start < border { border } else { bytes.len() };
                let n = buf.len().min(stop - start);
                buf[..n].copy_from_slice(&bytes[start..start + n]);
                Ok(n)
            },
            |addr| hits.push(addr as usize),
            |_| {},
        )
        .unwrap();
        assert_eq!(hits, [at]);
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
        let mut scratch = vec![0u8; CHUNK + MARKER.len() - 1];
        scan_region(
            (0, bytes.len() as u64),
            &mut scratch,
            |addr, buf| {
                let start = addr as usize;
                let end = start + buf.len();
                if buf.len() > 4096 || (start < 8192 && end > 4096) {
                    Err(ReadError::Unreadable)
                } else {
                    buf.copy_from_slice(&bytes[start..end]);
                    Ok(buf.len())
                }
            },
            |addr| hits.push(addr as usize),
            |n| progress += n,
        )
        .unwrap();
        assert_eq!(hits, positions);
        assert_eq!(progress, bytes.len() as u64);
    }

    #[test]
    fn fatal_read_error_aborts_scan_and_short_partial_jumps_page() {
        let calls = std::cell::Cell::new(0usize);
        let mut scratch = vec![0u8; 64 * 1024];
        let err = scan_region(
            (0, 1024 * 1024),
            &mut scratch,
            |_, _| {
                calls.set(calls.get() + 1);
                Err(ReadError::ProcessGone)
            },
            |_| {},
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err, ReadError::ProcessGone);
        assert_eq!(calls.get(), 1, "fatal error must abort immediately on the first read");

        // Partial read of 5 bytes (< MARKER.len()) right before an unreadable boundary jumps to
        // the next 4 KiB page instead of crawling 1 byte at a time.
        let reads = std::cell::Cell::new(0usize);
        scan_region(
            (4091, 8192),
            &mut scratch,
            |addr, buf| {
                reads.set(reads.get() + 1);
                if addr == 4091 {
                    buf[..5].fill(b'a');
                    Ok(5)
                } else {
                    buf.fill(b'b');
                    Ok(buf.len())
                }
            },
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(reads.get(), 2, "must jump directly to page 4096 rather than stepping 1 byte at a time");
    }

    #[test]
    fn extract_blob_recovers_across_backward_unreadable_page_in_one_pass() {
        // Unreadable page [0, 4096), readable JSON in [4096, 12288) spanning > 4 KiB before hit.
        let mut mem = vec![b' '; 12288];
        let prefix =
            b"{\"MiscItems\":[{\"ItemType\":\"/A\",\"ItemCount\":1}],\"Suits\":[],\"Token\":\"secret\",\"Padding\":\"";
        let suffix = b"\",\"LastInventorySync\":777}\0";
        mem[4096..4096 + prefix.len()].copy_from_slice(prefix);
        let hit_start = 9000;
        mem[hit_start..hit_start + suffix.len()].copy_from_slice(suffix);
        let hit = memmem::find(&mem, MARKER).unwrap() as u64;

        let read_ranges = std::cell::RefCell::new(Vec::<(u64, u64)>::new());
        let obj = extract_blob_from(
            |addr, buf| {
                let end = addr + buf.len() as u64;
                if addr < 4096 {
                    return Err(ReadError::Unreadable);
                }
                read_ranges.borrow_mut().push((addr, end));
                let n = buf.len().min(mem.len().saturating_sub(addr as usize));
                buf[..n].copy_from_slice(&mem[addr as usize..addr as usize + n]);
                Ok(n)
            },
            (0, mem.len() as u64),
            hit,
        )
        .unwrap()
        .expect("should recover JSON prefix across smaller backward steps");
        assert_eq!(obj["LastInventorySync"], 777);
        assert!(obj.get("Token").is_none());
        // No final full-range re-read after backward/forward scans.
        let ranges = read_ranges.borrow();
        assert!(ranges.iter().all(|&(a, b)| !(a <= 4096 && b > hit)), "must not re-read [lo, hi) a second time");
    }

    #[test]
    fn zeroize_bytes_wipes_buffer() {
        let mut buf = b"live-session-token".to_vec();
        zeroize_bytes(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[cfg(unix)]
    #[test]
    fn linux_maps_and_cmdline_filters_handle_spaces_versioned_libs_and_wrappers() {
        let maps = "\
7f0000000000-7f0000100000 rw-p 00000000 00:00 0\n\
7f0000200000-7f0000300000 rw-s 00000000 00:01 123 /memfd:vulkan\n\
7f0000400000-7f0000500000 rw-p 00001000 08:01 456 /mnt/Steam Library/steamapps/common/Warframe/Game.dll\n\
7f0000600000-7f0000700000 rw-p 00001000 08:01 457 /usr/lib/libc.so.6\n\
7f0000800000-7f0000900000 rw-p 00001000 08:01 458 /usr/lib/libvulkan.so.1 (deleted)\n\
7f0000a00000-7f0000b00000 rw-p 00000000 00:00 0 [heap]\n";
        assert_eq!(os::parse_maps(maps), vec![(0x7f0000000000, 0x7f0000100000), (0x7f0000a00000, 0x7f0000b00000)]);
        assert!(os::cmdline_is_game(b"C:\\Warframe\\Warframe.x64.exe\0-cluster:public\0"));
        assert!(os::cmdline_is_game(b"/proton/bin/wine64-preloader\0/game/Warframe.x64.exe\0"));
        assert!(!os::cmdline_is_game(b"/usr/bin/bwrap\0--ro-bind\0/a\0/b\0Warframe.x64.exe\0"));
        assert!(!os::cmdline_is_game(b"/usr/bin/vim\0Warframe.x64.exe\0"));
        assert!(!os::cmdline_is_game(b"/proton/bin/wine64\0Launcher.exe\0Warframe.x64.exe\0"));
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
