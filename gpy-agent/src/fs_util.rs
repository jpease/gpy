//! Small filesystem predicates and helpers shared across handlers.

use std::io::Read as _;
use std::path::Path;

/// Read up to `cap` bytes of `path` as UTF-8 (lossy), never more.
///
/// For repo-controlled files (`.git/**`, language-version files, plugin
/// manifests) where the crate must not let a maliciously or accidentally
/// huge file drive an unbounded read.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
pub fn read_small_file(path: &Path, cap: usize) -> std::io::Result<String> {
    let limit = u64::try_from(cap).unwrap_or(u64::MAX);
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Whether `path` (expected to be a directory) is NOT writable by the
/// current user.
///
/// On Unix this checks real access rights via `access(2)` (`W_OK`), not
/// mode bits — `std::fs::Permissions::readonly()` is `mode & 0o222 == 0`,
/// which ignores ownership and ACLs and so reports a root-owned 755
/// directory as writable to any user (#566). A path that doesn't exist, or
/// that `access` can't evaluate, is treated as read-only (fail closed —
/// this only feeds a display glyph, so the safe default is "don't claim
/// writable").
#[cfg(unix)]
#[must_use]
pub fn directory_is_read_only(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::W_OK).is_err()
}

/// Non-Unix fallback: mode-bit check via `std::fs::Permissions::readonly()`.
/// Native Windows has no POSIX `access(2)` equivalent wired through `nix`;
/// ACL-aware evaluation there is explicitly out of scope for #566.
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

    /// Regression test for #566: `std::fs::Permissions::readonly()` is mode
    /// bits (`mode & 0o222 == 0`), which ignores ownership.
    ///
    /// `/usr` is root-owned, mode 755 (world-readable/executable,
    /// owner-writable only) on every macOS/Linux box this runs on, so a normal
    /// user can never write there even though the mode bits alone (`0o755 &
    /// 0o222 != 0`, since the owner-write bit is set) would say "writable".
    /// The old `metadata().permissions().readonly()` logic reports `false`
    /// (writable) for exactly this path — this test would fail against that
    /// implementation and only passes against the `access(2)`-based fix.
    ///
    /// Skipped when running as root: root can write anywhere `access(2)`
    /// or not, so the assertion wouldn't hold and isn't testing anything
    /// meaningful in that context.
    #[test]
    fn root_owned_755_directory_is_read_only_for_normal_user() {
        if nix::unistd::Uid::effective().is_root() {
            // No meaningful assertion for root: everything is writable to
            // root regardless of ownership/mode, so this scenario can't be
            // exercised in a root-run test process (e.g. some CI images).
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

        // `TempDir::drop` needs write access on the directory itself to
        // remove its (empty) contents; restore a writable mode before the
        // `TempDir` goes out of scope so cleanup doesn't leak this entry
        // under /tmp or emit a drop-time error.
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))
            .expect("restore temp dir permissions before drop");
    }
}
