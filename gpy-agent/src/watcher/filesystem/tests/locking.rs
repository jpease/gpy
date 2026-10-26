use super::*;

/// How long the stand-in `git ls-files` takes.
const SLOW_BACKEND_RUN: Duration = Duration::from_millis(300);

/// Budget for a second repo's lookup; well under [`SLOW_BACKEND_RUN`].
const FAST_LOOKUP_BUDGET: Duration = Duration::from_millis(100);

/// Generous ceiling for gates and async refreshes; only a hang reaches it.
const GATE_BUDGET: Duration = Duration::from_secs(10);

/// A one-shot gate: the stand-in backend opens it, the test waits on it.
type TestGate = Arc<(Mutex<bool>, std::sync::Condvar)>;

/// Create a closed [`TestGate`].
fn new_gate() -> TestGate {
    Arc::new((Mutex::new(false), std::sync::Condvar::new()))
}

/// Open `gate`, waking every waiter.
fn open_gate(gate: &TestGate) {
    let (flag, condvar) = &**gate;
    *flag.lock().unwrap() = true;
    condvar.notify_all();
}

/// Block until `gate` opens; `false` means `budget` elapsed first.
fn wait_for_gate(gate: &TestGate, budget: Duration) -> bool {
    let (flag, condvar) = &**gate;
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    let mut open = flag.lock().unwrap();
    while !*open {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let (next, _) = condvar.wait_timeout(open, remaining).unwrap();
        open = next;
    }
    drop(open);
    true
}

/// Give `repo` the `.git/index` [`force_added_set`] fingerprints, without `git init`.
fn fake_repo_with_index(tmp: &tempfile::TempDir) -> PathBuf {
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".git").join("index"), b"index").expect("write index");
    repo
}

/// As [`fake_repo_with_index`], but leaks `tmp` so the path can outlive it.
fn fake_repo_with_index_leaked(tmp: tempfile::TempDir) -> PathBuf {
    let repo = std::fs::canonicalize(tmp.keep()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".git").join("index"), b"index").expect("write index");
    repo
}

/// `force_added_set` must not hold the cache mutex while `git ls-files` runs.
#[test]
fn force_added_refresh_never_holds_the_cache_mutex() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = fake_repo_with_index(&tmp);

    let entered = new_gate();
    let release = new_gate();
    let entered_for_ls_files = Arc::clone(&entered);
    let release_for_ls_files = Arc::clone(&release);
    let registry = test_registry_with_ls_files(true, HashSet::new(), move |_| {
        open_gate(&entered_for_ls_files);
        wait_for_gate(&release_for_ls_files, GATE_BUDGET);
        Vec::new()
    });

    let repo_for_worker = repo.clone();
    let registry_for_worker = Arc::clone(&registry);
    let worker = std::thread::spawn(move || {
        drop(force_added_set(&repo_for_worker, &registry_for_worker));
    });

    assert!(
        wait_for_gate(&entered, GATE_BUDGET),
        "the stand-in backend should have been entered"
    );
    let cache_free_during_git = registry.ignore().force_added.try_lock().is_ok();
    open_gate(&release);
    worker.join().expect("worker thread");

    assert!(
        cache_free_during_git,
        "the force-added cache mutex must be free while `git ls-files` runs"
    );
}

/// One repo's slow `git ls-files` must not stall another repo's
/// lookup through the shared force-added cache mutex.
#[test]
fn a_slow_repo_does_not_delay_another_repos_lookup() {
    let slow_tmp = tempfile::TempDir::new().expect("temp dir");
    let fast_tmp = tempfile::TempDir::new().expect("temp dir");
    let slow_repo = fake_repo_with_index(&slow_tmp);
    let fast_repo = fake_repo_with_index(&fast_tmp);

    let entered = new_gate();
    let entered_for_ls_files = Arc::clone(&entered);
    let slow_for_ls_files = slow_repo.clone();
    let registry = test_registry_with_ls_files(true, HashSet::new(), move |root| {
        if root == slow_for_ls_files {
            open_gate(&entered_for_ls_files);
            std::thread::sleep(SLOW_BACKEND_RUN);
        }
        Vec::new()
    });

    let slow_for_worker = slow_repo.clone();
    let registry_for_worker = Arc::clone(&registry);
    let worker = std::thread::spawn(move || {
        drop(force_added_set(&slow_for_worker, &registry_for_worker));
    });

    assert!(
        wait_for_gate(&entered, GATE_BUDGET),
        "the slow stand-in backend should have been entered"
    );
    let started = Instant::now();
    drop(force_added_set(&fast_repo, &registry));
    let elapsed = started.elapsed();

    worker.join().expect("worker thread");

    assert!(
        elapsed < FAST_LOOKUP_BUDGET,
        "a lookup for an unrelated repo took {elapsed:?}, so it queued behind \
         the slow repo's `git ls-files`"
    );
}

