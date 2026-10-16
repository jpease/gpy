//! Small filesystem helpers.

use std::io::Read as _;
use std::path::Path;

/// Read at most `cap` bytes of `path` as lossy UTF-8, so huge repo-controlled
/// files cannot force an unbounded read.
///
/// # Errors
/// Returns an error if the file cannot be opened or read.
pub fn read_small_file(path: &Path, cap: usize) -> std::io::Result<String> {
    let limit = u64::try_from(cap).unwrap_or(u64::MAX);
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Whether the current user cannot write `path`. Uses `access(2)`, not mode
/// bits, so ownership and ACLs count; fails closed.
#[cfg(unix)]
#[must_use]
pub fn directory_is_read_only(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::W_OK).is_err()
}

/// Mode-bit fallback; no `access(2)` equivalent here.
#[cfg(not(unix))]
#[must_use]
pub fn directory_is_read_only(path: &Path) -> bool {
    std::fs::metadata(path).map_or(true, |m| m.permissions().readonly())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{directory_is_read_only, read_small_file};
    #[cfg(unix)]
    use std::path::Path;

    #[test]
    fn read_small_file_reads_a_small_file_fully() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("small.txt");
        std::fs::write(&path, "hello world").unwrap();

        assert_eq!(read_small_file(&path, 4_096).unwrap(), "hello world");
    }

    #[test]
    fn read_small_file_truncates_at_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.txt");
        std::fs::write(&path, "abcdefghij").unwrap();

        let result = read_small_file(&path, 4).unwrap();
        assert_eq!(result.len(), 4, "result should be capped at 4 bytes");
        assert_eq!(result, "abcd");
    }

    #[test]
    fn read_small_file_errors_on_nonexistent_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.txt");

        assert!(read_small_file(&path, 4_096).is_err());
    }

    /// Mode bits say root-owned 755 `/usr` is writable; `access(2)` knows better.
    #[cfg(unix)]
    #[test]
    fn root_owned_755_directory_is_read_only_for_normal_user() {
        if nix::unistd::Uid::effective().is_root() {
            return;
        }

        let usr = Path::new("/usr");
        assert!(
            usr.is_dir(),
            "/usr must exist and be a directory for this test to be meaningful"
        );

        assert!(
            directory_is_read_only(usr),
            "/usr is root-owned mode 755 and must report read-only for a normal user"
        );
    }

    #[test]
    fn fresh_temp_dir_is_writable() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        assert!(
            !directory_is_read_only(tmp.path()),
            "a freshly created temp dir defaults to user-writable"
        );
    }

    #[test]
    #[cfg(unix)]
    fn mode_555_temp_dir_is_read_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("create temp dir");

        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o555))
            .expect("chmod temp dir to 0o555");

        assert!(
            directory_is_read_only(tmp.path()),
            "a mode 0o555 temp dir must report read-only"
        );

        // `TempDir::drop` needs write access to clean up.
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))
            .expect("restore temp dir permissions before drop");
    }
}
