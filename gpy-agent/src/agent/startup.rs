//! Agent startup initialization and validation
//!
//! This module contains all startup-related logic including:
//! - Startup diagnostics logging
//! - Clock timer creation
//! - Shutdown signal handlers

use crate::debug_log;
use crate::{Error, Result};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::signal;

/// Type alias for a boxed, pinned async shutdown signal future
pub(super) type ShutdownSignal = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Logs startup diagnostics.
///
/// The real socket bind happens later in [`crate::ipc::server::EndpointHandle::start`];
/// an unbindable socket surfaces there and propagates out of the event loop, so
/// there is no separate pre-bind check to perform here (a pre-check would only be
/// a redundant second bind and a TOCTOU race on the socket the shell readiness
/// ping waits on).
pub(super) fn startup_checks(socket_path: &str) {
    debug_log!("agent", "GPY Agent starting...");
    debug_log!("agent", "Socket path: {}", socket_path);
    debug_log!("agent", "Starting IPC server...");
}

/// Creates a clock timer that ticks every second.
pub(super) fn create_clock_timer() -> tokio::time::Interval {
    let mut clock_timer = tokio::time::interval(Duration::from_secs(1));
    clock_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    clock_timer
}

/// Creates a shutdown signal handler for graceful shutdown.
///
/// Listens for both SIGINT (ctrl-c) and SIGTERM, since SIGTERM is what
/// system shutdown, `kill <pid>`, and session managers send by default.
/// Without it, the agent hits the default disposition (instant death) and
/// never gets a chance to stop the watcher or clean up the socket file.
#[cfg(unix)]
pub(super) fn create_shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        let mut sigterm =
            signal::unix::signal(signal::unix::SignalKind::terminate()).map_err(|e| {
                Error::process(
                    "shutdown signal".to_owned(),
                    format!("Failed to setup SIGTERM handler: {e}"),
                )
            })?;

        tokio::select! {
            result = signal::ctrl_c() => {
                result.map_err(|e| {
                    Error::process(
                        "shutdown signal".to_owned(),
                        format!("Failed to setup signal handler: {e}"),
                    )
                })?;
            }
            _ = sigterm.recv() => {}
        }

        println!("Received shutdown signal, shutting down gracefully...");
        Ok::<(), Error>(())
    })
}

/// Creates a shutdown signal handler for graceful shutdown (non-Unix stub).
///
/// SIGTERM has no equivalent on this platform, so only ctrl-c is handled.
#[cfg(not(unix))]
pub(super) fn create_shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        signal::ctrl_c().await.map_err(|e| {
            Error::process(
                "shutdown signal".to_owned(),
                format!("Failed to setup signal handler: {e}"),
            )
        })?;
        println!("Received shutdown signal, shutting down gracefully...");
        Ok::<(), Error>(())
    })
}

/// Whether the current wall-clock minute warrants a fresh SIGUSR1 clock
/// broadcast, and the minute to remember as "last notified" afterward.
///
/// Pure: the caller supplies `current_minute` (`now_secs / 60`) rather than
/// this function reading the wall clock itself, so it's directly
/// unit-testable with fixed minute values.
///
/// - `show_seconds=true`: send on every call (every tick).
/// - `show_seconds=false`: send when `current_minute` differs from
///   `last_notified_minute`, avoiding duplicate broadcasts if the timer fires
///   twice within the same minute.
///
/// Returns `(should_send, new_last_notified_minute)`.
#[must_use]
pub const fn clock_signal_decision(
    show_seconds: bool,
    current_minute: u64,
    last_notified_minute: u64,
) -> (bool, u64) {
    if show_seconds {
        (true, current_minute)
    } else {
        let should_send = current_minute != last_notified_minute;
        (
            should_send,
            if should_send {
                current_minute
            } else {
                last_notified_minute
            },
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use nix::sys::signal::{Signal, raise};
    use serial_test::serial;

    /// SIGTERM must resolve the shutdown future instead of hitting the
    /// default disposition (instant process death). Self-raises SIGTERM
    /// against the test process itself, which is safe only because
    /// `create_shutdown_signal` installs its own handler (overriding the
    /// default) as soon as it starts running.
    #[tokio::test]
    #[serial(sigterm_self_signal)]
    async fn create_shutdown_signal_resolves_on_sigterm() {
        let handle = tokio::spawn(create_shutdown_signal());

        // Give the spawned task a chance to register the SIGTERM handler
        // before raising it; `signal::unix::signal` only overrides the OS
        // disposition once the async block starts executing.
        tokio::time::sleep(Duration::from_millis(50)).await;

        raise(Signal::SIGTERM).expect("raise SIGTERM");

        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown signal task should complete after SIGTERM")
            .expect("shutdown signal task should not panic");
        assert!(
            result.is_ok(),
            "shutdown signal future should resolve Ok on SIGTERM"
        );
    }

    /// SIGINT (ctrl-c) must keep working alongside the new SIGTERM branch.
    #[tokio::test]
    #[serial(sigterm_self_signal)]
    async fn create_shutdown_signal_resolves_on_sigint() {
        let handle = tokio::spawn(create_shutdown_signal());

        tokio::time::sleep(Duration::from_millis(50)).await;

        raise(Signal::SIGINT).expect("raise SIGINT");

        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown signal task should complete after SIGINT")
            .expect("shutdown signal task should not panic");
        assert!(
            result.is_ok(),
            "shutdown signal future should resolve Ok on SIGINT"
        );
    }
}
