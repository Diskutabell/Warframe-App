//! The engine's services. Each owns one part of the state and the background work that keeps it
//! current; `App` (app.rs) wires them together and answers for the API.
//!
//! - `settings`: the settings file, changed one patch at a time.
//! - `shell`: the desktop shell's inbox (window, tray, overlays).
//! - `catalog`: the game data and the names the OCR looks for.
//! - `accounts`: the accounts on this machine, their profiles, inventories and hand-set data,
//!   and DE's profile rate limit.
//! - `world`: the live worldstate, fetched at most once a minute.
//! - `gamelog`: EE.log, followed as the game writes it.
//! - `scans`: reading the reward screen and riven cards, and the overlays that show the results.
//! - `memscan`: when the inventory is read from the game's memory (if switched on).
//! - `notifications`: desktop notifications for watched fissures, Baro and finished builds.

pub mod accounts;
pub mod catalog;
pub mod gamelog;
pub mod memscan;
pub mod notifications;
pub mod scans;
pub mod settings;
pub mod shell;
pub mod watched;
pub mod world;

use std::sync::{Mutex, MutexGuard};

/// Locks a mutex, carrying on with the data when a panicking thread poisoned it: every value
/// guarded this way stays consistent between statements, and one bug must not take the engine
/// down with it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
