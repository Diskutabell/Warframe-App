//! State the summary shows, guarded so that no write goes unannounced.

use std::sync::{Mutex, MutexGuard};

use super::lock;
use crate::bus::Bus;

/// Reading it changes nothing; every write marks the summary dirty, so the UI is sent a fresh one
/// (`Bus::state_changed`). A read guard cannot be written through, so the compiler finds any
/// write that would otherwise go unannounced.
pub struct Watched<T> {
    value: Mutex<T>,
    bus: Bus,
}

impl<T> Watched<T> {
    pub fn new(value: T, bus: &Bus) -> Watched<T> {
        Watched { value: Mutex::new(value), bus: bus.clone() }
    }

    pub fn read(&self) -> Read<'_, T> {
        Read(lock(&self.value))
    }

    pub fn write(&self) -> Write<'_, T> {
        Write { guard: lock(&self.value), bus: &self.bus }
    }
}

pub struct Read<'a, T>(MutexGuard<'a, T>);

impl<T> std::ops::Deref for Read<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

pub struct Write<'a, T> {
    guard: MutexGuard<'a, T>,
    bus: &'a Bus,
}

impl<T> std::ops::Deref for Write<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for Write<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Write<'_, T> {
    fn drop(&mut self) {
        self.bus.state_changed();
    }
}
