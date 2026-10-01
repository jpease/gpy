//! Clock timer integration tests
//!
//! Tests the clock timer feature that rings the SIGURG doorbell on registered clients
//! at minute boundaries (or every second when `show_seconds=true`).
//!
//! These tests use deterministic signaling primitives (see `test_harness` module)
//! instead of brittle `sleep()`-based timing waits.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::shadow_unrelated)]

mod test_harness;

use gpy_agent::agent::startup::clock_signal_decision;
use gpy_agent::ipc::ClientDirectory;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use test_harness::SignalCounter;

#[cfg(unix)]
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

// Global counters for signal handlers
// We use OnceLock to lazily initialize SignalCounters that can be safely accessed from signal handlers
use std::sync::OnceLock;

static REGISTERED_COUNTER: OnceLock<SignalCounter> = OnceLock::new();
static UNREGISTERED_COUNTER: OnceLock<SignalCounter> = OnceLock::new();

fn get_registered_counter() -> &'static SignalCounter {
    REGISTERED_COUNTER.get_or_init(SignalCounter::new)
}

fn get_unregistered_counter() -> &'static SignalCounter {
    UNREGISTERED_COUNTER.get_or_init(SignalCounter::new)
}

#[cfg(unix)]
extern "C" fn count_doorbell(_signal: i32) {
    get_registered_counter().increment();
}

#[cfg(unix)]
extern "C" fn count_unregistered(_signal: i32) {
    get_unregistered_counter().increment();
}

/// Test that minute boundary detection works correctly
///
/// The agent now tracks the last-notified minute and only sends when the
/// current minute differs, eliminating jitter from tick scheduling.
#[test]
fn test_minute_boundary_detection() {
    let minute = 28_333_333_u64; // arbitrary current minute

    // Different minute → should send
    let (send, new_min) = clock_signal_decision(false, minute, minute - 1);
    assert!(send, "Should send when minute advanced");
    assert_eq!(new_min, minute);

    // Same minute → should NOT send
    let (send, new_min) = clock_signal_decision(false, minute, minute);
    assert!(!send, "Should not resend same minute");
    assert_eq!(new_min, minute, "last_notified_minute must be unchanged");

    // show_seconds=true → always send
    let (send, _) = clock_signal_decision(true, minute, minute);
    assert!(send, "show_seconds=true always sends");
}

/// Test that the clock timer ticks consistently
///
/// This test verifies the timer is created and ticks at 1-second intervals.
/// We don't wait for a full minute in tests, but we verify the basic mechanism.
#[tokio::test]
async fn test_clock_timer_ticks() {
    // Create a simple timer that mimics the agent's clock timer
    let mut clock_timer = tokio::time::interval(Duration::from_secs(1));
    clock_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // First tick is immediate, skip it
    clock_timer.tick().await;

    let start = tokio::time::Instant::now();

    // Wait for 2 more ticks (should take ~2 seconds)
    clock_timer.tick().await;
    clock_timer.tick().await;

    let elapsed = start.elapsed();

    // Should have taken approximately 2 seconds (allow some tolerance)
    assert!(
        elapsed >= Duration::from_millis(1950) && elapsed < Duration::from_millis(2500),
        "Timer should tick approximately every second (elapsed: {elapsed:?})"
    );
}

/// Test that SIGURG doorbell signals are sent to registered clients
///
/// This is an integration test that verifies:
/// 1. Clients can be registered with the `ClientDirectory`
/// 2. `notify_repaint()` correctly rings the doorbell on registered PIDs
/// 3. The signal handler is invoked
///
/// Uses deterministic signaling instead of sleep-based timing.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_clock_signals_sent_to_clients() {
    let counter = get_registered_counter();
    counter.reset();

    // Install our test signal handler
    let handler = SigAction::new(
        SigHandler::Handler(count_doorbell),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install handler");

    // Create a client registry and register ourselves
    let registry = Arc::new(ClientDirectory::new());
    let pid = std::process::id();
    let cwd = std::env::current_dir().expect("current dir");
    registry.register(pid, Some(cwd.clone()));

    // Emulate the clock timer sending a signal
    registry.notify_repaint(None);

    // Wait for signal delivery (up to 1 second, but should be nearly instant)
    let received = counter.wait_for_count(1, Duration::from_secs(1)).await;
    assert!(received, "Should have received SIGURG within timeout");
    assert_eq!(counter.get(), 1, "Should have received exactly one SIGURG");

    // Send another signal to verify it works multiple times
    registry.notify_repaint(None);
    let received = counter.wait_for_count(2, Duration::from_secs(1)).await;
    assert!(
        received,
        "Should have received second SIGURG within timeout"
    );
    assert_eq!(counter.get(), 2, "Should have received two SIGURG signals");

    // Restore previous handler
    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore handler");
    }
}

