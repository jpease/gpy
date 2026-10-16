//! Process-environment guard for tests that mutate env vars.
//!
//! Restores on drop (including unwinding); callers must still be `#[serial]`.

#![allow(dead_code)]

use std::ffi::{OsStr, OsString};

/// Restores every environment variable it touched to its original value on drop.
#[derive(Default)]
pub struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    /// Snapshot `keys` and remove them from the environment.
    #[must_use]
    pub fn cleared(keys: &[&'static str]) -> Self {
        let mut guard = Self::default();
        for &key in keys {
            guard.remove(key);
        }
        guard
    }

    /// Snapshot `key` and set it to `value`.
    #[must_use]
    pub fn set_one(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let mut guard = Self::default();
        guard.set(key, value);
        guard
    }

    /// Set `key` to `value`, snapshotting its original value the first time it is touched.
    pub fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.snapshot(key);
        // SAFETY: callers serialize env-mutating tests, so no other thread reads
        // the environment concurrently.
        unsafe { std::env::set_var(key, value) };
    }

    /// Remove `key`, snapshotting its original value the first time it is touched.
    pub fn remove(&mut self, key: &'static str) {
        self.snapshot(key);
        // SAFETY: see `set`.
        unsafe { std::env::remove_var(key) };
    }

    fn snapshot(&mut self, key: &'static str) {
        if !self.saved.iter().any(|(saved, _)| *saved == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, original) in &self.saved {
            // SAFETY: see `EnvGuard::set`.
            match original {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}
