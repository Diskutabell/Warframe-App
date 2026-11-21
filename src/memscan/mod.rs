//! Off by default: a read-only scan of the running game for its inventory JSON.
//!
//! This mirrors what Overwolf's Warframe plugin (used by AlecaFrame) does: the game already holds
//! the inventory it downloaded from DE as JSON text; the scan finds that text (anchored on
//! "LastInventorySync") and copies it out. It never writes to the game and never injects. When
//! enabled it runs by itself after login and after each mission (`State::schedule_memscan`).
//! Whether the system allows the read is decided when the game's memory is opened.
//!
//! Digital Extremes' position on third-party tools is "use at your own risk".
//!
//! Every build has what is here: the errors, what a capture is expected to hold, and the entry
//! points. The reading itself (`read.rs`, with the system parts for Linux and Windows) is only
//! compiled with the `memscan` feature: a build without it has no code that opens or reads
//! another process.

#[cfg(feature = "memscan")]
mod read;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "memscan")]
pub use read::{availability, find_game_pid, scan_inventory};

#[derive(Debug)]
pub enum ScanError {
    NotRunning,
    /// The system refused to let this process read the game's memory.
    Denied(String),
    /// The inventory text is not (completely) in memory right now; it is after the next sync.
    #[cfg_attr(not(feature = "memscan"), allow(dead_code))]
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

/// Built without the memory scan: nothing here reads another process.
#[cfg(not(feature = "memscan"))]
const LEFT_OUT: &str = "This build of warframeproject was made without the memory scan.";

#[cfg(not(feature = "memscan"))]
pub fn availability() -> Result<(), String> {
    Err(LEFT_OUT.to_string())
}

#[cfg(not(feature = "memscan"))]
pub fn find_game_pid() -> Option<u32> {
    None
}

#[cfg(not(feature = "memscan"))]
pub fn scan_inventory(_pid: u32, _progress: impl Fn(f64)) -> Result<(Vec<Value>, ScanInfo), ScanError> {
    Err(ScanError::Denied(LEFT_OUT.to_string()))
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
}