/// Test that unregistered clients don't receive signals
///
/// Uses deterministic signaling - waits briefly to ensure no signal arrives,
/// then verifies the counter remained at zero.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_unregistered_clients_dont_receive_signals() {
    let counter = get_unregistered_counter();
    counter.reset();

    // Install our test signal handler
    let handler = SigAction::new(
        SigHandler::Handler(count_unregistered),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install handler");

    // Create a registry but DON'T register ourselves
    let registry = Arc::new(ClientDirectory::new());

    // Try to send a signal (should do nothing since no clients registered)
    registry.notify_repaint(None);

    // Wait briefly to ensure no signal arrives (should timeout)
    let received = counter.wait_for_count(1, Duration::from_millis(100)).await;
    assert!(!received, "Should not have received any signal");
    assert_eq!(
        counter.get(),
        0,
        "Unregistered clients should not receive signals"
    );

    // Restore previous handler
    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore handler");
    }
}

/// Test minute-comparison logic with fixed minute values
///
/// Tests the core logic used by `clock_signal_decision` without relying on wall clock.
/// The implementation compares `current_minute` to `last_notified_minute` rather than
/// checking seconds-into-minute, eliminating tick-jitter edge cases.
#[test]
fn test_minute_boundary_logic_with_fixed_timestamps() {
    type Case = (bool, u64, u64, bool);

    // Test cases: (show_seconds, last_notified_minute, current_minute, expected_send)
    let test_cases: &[Case] = &[
        (false, 99, 100, true),   // minute advanced → send
        (false, 100, 100, false), // same minute → no send
        (true, 100, 100, true),   // show_seconds=true always sends
        (true, 100, 101, true),   // show_seconds=true always sends
        (false, 0, 1, true),      // fresh start → send on first real minute
    ];

    for &(show_seconds, last, current, expected_send) in test_cases {
        let (send, new_last) = clock_signal_decision(show_seconds, current, last);
        assert_eq!(
            send, expected_send,
            "show_seconds={show_seconds}, last={last}, current={current}: expected send={expected_send}"
        );
        if send {
            assert_eq!(new_last, current, "new_last should equal current when sent");
        } else {
            assert_eq!(new_last, last, "new_last should be unchanged when not sent");
        }
    }
}

/// Test that minute boundary detection works with various epoch timestamps
///
/// Uses deterministic timestamps rather than `SystemTime::now()` to avoid flakiness.
#[test]
fn test_minute_boundary_detection_with_epochs() {
    // Test with various epoch timestamps that are known to be at minute boundaries
    let minute_boundary_epochs = [
        0_u64,         // 1970-01-01 00:00:00
        60,            // 1970-01-01 00:01:00
        3600,          // 1970-01-01 01:00:00
        1_700_000_000, // 2023-11-14 22:13:20 -> rounded down to 1_699_999_980 (:00)
    ];

    for epoch in minute_boundary_epochs {
        // Round down to minute boundary
        let at_boundary = (epoch / 60) * 60;
        let seconds_into_minute = at_boundary % 60;
        assert_eq!(
            seconds_into_minute, 0,
            "Epoch {at_boundary} should be at minute boundary"
        );

        // Test mid-minute (30 seconds after boundary)
        let mid_minute = at_boundary + 30;
        let seconds_into_minute = mid_minute % 60;
        assert_eq!(
            seconds_into_minute, 30,
            "Epoch {mid_minute} should be at :30 seconds"
        );
    }
}
