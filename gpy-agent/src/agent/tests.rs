mod language_tests {
    #![allow(clippy::missing_panics_doc)]

    use crate::config::LanguageSettings;
    use crate::language::detector::DetectedLanguage;
    use crate::language::display::build_language_display_info;
    use crate::theme::ThemeConfig;

    fn sample_detected_languages() -> Vec<DetectedLanguage> {
        vec![
            DetectedLanguage {
                name: "alpha".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 42,
            },
            DetectedLanguage {
                name: "beta".to_owned(),
                confidence: 0.8,
                file_count: 2,
                total_bytes: 256,
            },
        ]
    }

    #[test]
    fn build_language_display_retains_languages_when_versions_hidden() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: false,
            ..LanguageSettings::default()
        };

        let info = build_language_display_info(&detected, &theme, &language_cfg);

        assert_eq!(info.len(), detected.len());
    }

    #[test]
    fn build_language_display_filters_enabled_languages() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: true,
            enabled_languages: vec!["alpha".to_owned()],
            ..LanguageSettings::default()
        };

        let info = build_language_display_info(&detected, &theme, &language_cfg);
        let names: Vec<&str> = info.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(names, vec!["alpha"]);
    }

    #[test]
    fn build_language_display_disabled_returns_empty() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            enabled: false,
            ..LanguageSettings::default()
        };

        let info = build_language_display_info(&detected, &theme, &language_cfg);
        assert!(info.is_empty());
    }
}

mod reconcile_interval_tests {
    #![allow(clippy::missing_panics_doc)]

    use super::super::parse_reconcile_interval_secs;

    #[test]
    fn parse_reconcile_interval_secs_rejects_unset_zero_and_invalid() {
        assert_eq!(parse_reconcile_interval_secs(None), None);
        assert_eq!(parse_reconcile_interval_secs(Some("0")), None);
        assert_eq!(parse_reconcile_interval_secs(Some("not-a-number")), None);
    }

    #[test]
    fn parse_reconcile_interval_secs_accepts_positive_integer() {
        assert_eq!(parse_reconcile_interval_secs(Some("5")), Some(5_u64));
    }
}