/// One repo's cold-start poll walk (which may run `git ls-files`) must not
/// block another repo's `watch()` on the shared backend mutex.
#[test]
fn cold_start_for_one_repo_does_not_block_another_repos_watch() {
    let primary_tmp = tempfile::TempDir::new().expect("temp dir");
    let secondary_tmp = tempfile::TempDir::new().expect("temp dir");
    let repo_a = fake_repo_with_index(&primary_tmp);
    let repo_b = fake_repo_with_index(&secondary_tmp);

    // Ignored, so the descent reaches `force_added_set`.
    std::fs::write(repo_a.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
    std::fs::create_dir_all(repo_a.join("ignored")).expect("ignored dir");

    let entered = new_gate();
    let release = new_gate();
    let entered_for_ls_files = Arc::clone(&entered);
    let release_for_ls_files = Arc::clone(&release);
    let gated_repo = repo_a.clone();
    let registry = test_registry_with_ls_files(
        true,
        HashSet::from([repo_a.clone(), repo_b.clone()]),
        move |root| {
            if root == gated_repo {
                open_gate(&entered_for_ls_files);
                wait_for_gate(&release_for_ls_files, GATE_BUDGET);
            }
            Vec::new()
        },
    );
    let (callback, _events) = recording_callback();
    let watcher = FileSystemWatcher::with_fallback_policy(
        callback,
        Some(Arc::clone(&registry)),
        Duration::from_millis(50),
        no_op_scheduler(),
        FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_secs(3_600),
        },
    )
    .expect("create watcher");

    std::thread::scope(|scope| {
        let watcher_ref = &watcher;
        let repo_a_for_worker = repo_a.clone();
        let worker = scope.spawn(move || {
            watcher_ref.watch(&repo_a_for_worker).expect("watch repo a");
        });

        assert!(
            wait_for_gate(&entered, GATE_BUDGET),
            "the stand-in backend for repo A's cold start should have been entered"
        );

        let started = Instant::now();
        watcher.watch(&repo_b).expect("watch repo b");
        let elapsed = started.elapsed();

        open_gate(&release);
        worker.join().expect("worker thread");

        assert!(
            elapsed < FAST_LOOKUP_BUDGET,
            "watch() for an unrelated repo took {elapsed:?} while repo A's cold \
             start was still running, so it queued behind the shared backend \
             mutex"
        );
    });
}

