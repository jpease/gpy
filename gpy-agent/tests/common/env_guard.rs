//! Process-environment guard for tests that mutate env vars.
//!
//! Restores on drop (including unwinding); callers must still be `#[serial]`.

#![allow(
    dead_code,
    reason = "included by several test binaries, each using a subset"
)]
#![allow(
    unsafe_code,
    reason = "edition 2024 makes env::set_var/remove_var unsafe; mutating the process env is this guard's purpose"
)]

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
        // SAFETY: every test that builds an `EnvGuard` is serialized (`#[serial]`, or
        // the binary's own env lock in `stale_socket_tests`), so no other test thread
        // reads or writes the environment while this runs.
        unsafe { std::env::set_var(key, value) };
    }

    /// Remove `key`, snapshotting its original value the first time it is touched.
    pub fn remove(&mut self, key: &'static str) {
        self.snapshot(key);
        // SAFETY: as in `set`, env-mutating tests are serialized, so no other
        // thread touches the environment concurrently.
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
            match original {
                // SAFETY: the guard is dropped inside the same serialized test that
                // created it, so no other thread touches the environment.
                Some(value) => unsafe { std::env::set_var(key, value) },
                // SAFETY: as above, the owning serialized test is the only env user.
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}
