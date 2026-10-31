use super::tests::init_test_repo;
use super::*;

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?} failed");
}

#[test]
fn pathspec_status_reports_only_named_paths() {
    let (_dir, path) = init_test_repo();
    fs::write(path.join("other.txt"), "one\n").expect("write other.txt");
    git(&path, &["add", "other.txt"]);
    git(&path, &["commit", "-q", "-m", "other"]);
    fs::write(path.join("file.txt"), "changed\n").expect("modify file.txt");
    fs::write(path.join("other.txt"), "changed\n").expect("modify other.txt");

    let scan = NativeGitBackend::get_pathspec_status(
        &path,
        &[PathBuf::from("file.txt")],
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("pathspec status");

    assert_eq!(scan.files.len(), 1);
    assert!(
        scan.files
            .get(Path::new("file.txt"))
            .is_some_and(|status| status.unstaged)
    );
}

#[test]
fn pathspec_status_works_on_detached_head() {
    let (_dir, path) = init_test_repo();
    git(&path, &["checkout", "-q", "--detach"]);
    fs::write(path.join("file.txt"), "changed\n").expect("modify file.txt");

    let scan = NativeGitBackend::get_pathspec_status(
        &path,
        &[PathBuf::from("file.txt")],
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("pathspec status");

    assert!(
        scan.files
            .get(Path::new("file.txt"))
            .is_some_and(|status| status.unstaged)
    );
}

/// A pathspec is passed to git as the raw `OsStr`, so a file whose
/// name is not valid UTF-8 is still matched. Skipped where the filesystem
/// rejects such names (APFS).
#[cfg(unix)]
#[test]
fn pathspec_status_matches_a_non_utf8_file_name() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (_dir, path) = init_test_repo();
    let name = PathBuf::from(OsStr::from_bytes(b"bad\xff.txt"));
    if fs::write(path.join(&name), "one\n").is_err() {
        return;
    }
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "non-utf8"]);
    fs::write(path.join(&name), "two\n").expect("modify non-UTF-8 file");

    let scan = NativeGitBackend::get_pathspec_status(
        &path,
        std::slice::from_ref(&name),
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("pathspec status");

    assert!(
        scan.files.get(&name).is_some_and(|status| status.unstaged),
        "the edit must be seen through the pathspec"
    );
}