/// Fixture parking `force_poll_fallback` between its snapshot and the swap,
/// so repo B arms natively and must be re-armed by the race fallback.
fn swap_race_fixture() -> (FileSystemWatcher, PathBuf, PathBuf, TestGate, TestGate) {
    let repo_a = fake_repo_with_index_leaked(tempfile::TempDir::new().expect("temp dir"));
    let repo_b = fake_repo_with_index_leaked(tempfile::TempDir::new().expect("temp dir"));
    std::fs::write(repo_a.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
    std::fs::create_dir_all(repo_a.join("ignored")).expect("ignored dir");

    // Gates repo A's `git ls-files` only once armed, after its native watch.
    let armed = Arc::new(AtomicBool::new(false));
    let entered = new_gate();
    let release = new_gate();
    let armed_for_ls_files = Arc::clone(&armed);
    let entered_for_ls_files = Arc::clone(&entered);
    let release_for_ls_files = Arc::clone(&release);
    let gated_repo = repo_a.clone();
    let registry = test_registry_with_ls_files(
        true,
        HashSet::from([repo_a.clone(), repo_b.clone()]),
        move |root| {
            if root == gated_repo && armed_for_ls_files.load(Ordering::SeqCst) {
                open_gate(&entered_for_ls_files);
                wait_for_gate(&release_for_ls_files, GATE_BUDGET);
            }
            Vec::new()
        },
    );
    let (callback, _events) = recording_callback();
    let watcher = FileSystemWatcher::with_fallback_policy(
        callback,
        Some(Arc::clone(&registry)),
        Duration::from_millis(50),
        no_op_scheduler(),
        FallbackPolicy::NativeOnly,
    )
    .expect("create watcher");
    watcher.watch(&repo_a).expect("watch repo a natively");
    armed.store(true, Ordering::SeqCst);

    (watcher, repo_a, repo_b, entered, release)
}

#[test]
fn watch_racing_the_native_to_poll_swap_still_ends_up_armed_correctly() {
    let (watcher, _repo_a, repo_b, entered, release) = swap_race_fixture();

    std::thread::scope(|scope| {
        let watcher_ref = &watcher;
        let swap = scope.spawn(move || {
            watcher_ref.force_poll_fallback(Duration::from_secs(3_600));
        });

        assert!(
            wait_for_gate(&entered, GATE_BUDGET),
            "the swap should have parked on repo A's cold start"
        );
        assert_eq!(
            watcher.backend_kind(),
            BackendKind::Native,
            "the swap must not have applied yet -- it is parked before its own \
             final lock"
        );

        watcher
            .watch(&repo_b)
            .expect("watch repo b while the swap is parked");

        open_gate(&release);
        swap.join().expect("swap thread");
    });

    assert_eq!(
        watcher.backend_kind(),
        BackendKind::Poll,
        "the swap must have completed"
    );
    let guard = watcher.backend.lock().expect("backend lock");
    let registration = guard
        .registrations
        .get(&(repo_b.clone(), WatchMode::Recursive))
        .expect("repo B must still be a watched path after the swap");
    assert!(
        registration.targets.contains(&(repo_b, WatchMode::Shallow)),
        "repo B, added while the swap was mid-flight and so absent from its \
         pre-swap snapshot, must still end up re-armed on the poll backend \
         with its expansion, got: {:?}",
        registration.targets
    );
    drop(guard);
}

/// After the off-thread refresh, a newly force-added file is picked up and the
/// earlier one stays exempt.
#[test]
fn force_added_set_picks_up_an_index_change_after_the_refresh() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

    git_init(&repo);
    std::fs::write(repo.join(".gitignore"), "*.log\n").expect("write gitignore");
    std::fs::write(repo.join("first.log"), "tracked\n").expect("write first.log");
    std::fs::write(repo.join("other.log"), "untracked\n").expect("write other.log");
    git_in(&repo, &["add", "-f", "first.log"]);
    git_in(&repo, &["add", ".gitignore"]);
    git_in(&repo, &["commit", "-m", "init"]);

    assert!(
        !path_is_gitignored(&repo, &repo.join("first.log"), &registry),
        "the force-added file must not be suppressed"
    );

    // A second force-add rewrites `.git/index`, staling the cached set.
    let second = repo.join("second.log");
    std::fs::write(&second, "also tracked\n").expect("write second.log");
    git_in(&repo, &["add", "-f", "second.log"]);

    let deadline = Instant::now()
        .checked_add(GATE_BUDGET)
        .unwrap_or_else(Instant::now);
    let mut picked_up = false;
    while Instant::now() < deadline {
        if is_force_added(&repo, &second, &registry) {
            picked_up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        picked_up,
        "the newly force-added file must appear once the refresh completes"
    );
    assert!(
        !path_is_gitignored(&repo, &repo.join("first.log"), &registry),
        "the original force-add must survive the refresh"
    );
    assert!(
        path_is_gitignored(&repo, &repo.join("other.log"), &registry),
        "an untracked file matching the ignore glob must still be suppressed"
    );
}

/// A nested `.gitignore` un-ignoring a root-ignored path is honored and stays live.
#[test]
fn nested_unignore_is_honored_and_invalidates_on_edit() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".gitignore"), "build/\n").expect("root gitignore");

    let subdir = repo.join("subdir");
    std::fs::create_dir_all(subdir.join("build")).expect("nested build dir");
    let target = subdir.join("build").join("foo");
    std::fs::write(&target, "x").expect("write target");

    assert!(
        path_is_gitignored(repo, &target, &registry),
        "root build/ rule should ignore the nested path with no override"
    );

    std::fs::write(subdir.join(".gitignore"), "!build/\n").expect("nested un-ignore");
    assert!(
        !path_is_gitignored(repo, &target, &registry),
        "a nested !build/ rule must re-include the path"
    );

    // Different length, so the fingerprint flips deterministically.
    std::fs::write(subdir.join(".gitignore"), "# no override here now\n")
        .expect("rewrite nested gitignore");
    assert!(
        path_is_gitignored(repo, &target, &registry),
        "dropping the nested negation must re-ignore the path without a restart"
    );
}
