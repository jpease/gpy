//! The untracked-cache guard (#1204) against a real stale cache.
//!
//! Git trusts a cached directory listing while the directory's whole-second
//! mtime is unchanged. With `core.trustctime=false`, restoring an old mtime
//! after a change makes the listing stale on demand, without racing the clock.

use super::tests::init_test_repo;
use super::*;
use std::collections::BTreeSet;
use std::fs::File;
use std::time::SystemTime;

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?} failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Untracked paths from a plain `git status`, which uses and writes the cache.
fn plain_status_untracked(path: &Path) -> BTreeSet<String> {
    git(path, &["status", "--porcelain"])
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .map(str::to_owned)
        .collect()
}

fn backend_untracked(path: &Path) -> BTreeSet<String> {
    NativeGitBackend::get_complete_status(path, 100, false, Duration::from_secs(10))
        .expect("status")
        .expect("a repository")
        .files
        .into_iter()
        .filter(|(_, status)| status.untracked)
        .map(|(file, _)| file.to_string_lossy().into_owned())
        .collect()
}

fn set_dir_mtime(dir: &Path, when: SystemTime) {
    File::open(dir)
        .and_then(|handle| handle.set_modified(when))
        .expect("set directory mtime");
}

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// A repo whose index carries the untracked cache, with `u1.txt` cached and
/// then renamed to `u2.txt` behind an unchanged directory mtime.
fn repo_with_stale_cache() -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = init_test_repo();
    git(&path, &["config", "core.untrackedCache", "true"]);
    git(&path, &["config", "core.trustctime", "false"]);
    git(&path, &["update-index", "--untracked-cache"]);

    let past = SystemTime::now()
        .checked_sub(Duration::from_secs(10))
        .expect("ten seconds ago");
    fs::write(path.join("u1.txt"), "x").expect("write u1");
    set_dir_mtime(&path, past);
    assert_eq!(plain_status_untracked(&path), names(&["u1.txt"]));

    fs::rename(path.join("u1.txt"), path.join("u2.txt")).expect("rename");
    set_dir_mtime(&path, past);
    assert_eq!(
        plain_status_untracked(&path),
        names(&["u1.txt"]),
        "fixture must reproduce the stale listing"
    );
    (dir, path)
}

#[test]
fn stale_untracked_listing_is_reported_correctly_and_repaired() {
    let (_dir, path) = repo_with_stale_cache();

    assert_eq!(backend_untracked(&path), names(&["u2.txt"]));
    assert_eq!(
        plain_status_untracked(&path),
        names(&["u2.txt"]),
        "git's own status must stop replaying the stale listing"
    );
}

#[test]
fn a_correct_cache_is_left_alone() {
    let (_dir, path) = init_test_repo();
    git(&path, &["config", "core.untrackedCache", "true"]);
    git(&path, &["update-index", "--untracked-cache"]);
    fs::write(path.join("fresh.txt"), "x").expect("write");
    assert_eq!(plain_status_untracked(&path), names(&["fresh.txt"]));
    let index = path.join(".git").join("index");
    let before = fs::metadata(&index)
        .expect("index")
        .modified()
        .expect("mtime");

    assert_eq!(backend_untracked(&path), names(&["fresh.txt"]));
    assert_eq!(backend_untracked(&path), names(&["fresh.txt"]));

    let after = fs::metadata(&index)
        .expect("index")
        .modified()
        .expect("mtime");
    assert_eq!(before, after, "a matching cache must not rewrite the index");
}
