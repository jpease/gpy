//! Clock timer: SIGURG doorbell on minute boundaries (or every second with
//! `show_seconds=true`). `test_clock_timer_ticks` takes ~2s of wall clock.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

mod test_harness;

use gpy_agent::agent::startup::clock_signal_decision;
use gpy_agent::ipc::ClientDirectory;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use test_harness::SignalCounter;

#[cfg(unix)]
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

// OnceLock so the counters are safely reachable from signal handlers.
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

/// Sends only when the minute differs from the last-notified one.
#[test]
fn test_minute_boundary_detection() {
    let minute = 28_333_333_u64; // arbitrary current minute

    let (send, new_min) = clock_signal_decision(false, minute, minute - 1);
    assert!(send, "Should send when minute advanced");
    assert_eq!(new_min, minute);

    let (send, new_min) = clock_signal_decision(false, minute, minute);
    assert!(!send, "Should not resend same minute");
    assert_eq!(new_min, minute, "last_notified_minute must be unchanged");

    let (send, _) = clock_signal_decision(true, minute, minute);
    assert!(send, "show_seconds=true always sends");
}

/// A 1-second interval ticks at ~1s against the wall clock.
#[tokio::test]
async fn test_clock_timer_ticks() {
    let mut clock_timer = tokio::time::interval(Duration::from_secs(1));
    clock_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // First tick is immediate.
    clock_timer.tick().await;

    let start = tokio::time::Instant::now();

    clock_timer.tick().await;
    clock_timer.tick().await;

    let elapsed = start.elapsed();

    // ~2 seconds, with tolerance.
    assert!(
        elapsed >= Duration::from_millis(1950) && elapsed < Duration::from_millis(2500),
        "Timer should tick approximately every second (elapsed: {elapsed:?})"
    );
}

/// `notify_repaint()` rings SIGURG on registered PIDs.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_clock_signals_sent_to_clients() {
    let counter = get_registered_counter();
    counter.reset();

    let handler = SigAction::new(
        SigHandler::Handler(count_doorbell),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install handler");

    let registry = Arc::new(ClientDirectory::new());
    let pid = std::process::id();
    let cwd = std::env::current_dir().expect("current dir");
    registry.register(pid, Some(cwd.clone()));

    registry.notify_repaint(None);

    // Up to 1s; should be near-instant.
    let received = counter.wait_for_count(1, Duration::from_secs(1)).await;
    assert!(received, "Should have received SIGURG within timeout");
    assert_eq!(counter.get(), 1, "Should have received exactly one SIGURG");

    registry.notify_repaint(None);
    let received = counter.wait_for_count(2, Duration::from_secs(1)).await;
    assert!(
        received,
        "Should have received second SIGURG within timeout"
    );
    assert_eq!(counter.get(), 2, "Should have received two SIGURG signals");

    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore handler");
    }
}

/// Unregistered clients get no signal.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_unregistered_clients_dont_receive_signals() {
    let counter = get_unregistered_counter();
    counter.reset();

    let handler = SigAction::new(
        SigHandler::Handler(count_unregistered),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install handler");

    let registry = Arc::new(ClientDirectory::new());

    registry.notify_repaint(None);

    // Expect a timeout: no signal should arrive.
    let received = counter.wait_for_count(1, Duration::from_millis(100)).await;
    assert!(!received, "Should not have received any signal");
    assert_eq!(
        counter.get(),
        0,
        "Unregistered clients should not receive signals"
    );

    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore handler");
    }
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
        let seconds_into_minute = mid_minute % 60;
        assert_eq!(
            seconds_into_minute, 30,
            "Epoch {mid_minute} should be at :30 seconds"
        );
    }
}
