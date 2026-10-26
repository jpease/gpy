//! Agent startup: diagnostics, timers, and shutdown signal handlers.

use crate::debug_log;
use crate::{Error, Result};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::signal;

/// Boxed, pinned async shutdown signal future.
pub(super) type ShutdownSignal = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Logs startup diagnostics. The real bind (and its errors) happens in
/// [`crate::ipc::server::EndpointHandle::start`]; a pre-check would race.
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

/// Creates the 60 s pruning and shell re-nudge timer.
/// First tick is at +60 s: the restart-nudge task already covers t=0.
pub(super) fn create_pruning_timer() -> tokio::time::Interval {
    let mut pruning_timer = tokio::time::interval(Duration::from_secs(60));
    pruning_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    pruning_timer.reset();
    pruning_timer
}

/// Creates a shutdown future resolving on SIGINT or SIGTERM (what `kill` and
/// session managers send), so the watcher and socket get cleaned up.
#[cfg(unix)]
pub(super) fn create_shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        let mut sigterm =
            signal::unix::signal(signal::unix::SignalKind::terminate()).map_err(|e| {
                Error::process(
                    "shutdown signal",
                    format!("Failed to setup SIGTERM handler: {e}"),
                )
            })?;

        tokio::select! {
            result = signal::ctrl_c() => {
                result.map_err(|e| {
                    Error::process("shutdown signal",
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

/// Creates a shutdown future resolving on ctrl-c (no SIGTERM off Unix).
#[cfg(not(unix))]
pub(super) fn create_shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        signal::ctrl_c().await.map_err(|e| {
            Error::process(
                "shutdown signal",
                format!("Failed to setup signal handler: {e}"),
            )
        })?;
        println!("Received shutdown signal, shutting down gracefully...");
        Ok::<(), Error>(())
    })
}

/// Whether to send a clock broadcast this tick, and the new last-notified minute.
/// With `show_seconds` off, sends only when `current_minute` changed.
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

    /// The restart nudge must be the only nudge at t=0.
    #[tokio::test]
    async fn pruning_timer_first_tick_is_not_immediate() {
        let mut timer = create_pruning_timer();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), timer.tick())
                .await
                .is_err()
        );
    }

    /// Safe to self-raise: `create_shutdown_signal` installs its own handler first.
    #[tokio::test]
    #[serial(sigterm_self_signal)]
    async fn create_shutdown_signal_resolves_on_sigterm() {
        let handle = tokio::spawn(create_shutdown_signal());

        // Let the task install the SIGTERM handler before raising.
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

    /// SIGINT must keep working alongside SIGTERM.
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
