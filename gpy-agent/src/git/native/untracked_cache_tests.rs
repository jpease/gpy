use super::tests::init_test_repo;
use super::*;

/// `git config --local --get-all core.untrackedCache` as seen from `path`.
fn untracked_cache_values(path: &Path) -> String {
    let output = Command::new("git")
        .args(["config", "--local", "--get-all", "core.untrackedCache"])
        .current_dir(path)
        .output()
        .expect("run git config");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Whether `<repo>/.git/index` carries the `UNTR` extension. Fixture file
/// names are lowercase, so a path cannot produce the same four bytes.
fn index_has_untracked_cache(repo: &Path) -> bool {
    fs::read(repo.join(".git").join("index"))
        .expect("read index")
        .windows(4)
        .any(|window| window == b"UNTR")
}

/// Some overlay, NFS and bind-mount filesystems fail git's mtime probe, and
/// git then refuses to add the extension.
fn filesystem_supports_untracked_cache(repo: &Path) -> bool {
    Command::new("git")
        .args(["update-index", "--test-untracked-cache"])
        .current_dir(repo)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn untracked_cache_goes_to_the_index_not_git_config() {
    let (_dir, path) = init_test_repo();
    // A user/system-level setting would (correctly) stop gpy adding it.
    if NativeGitBackend::is_untracked_cache_configured(&path, NativeGitBackend::default_deadline())
        || !filesystem_supports_untracked_cache(&path)
    {
        return;
    }

    NativeGitBackend::get_complete_status(&path, 100, false, Duration::from_secs(5))
        .expect("status");

    assert_eq!(untracked_cache_values(&path), "");
    assert!(index_has_untracked_cache(&path));
}

#[test]
fn untracked_cache_setup_respects_explicit_false() {
    let (_dir, path) = init_test_repo();
    let status = Command::new("git")
        .args(["config", "core.untrackedCache", "false"])
        .current_dir(&path)
        .status()
        .expect("run git config");
    assert!(status.success());
    // A user-level `core.untrackedCache = true` makes `git add` in the
    // fixture write the extension; start from an index without it.
    let update = Command::new("git")
        .args(["update-index", "--no-untracked-cache"])
        .current_dir(&path)
        .status()
        .expect("run git update-index");
    assert!(update.success());

    NativeGitBackend::get_complete_status(&path, 100, false, Duration::from_secs(5))
        .expect("status");

    assert_eq!(untracked_cache_values(&path), "false");
    assert!(!index_has_untracked_cache(&path));
}