/// Reconcile ticks skipped during an in-flight scan are caught up afterwards, never concurrently.
mod reconcile_scan_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::super::{PathBuf, run_reconcile_scan};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    fn fake_repo() -> Vec<PathBuf> {
        vec![PathBuf::from("/fake/repo")]
    }

    /// `mpsc::Receiver` is not `Sync`, so share it behind a `Mutex`.
    type SharedRx<T> = Arc<Mutex<mpsc::Receiver<T>>>;

    /// Fake scan that signals `start_tx` and blocks on `release_rx`.
    fn gated_scan_fn(
        call_count: Arc<AtomicUsize>,
        start_tx: mpsc::Sender<usize>,
        release_rx: SharedRx<()>,
    ) -> impl Fn(Vec<PathBuf>) + Send + Clone + 'static {
        move |_repos: Vec<PathBuf>| {
            let call_number = call_count.fetch_add(1, Ordering::AcqRel).saturating_add(1);
            let _ = start_tx.send(call_number);
            let _ = release_rx.lock().expect("release_rx mutex poisoned").recv();
        }
    }

    /// Waits off the runtime thread for the next scan start; returns the 1-based call number.
    async fn await_scan_started(start_rx: &SharedRx<usize>) -> usize {
        let owned_start_rx = Arc::clone(start_rx);
        tokio::task::spawn_blocking(move || {
            owned_start_rx
                .lock()
                .expect("start_rx mutex poisoned")
                .recv()
                .expect("scan_fn never started")
        })
        .await
        .expect("blocking wait task panicked")
    }

    /// No skipped tick: exactly one pass, no catch-up.
    #[tokio::test]
    async fn no_pending_tick_runs_exactly_one_pass() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let (start_tx, start_rx_raw) = mpsc::channel::<usize>();
        let shared_start_rx = Arc::new(Mutex::new(start_rx_raw));
        let (release_tx, release_rx_raw) = mpsc::channel::<()>();
        let shared_release_rx = Arc::new(Mutex::new(release_rx_raw));

        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));
        let roots_fn_calls = Arc::new(AtomicUsize::new(0));
        let roots_fn_calls_for_closure = Arc::clone(&roots_fn_calls);
        let roots_fn = move || {
            roots_fn_calls_for_closure.fetch_add(1, Ordering::AcqRel);
            fake_repo()
        };
        let scan_fn = gated_scan_fn(Arc::clone(&call_count), start_tx, shared_release_rx);

        let handle = tokio::spawn(run_reconcile_scan(
            fake_repo(),
            Arc::clone(&in_flight),
            Arc::clone(&pending),
            roots_fn,
            scan_fn,
        ));

        let first_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(first_call_number, 1);
        release_tx.send(()).expect("failed to release scan pass 1");

        handle.await.expect("run_reconcile_scan task panicked");

        assert_eq!(
            call_count.load(Ordering::Acquire),
            1,
            "no tick was skipped, so exactly one scan pass should run"
        );
        assert_eq!(
            roots_fn_calls.load(Ordering::Acquire),
            0,
            "roots_fn (only used for the catch-up pass) must not run when nothing is pending"
        );
        assert!(!pending.load(Ordering::Acquire));
        assert!(
            !in_flight.load(Ordering::Acquire),
            "in_flight must be released"
        );
    }

    /// A tick during the first scan triggers exactly one catch-up pass; a tick during
    /// the catch-up stays pending rather than causing a third pass.
    #[tokio::test]
    async fn skipped_tick_triggers_exactly_one_bounded_catch_up_pass() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let (start_tx, start_rx_raw) = mpsc::channel::<usize>();
        let shared_start_rx = Arc::new(Mutex::new(start_rx_raw));
        let (release_tx, release_rx_raw) = mpsc::channel::<()>();
        let shared_release_rx = Arc::new(Mutex::new(release_rx_raw));

        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));
        let roots_fn = fake_repo;
        let scan_fn = gated_scan_fn(Arc::clone(&call_count), start_tx, shared_release_rx);

        let handle = tokio::spawn(run_reconcile_scan(
            fake_repo(),
            Arc::clone(&in_flight),
            Arc::clone(&pending),
            roots_fn,
            scan_fn,
        ));

        let first_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(first_call_number, 1);

        // Tick while pass 1 is in flight marks `pending`.
        pending.store(true, Ordering::Release);
        release_tx.send(()).expect("failed to release scan pass 1");

        let second_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(second_call_number, 2, "expected exactly one catch-up pass");

        // Tick mid-catch-up must not cause a third pass.
        pending.store(true, Ordering::Release);
        release_tx.send(()).expect("failed to release scan pass 2");

        handle.await.expect("run_reconcile_scan task panicked");

        assert_eq!(
            call_count.load(Ordering::Acquire),
            2,
            "catch-up must run exactly once per invocation, never loop"
        );
        assert!(
            pending.load(Ordering::Acquire),
            "the tick that arrived during the catch-up pass must remain \
             pending for the next scan completion to handle"
        );
        assert!(
            !in_flight.load(Ordering::Acquire),
            "in_flight must be released once the bounded catch-up completes"
        );
    }

    /// The catch-up pass re-fetches watched roots instead of reusing `initial_repos`.
    #[tokio::test]
    async fn catch_up_pass_uses_freshly_fetched_roots() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let (start_tx, start_rx_raw) = mpsc::channel::<usize>();
        let shared_start_rx = Arc::new(Mutex::new(start_rx_raw));
        let (release_tx, release_rx_raw) = mpsc::channel::<()>();
        let shared_release_rx = Arc::new(Mutex::new(release_rx_raw));

        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));

        let seen_repos: Arc<Mutex<Vec<Vec<PathBuf>>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_repos_for_scan = Arc::clone(&seen_repos);
        let call_count_for_scan = Arc::clone(&call_count);
        let scan_fn = move |repos: Vec<PathBuf>| {
            let call_number = call_count_for_scan.fetch_add(1, Ordering::AcqRel) + 1;
            seen_repos_for_scan
                .lock()
                .expect("seen_repos mutex poisoned")
                .push(repos);
            let _ = start_tx.send(call_number);
            let _ = shared_release_rx
                .lock()
                .expect("release_rx mutex poisoned")
                .recv();
        };

        let roots_fn = || vec![PathBuf::from("/fresh/repo-added-after-snapshot")];

        let handle = tokio::spawn(run_reconcile_scan(
            fake_repo(),
            Arc::clone(&in_flight),
            Arc::clone(&pending),
            roots_fn,
            scan_fn,
        ));

        let first_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(first_call_number, 1);
        pending.store(true, Ordering::Release);
        release_tx.send(()).expect("failed to release scan pass 1");

        let second_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(second_call_number, 2);
        release_tx.send(()).expect("failed to release scan pass 2");

        handle.await.expect("run_reconcile_scan task panicked");

        // Drop the guard before asserting.
        let seen = seen_repos
            .lock()
            .expect("seen_repos mutex poisoned")
            .clone();

        assert_eq!(seen.len(), 2);
        let first_pass = seen.first().expect("pass 1 recorded no repos");
        let catch_up_pass = seen.get(1).expect("catch-up pass recorded no repos");
        assert_eq!(*first_pass, fake_repo(), "pass 1 uses the initial snapshot");
        assert_eq!(
            *catch_up_pass,
            vec![PathBuf::from("/fresh/repo-added-after-snapshot")],
            "catch-up pass must use freshly-fetched roots, not the stale initial snapshot"
        );
    }
}

