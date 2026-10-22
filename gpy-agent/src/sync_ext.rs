//! Lock acquisition that recovers a poisoned lock instead of panicking.
//!
//! Release builds abort on panic (ADR-0008), so a lock can only be poisoned in
//! tests and benchmarks, which unwind. Recovering keeps a panicking test from
//! cascading into every later test that shares the lock, and takes the lock
//! without `unwrap()`/`expect()`, which the crate's lint policy denies.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// [`Mutex::lock`] that returns the guard even when the mutex is poisoned.
pub(crate) trait MutexExt<T: ?Sized> {
    /// Locks the mutex, recovering the guard from a poisoned lock.
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`RwLock::read`] and [`RwLock::write`] that return the guard even when the
/// lock is poisoned.
pub(crate) trait RwLockExt<T: ?Sized> {
    /// Takes a shared read guard, recovering it from a poisoned lock.
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T>;
    /// Takes an exclusive write guard, recovering it from a poisoned lock.
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}
