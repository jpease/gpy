#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use super::{MAX_COALESCED_FOLLOW_UPS, RepoRefreshCoordinator, run_coalesced};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

/// Second same-repo caller is refused and records a pending follow-up.
#[test]
fn try_start_grants_first_caller_and_records_pending() {
    let coordinator = RepoRefreshCoordinator::default();
    let repo = PathBuf::from("/repo/state");

    assert!(coordinator.try_start(&repo), "first caller runs");
    assert!(
        !coordinator.try_start(&repo),
        "second concurrent caller for the same repo is refused"
    );
    // Pending follow-up: one more pass, then done.
    assert!(coordinator.finish(&repo), "pending follow-up owed");
    assert!(!coordinator.finish(&repo), "no further work; entry removed");
    assert!(coordinator.try_start(&repo));
    coordinator.abandon(&repo);
}

/// In-flight refresh for repo A never refuses repo B.
#[test]
fn different_repos_never_serialize_in_state_machine() {
    let coordinator = RepoRefreshCoordinator::default();
    let repo_a = PathBuf::from("/repo/a");
    let repo_b = PathBuf::from("/repo/b");

    assert!(coordinator.try_start(&repo_a));
    assert!(
        coordinator.try_start(&repo_b),
        "a different repo must never be serialized behind an in-flight repo"
    );
    assert!(!coordinator.finish(&repo_a));
    assert!(!coordinator.finish(&repo_b));
}

/// Concurrent mid-scan events coalesce into exactly one follow-up.
#[test]
fn run_coalesced_coalesces_concurrent_events_into_single_follow_up() {
    let coordinator = RepoRefreshCoordinator::default();
    let repo = PathBuf::from("/repo/single");
    let mut runs = 0_u32;

    run_coalesced(&coordinator, &repo, |_is_follow_up| {
        runs += 1;
        if runs == 1 {
            // Two concurrent events during the initial pass.
            assert!(!coordinator.try_start(&repo));
            assert!(!coordinator.try_start(&repo));
        }
    });

    assert_eq!(
        runs, 2,
        "one initial pass plus exactly one coalesced follow-up"
    );
    assert!(coordinator.try_start(&repo));
    coordinator.abandon(&repo);
}

/// An older, slower scan never leaves its stale result in the cache.
#[test]
fn run_coalesced_stale_scan_cannot_overwrite_fresher() {
    let coordinator = Arc::new(RepoRefreshCoordinator::default());
    let repo = PathBuf::from("/repo/interleave");
    let disk = Arc::new(AtomicUsize::new(1));
    let cache = Arc::new(AtomicUsize::new(0));

    let (pass1_reading_tx, pass1_reading_rx) = mpsc::channel::<()>();
    let (release_pass1_tx, release_pass1_rx) = mpsc::channel::<()>();

    let owner = {
        let thread_coordinator = Arc::clone(&coordinator);
        let thread_repo = repo.clone();
        let thread_disk = Arc::clone(&disk);
        let thread_cache = Arc::clone(&cache);
        thread::spawn(move || {
            let mut pass = 0_u32;
            run_coalesced(&thread_coordinator, &thread_repo, |_is_follow_up| {
                let value = thread_disk.load(Ordering::Acquire);
                thread_cache.store(value, Ordering::Release);
                pass += 1;
                if pass == 1 {
                    // Hold pass 1 (old value) while the test advances disk and injects an event.
                    pass1_reading_tx.send(()).expect("announce pass 1");
                    release_pass1_rx.recv().expect("await pass 1 release");
                }
            });
        })
    };

    pass1_reading_rx.recv().expect("pass 1 should start");
    assert_eq!(
        cache.load(Ordering::Acquire),
        1,
        "pass 1 wrote the older (stale) value"
    );

    // Concurrent same-repo event: must be coalesced, not run in parallel.
    disk.store(2, Ordering::Release);
    assert!(
        !coordinator.try_start(&repo),
        "a concurrent same-repo refresh must be coalesced, not run in parallel"
    );

    // The follow-up re-reads the newest disk state.
    release_pass1_tx.send(()).expect("release pass 1");
    owner.join().expect("owner thread panicked");

    assert_eq!(
        cache.load(Ordering::Acquire),
        2,
        "the fresher state must win; the stale pass 1 result must not survive"
    );
}