/// Registry and watcher agree after every prune, even when a broadcast dropped clients from the registry alone.
mod prune_reconcile_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::super::{Arc, Mutex, PathBuf, prune_dead_clients};
    use super::super::{ClientDirectory, MultiRepoWatcher, SharedWatcherSlot, WatcherConfig};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
    use std::process::Command;

    fn init_repo(root: &Path) {
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "init", "--quiet"])
            .current_dir(root)
            .status()
            .expect("run git init");
        assert!(status.success(), "git init must succeed");
    }

    fn watcher_slot() -> (SharedWatcherSlot, Arc<MultiRepoWatcher>) {
        let watcher = Arc::new(
            MultiRepoWatcher::builder()
                .config(WatcherConfig::default())
                .callback(Box::new(|_event| {}))
                .build()
                .expect("build watcher"),
        );
        (Arc::new(Mutex::new(Some(Arc::clone(&watcher)))), watcher)
    }

    /// Two canonical temp repos (the second with a subdirectory), registry, and watcher.
    struct Fixture {
        _tmp: tempfile::TempDir,
        repos: Vec<PathBuf>,
        /// `(cwd, index into repos)`
        cwds: Vec<(PathBuf, usize)>,
        registry: ClientDirectory,
        slot: SharedWatcherSlot,
        watcher: Arc<MultiRepoWatcher>,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().expect("temp dir");
        let base = std::fs::canonicalize(tmp.path()).expect("canonicalize temp dir");
        let mut repos = Vec::new();
        let mut cwds = Vec::new();
        for (index, name) in ["repo-a", "repo-b"].into_iter().enumerate() {
            let root = base.join(name);
            std::fs::create_dir_all(root.join("sub")).expect("create repo dir");
            init_repo(&root);
            cwds.push((root.clone(), index));
            cwds.push((root.join("sub"), index));
            repos.push(root);
        }
        let shell_dir = base.join("shell");
        std::fs::create_dir_all(&shell_dir).expect("create shell dir");
        let (slot, watcher) = watcher_slot();
        Fixture {
            _tmp: tmp,
            repos,
            cwds,
            registry: ClientDirectory::with_shell_dir(shell_dir),
            slot,
            watcher,
        }
    }

    // Native Windows has no start-time lookup to detect a recycled PID.
    #[cfg(unix)]
    #[test]
    fn prune_reconciles_watcher_with_registry_after_broadcast_drop() {
        let fx = fixture();
        let pid = std::process::id();
        let repo = fx.repos.first().expect("repo").clone();

        fx.registry
            .register_with_started_at_for_test(pid, Some(repo.clone()), Some(1));
        fx.watcher.register_client(pid, &repo).unwrap();
        assert_eq!(fx.watcher.watched_repo_count(), 1);

        fx.registry.notify_repaint(None);
        assert!(!fx.registry.is_registered(pid), "broadcast must drop it");
        assert_eq!(
            fx.watcher.client_pids(),
            vec![pid],
            "watcher still holds it"
        );

        let pruned = prune_dead_clients(&fx.registry, &fx.slot);

        assert_eq!(pruned, vec![pid], "reconciled pid is reported once");
        assert_eq!(fx.watcher.client_pids(), Vec::<u32>::new());
        assert_eq!(fx.watcher.watched_repo_count(), 0);
        assert_eq!(fx.watcher.watched_roots(), Vec::<std::path::PathBuf>::new());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            cases: 24,
            failure_persistence: None,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Random op sequences with prunes leave the watcher matching the registry.
        #[test]
        fn prune_keeps_watcher_matching_registry(
            steps in proptest::collection::vec(
                (0_u8..4_u8, 0_usize..2_usize, 0_usize..4_usize, proptest::bool::ANY),
                1..=24,
            ),
        ) {
            let fx = fixture();
            // Live pids, so start-time-less registrations survive the prune.
            #[cfg(unix)]
            let second_pid = std::os::unix::process::parent_id();
            // Native Windows reports every client alive, so any distinct id works.
            #[cfg(not(unix))]
            let second_pid = std::process::id().wrapping_add(1);
            let live = [std::process::id(), second_pid];
            let mut model: BTreeMap<u32, usize> = BTreeMap::new();
            for (done, (kind, pid_index, cwd_index, prune)) in steps.iter().copied().enumerate() {
                let trace = steps.get(..=done).expect("in range");
                let pid = *live.get(pid_index).expect("pid index");
                let (cwd, repo_index) = fx.cwds.get(cwd_index).expect("cwd index");
                match kind {
                    0 => {
                        fx.registry
                            .register_with_started_at_for_test(pid, Some(cwd.clone()), None);
                        fx.watcher.register_client(pid, cwd).unwrap();
                        model.insert(pid, *repo_index);
                    }
                    1 => {
                        if fx.registry.update_workspace(pid, cwd) {
                            fx.watcher.update_client(pid, cwd).unwrap();
                            model.insert(pid, *repo_index);
                        }
                    }
                    2 => {
                        fx.registry.unregister(pid);
                        fx.watcher.unregister_client(pid);
                        model.remove(&pid);
                    }
                    _ => {
                        fx.registry.unregister(pid);
                        model.remove(&pid);
                    }
                }
                if prune || trace.len() == steps.len() {
                    let _ = prune_dead_clients(&fx.registry, &fx.slot);
                    let mut registered = fx.registry.clients();
                    registered.sort_unstable();
                    assert_eq!(
                        fx.watcher.client_pids(),
                        registered,
                        "watcher pids != registry pids after {trace:?}"
                    );
                    let wanted: BTreeSet<PathBuf> = model
                        .values()
                        .filter_map(|index| fx.repos.get(*index).cloned())
                        .collect();
                    let watched: BTreeSet<PathBuf> = fx.watcher.watched_roots().into_iter().collect();
                    assert_eq!(watched, wanted, "watched repos != repos with a client after {trace:?}");
                }
            }
        }
    }
}
