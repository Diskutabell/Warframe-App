//! Every reward screen the app read, so the overview can show what dropped and what it was worth.
//! Kept in the database (`db::Db::record_reward`), without a cap since 0.10.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reward {
    pub name: String,
    pub uid: String,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    /// Still needed for an unfinished set when the screen was read.
    pub needed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub t: f64,
    pub drops: Vec<Reward>,
    pub best: Option<String>,
    /// The key of the account that was logged in (`accounts.rs`). Entries from before 0.3.0 have
    /// none and are shown for every account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}
