//! Clock timer: SIGURG doorbell on minute boundaries (or every second with
//! `show_seconds=true`). `test_clock_timer_ticks` takes ~2s of wall clock.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

#[path = "common/sigurg.rs"]
mod sigurg;
#[path = "common/wait.rs"]
mod wait;

use gpy_agent::agent::startup::clock_signal_decision;
#[cfg(unix)]
use gpy_agent::ipc::ClientDirectory;
#[cfg(unix)]
use serial_test::serial;
#[cfg(unix)]
use sigurg::SigurgCounter;
#[cfg(unix)]
use std::sync::Arc;
use std::time::Duration;
#[cfg(unix)]
use wait::wait_until_async;

#[cfg(unix)]
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Sends only when the minute differs from the last-notified one.
#[test]
fn test_minute_boundary_detection() {
    let minute = 28_333_333_u64; // arbitrary current minute

    let (send, new_min) = clock_signal_decision(false, minute, minute - 1);
    assert!(send, "Should send when minute advanced");
    assert_eq!(new_min, minute);

    let (resend, unchanged_min) = clock_signal_decision(false, minute, minute);
    assert!(!resend, "Should not resend same minute");
    assert_eq!(
        unchanged_min, minute,
        "last_notified_minute must be unchanged"
    );

    let (seconds_send, _) = clock_signal_decision(true, minute, minute);
    assert!(seconds_send, "show_seconds=true always sends");
}

/// A 1-second interval never ticks early: two ticks after the immediate first
/// one take at least 2 s. No upper bound, since a late tick only measures load.
#[tokio::test]
async fn test_clock_timer_ticks() {
    let start = tokio::time::Instant::now();
    let mut clock_timer = tokio::time::interval(Duration::from_secs(1));
    clock_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // First tick is immediate.
    clock_timer.tick().await;
    clock_timer.tick().await;
    clock_timer.tick().await;

    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_secs(2),
        "Timer must not tick before each 1 s period elapses (elapsed: {elapsed:?})"
    );
}

/// `notify_repaint()` rings SIGURG on registered PIDs.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_clock_signals_sent_to_clients() {
    let doorbell = SigurgCounter::install();

    let registry = Arc::new(ClientDirectory::new());
    let pid = std::process::id();
    let cwd = std::env::current_dir().expect("current dir");
    registry.register(pid, Some(cwd.clone()));

    registry.notify_repaint(None);

    // Up to 1s; should be near-instant.
    let received = wait_until_async(Duration::from_secs(1), POLL_INTERVAL, || {
        doorbell.count() >= 1
    })
    .await;
    assert!(received, "Should have received SIGURG within timeout");
    assert_eq!(
        doorbell.count(),
        1,
        "Should have received exactly one SIGURG"
    );

    registry.notify_repaint(None);
    let received_second = wait_until_async(Duration::from_secs(1), POLL_INTERVAL, || {
        doorbell.count() >= 2
    })
    .await;
    assert!(
        received_second,
        "Should have received second SIGURG within timeout"
    );
    assert_eq!(
        doorbell.count(),
        2,
        "Should have received two SIGURG signals"
    );
}

/// Unregistered clients get no signal.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_unregistered_clients_dont_receive_signals() {
    let doorbell = SigurgCounter::install();

    let registry = Arc::new(ClientDirectory::new());

    registry.notify_repaint(None);

    // Expect a timeout: no signal should arrive.
    let received = wait_until_async(Duration::from_millis(100), POLL_INTERVAL, || {
        doorbell.count() >= 1
    })
    .await;
    assert!(!received, "Should not have received any signal");
    assert_eq!(
        doorbell.count(),
        0,
        "Unregistered clients should not receive signals"
    );
}

/// Minute comparison in `clock_signal_decision`, without the wall clock.
#[test]
fn test_minute_boundary_logic_with_fixed_timestamps() {
    type Case = (bool, u64, u64, bool);

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

/// Minute rounding over fixed epoch timestamps.
#[test]
fn test_minute_boundary_detection_with_epochs() {
    let minute_boundary_epochs = [
        0_u64,         // 1970-01-01 00:00:00
        60,            // 1970-01-01 00:01:00
        3600,          // 1970-01-01 01:00:00
        1_700_000_000, // 2023-11-14 22:13:20 -> rounded down to 1_699_999_980 (:00)
    ];

    for epoch in minute_boundary_epochs {
        let at_boundary = (epoch / 60) * 60;
        let seconds_into_minute = at_boundary % 60;
        assert_eq!(
            seconds_into_minute, 0,
            "Epoch {at_boundary} should be at minute boundary"
        );

        let mid_minute = at_boundary + 30;
        let mid_seconds = mid_minute % 60;
        assert_eq!(
            mid_seconds, 30,
            "Epoch {mid_minute} should be at :30 seconds"
        );
    }
}
