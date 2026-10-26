//! Polling helpers that replace fixed sleeps with a bounded wait on a condition.

#![allow(
    dead_code,
    reason = "included by several test binaries, each using a subset"
)]

use std::time::{Duration, Instant};

/// Poll `predicate` every `poll_interval` until it returns true or `timeout`
/// elapses. Returns the final predicate result, so a condition that becomes
/// true on the last poll still counts.
#[must_use]
pub fn wait_until(
    timeout: Duration,
    poll_interval: Duration,
    mut predicate: impl FnMut() -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        std::thread::sleep(poll_interval);
    }
    predicate()
}

/// Async form of [`wait_until`]: sleeps on the tokio timer between polls so
/// tasks on the same runtime (for example an in-process server) keep running.
#[must_use]
pub async fn wait_until_async(
    timeout: Duration,
    poll_interval: Duration,
    mut predicate: impl FnMut() -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        tokio::time::sleep(poll_interval).await;
    }
    predicate()
}