/// Repo B completes while repo A's scan is blocked.
#[test]
fn run_coalesced_different_repos_run_concurrently() {
    let coordinator = Arc::new(RepoRefreshCoordinator::default());
    let repo_a = PathBuf::from("/repo/a");
    let repo_b = PathBuf::from("/repo/b");

    let (a_started_tx, a_started_rx) = mpsc::channel::<()>();
    let (release_a_tx, release_a_rx) = mpsc::channel::<()>();

    let handle_a = {
        let thread_coordinator = Arc::clone(&coordinator);
        let thread_repo_a = repo_a;
        thread::spawn(move || {
            run_coalesced(&thread_coordinator, &thread_repo_a, |_is_follow_up| {
                a_started_tx.send(()).expect("announce repo A scan");
                release_a_rx.recv().expect("await repo A release");
            });
        })
    };

    a_started_rx.recv().expect("repo A scan should start");

    let b_ran = AtomicBool::new(false);
    run_coalesced(&coordinator, &repo_b, |_is_follow_up| {
        b_ran.store(true, Ordering::Release);
    });
    assert!(
        b_ran.load(Ordering::Acquire),
        "repo B must run to completion while repo A's scan is still blocked"
    );

    release_a_tx.send(()).expect("release repo A");
    handle_a.join().expect("repo A thread panicked");
}

/// The follow-up cap ends sustained churn and frees the repo.
#[test]
fn run_coalesced_follow_up_cap_prevents_unbounded_loop() {
    let coordinator = RepoRefreshCoordinator::default();
    let repo = PathBuf::from("/repo/churn");
    let mut runs = 0_u32;

    run_coalesced(&coordinator, &repo, |_is_follow_up| {
        runs += 1;
        // Every scan re-arms pending.
        coordinator.try_start(&repo);
    });

    assert_eq!(
        runs,
        MAX_COALESCED_FOLLOW_UPS + 2,
        "initial pass plus exactly MAX_COALESCED_FOLLOW_UPS follow-ups, plus the \
         final cap-triggered scan that captures the last pending state before \
         abandoning"
    );
    assert!(
        coordinator.try_start(&repo),
        "repo must be runnable again after the cap-triggered abandon"
    );
    coordinator.abandon(&repo);
}

/// A burst that trips the cap and then settles still captures the final state.
#[test]
fn run_coalesced_cap_scan_captures_settled_state_after_burst() {
    let coordinator = RepoRefreshCoordinator::default();
    let repo = PathBuf::from("/repo/settles-after-cap");

    let disk = AtomicUsize::new(0);
    let captured = AtomicUsize::new(0);
    let mut calls = 0_usize;

    // Same burst size that trips the cap in the test above.
    let churn_passes = usize::try_from(MAX_COALESCED_FOLLOW_UPS + 1).expect("fits usize");

    run_coalesced(&coordinator, &repo, |_is_follow_up| {
        calls += 1;
        captured.store(disk.load(Ordering::SeqCst), Ordering::SeqCst);

        if calls <= churn_passes {
            // Change lands after this scan read disk; churn stops after `churn_passes`.
            disk.store(calls, Ordering::SeqCst);
            coordinator.try_start(&repo);
        }
    });

    assert_eq!(
        calls,
        churn_passes + 1,
        "the final cap-triggered scan must run to observe the settled state"
    );
    assert_eq!(
        captured.load(Ordering::SeqCst),
        churn_passes,
        "the final cap scan must observe the settled disk state written by the \
         last churn pass, proving it is not dropped when the cap is hit"
    );
}
