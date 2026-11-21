//! The settings: read by everything, changed one patch at a time and saved at once.

use std::sync::{Mutex, RwLock};

use anyhow::Result;
use serde_json::Value;

use super::lock;
use crate::bus::{Bus, Topic};
use crate::config::Settings;

pub struct SettingsStore {
    value: RwLock<Settings>,
    /// Serialises read-patch-save-swap, so two changes made at once cannot lose one another.
    update: Mutex<()>,
    bus: Bus,
}

/// A change that went through: the settings before and after it.
pub struct Changed {
    pub before: Settings,
    pub after: Settings,
}

impl SettingsStore {
    pub fn new(settings: Settings, bus: &Bus) -> SettingsStore {
        SettingsStore { value: RwLock::new(settings), update: Mutex::new(()), bus: bus.clone() }
    }

    pub fn get(&self) -> Settings {
        self.value.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replaces the settings in memory only. Tests must never write the settings file: the
    /// config directory is the user's real one.
    #[cfg(test)]
    pub fn set_in_memory(&self, settings: Settings) {
        *self.value.write().unwrap_or_else(|e| e.into_inner()) = settings;
    }

    /// Applies a patch from the UI and saves it. A value of the wrong type is refused with a
    /// message the UI can show, and nothing changes.
    pub fn update(&self, patch: &Value) -> Result<Changed> {
        let _one_at_a_time = lock(&self.update);
        let before = self.get();
        let after = before.patched(patch)?;
        // A --port from the command line is for this run only: keep the file's own port unless the
        // patch sets one, or any saved setting would make the override permanent.
        let mut to_save = after.clone();
        if patch.get("port").is_none() {
            to_save.port = Settings::load().port;
        }
        to_save.save()?;
        *self.value.write().unwrap_or_else(|e| e.into_inner()) = after.clone();
        self.bus.signal(Topic::Settings);
        self.bus.state_changed();
        Ok(Changed { before, after })
    }
}
