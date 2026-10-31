//! Small filesystem helpers.

use crate::deadline::Deadline;
use std::io::Read as _;
use std::path::Path;

/// Read at most `cap` bytes of `path` as lossy UTF-8, so huge repo-controlled
/// files cannot force an unbounded read.
///
/// # Errors
/// Returns an error if `path` is not a regular file or cannot be opened or read.
pub fn read_small_file(path: &Path, cap: usize) -> std::io::Result<String> {
    let bytes = read_regular_file_capped(path, cap)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read at most `cap` bytes of `path`, refusing anything but a regular file.
///
/// The open is non-blocking, so a FIFO without a writer cannot stall it, and
/// the type check runs on the opened descriptor, so swapping a special file in
/// after any earlier check still fails here.
///
/// # Errors
/// Returns an error if `path` is not a regular file or cannot be opened or read.
pub fn read_regular_file_capped(path: &Path, cap: usize) -> std::io::Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // O_NOCTTY: a symlink to a terminal must not become the daemon's controlling tty.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a regular file: {}", path.display()),
        ));
    }
    let limit = u64::try_from(cap).unwrap_or(u64::MAX);
    let mut buf = Vec::new();
    file.take(limit).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Create `path` with `contents`, failing with `AlreadyExists` instead of
/// replacing a file that is already there (including one created concurrently).
///
/// # Errors
/// Returns an error if the file exists or cannot be created or written.
pub fn create_new(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(contents)
}

/// [`create_new`] for a file the script-plugin policy must accept: requested
/// mode `0644`, narrowed (never widened) by the process umask, so it is never
/// group- or world-writable whatever the umask. Windows has no mode bits.
///
/// # Errors
/// Returns an error if the file exists or cannot be created or written.
pub fn create_new_unshared(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o644);
    }
    options.open(path)?.write_all(contents)
}

/// Create the single directory `path` (parent must exist) with requested mode
/// `0755`, narrowed by the process umask, so it is never group- or
/// world-writable. Fails with `AlreadyExists` if `path` exists.
///
/// # Errors
/// Returns an error if the directory exists or cannot be created.
pub fn create_dir_unshared(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o755);
        builder
    };
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    builder.create(path)
}

static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Replace `path` with `contents` through a temp file in the same directory
/// and a rename, so a failed write (disk full, crash) leaves the previous file
/// whole. A symlinked `path` is resolved first so the link survives, and an
/// existing file's permissions carry over. The parent directory must exist.
///
/// # Errors
/// Returns an error if a symlink cannot be resolved or the temp file cannot be
/// written or renamed into place; the temp file is removed on failure.
pub fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let target = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let count = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = parent.join(format!(".{file_name}.tmp.{}.{count}", std::process::id()));

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        // Match the target's mode while the temp file is still empty: a private
        // file's contents must never sit in a wider-readable temp file.
        if let Ok(meta) = std::fs::metadata(&target) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)
    })();
    if result.is_err() {
        drop(std::fs::remove_file(&temp));
    }
    result
}

/// Exclusive advisory lock on `<path>.lock`, released on drop. Serializes
/// read-modify-write cycles on `path` across processes; the lock file stays in
/// place because unlinking one races like unlinking a socket.
pub struct PathLock {
    lock: std::fs::File,
}

impl Drop for PathLock {
    fn drop(&mut self) {
        // Explicit, not just the close: Windows frees a closed handle's
        // locks only "eventually" (`LockFileEx` docs), which would stall the
        // next writer. Closing still releases it if this fails.
        drop(self.lock.unlock());
    }
}

/// Take the exclusive lock beside `path`, polling for up to `wait`. The parent
/// directory must exist. Resolve symlinks first so every writer locks the same
/// file. The lock is `flock` on Unix and `LockFileEx` on Windows, both through
/// [`std::fs::File::try_lock`]; on Windows it is mandatory, which is harmless
/// because nothing reads or writes the lock file.
///
/// # Errors
/// Returns an error if the lock file cannot be opened, or `TimedOut` if another
/// holder keeps the lock for the whole `wait`.
pub fn lock_beside(path: &Path, wait: std::time::Duration) -> std::io::Result<PathLock> {
    use std::fs::TryLockError;

    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let mut options = std::fs::OpenOptions::new();
    // `write`, not `read`: the lock only needs a handle with either access.
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(&lock_path)?;
    let deadline = Deadline::after(wait);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(PathLock { lock: file }),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(TryLockError::Error(e)) => return Err(e),
        }
        if deadline.expired() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("{} is held by another process", lock_path.to_string_lossy()),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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

    #[test]
    fn read_small_file_rejects_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_small_file(tmp.path(), 4_096).is_err());
    }

    /// A writer-less FIFO would block a plain `open` forever; it must fail fast.
    #[cfg(unix)]
    #[test]
    fn read_small_file_rejects_a_fifo_without_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pipe");
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::S_IRWXU).unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(read_small_file(&path, 4_096).is_err()));
        let rejected = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("reading a FIFO blocked"));
        assert!(rejected, "a FIFO is not a regular file");
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

    /// The temp file takes the target's mode before any content is written, so a
    /// private file's contents are never readable through a wider temp file.
    #[test]
    #[cfg(unix)]
    fn write_atomically_keeps_a_private_file_private() {
        use super::write_atomically;
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rc");
        std::fs::write(&path, "secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        write_atomically(&path, b"replacement").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
    }

    /// A write that cannot complete must leave the previous file intact. An
    /// in-place write would succeed here (the file itself is writable) and is
    /// exactly the truncate-then-fail shape this guards against.
    #[test]
    #[cfg(unix)]
    fn write_atomically_failure_leaves_previous_file_intact() {
        use super::write_atomically;
        use std::os::unix::fs::PermissionsExt;

        if nix::unistd::Uid::effective().is_root() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("theme.toml");
        std::fs::write(&path, "previous").unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = write_atomically(&path, b"replacement");
        let entries = std::fs::read_dir(tmp.path()).unwrap().count();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err(), "a read-only directory must fail the write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous");
        assert_eq!(entries, 1, "no temp file may be left behind");
    }

    /// Concurrent atomic writes to the same target don't collide on temp files.
    #[test]
    fn concurrent_write_atomically_does_not_collide() {
        use super::write_atomically;
        use std::sync::Arc;

        let tmp = tempfile::tempdir().unwrap();
        let path = Arc::new(tmp.path().join("concurrent.txt"));
        let mut handles = Vec::new();

        for i in 0_u32..10_u32 {
            let p = Arc::clone(&path);
            handles.push(std::thread::spawn(move || {
                let data = format!("thread-{i}");
                write_atomically(&p, data.as_bytes()).unwrap();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let content = std::fs::read_to_string(&*path).unwrap();
        assert!(content.starts_with("thread-"));
    }

    /// A second locker waits out the first and gets in once it drops.
    #[test]
    fn lock_beside_excludes_until_the_holder_drops() {
        use super::lock_beside;
        use std::time::Duration;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let held = lock_beside(&path, Duration::from_secs(1)).unwrap();
        let blocked = lock_beside(&path, Duration::from_millis(50));
        assert_eq!(
            blocked.err().map(|e| e.kind()),
            Some(std::io::ErrorKind::TimedOut),
            "a held lock must time out the second locker"
        );

        drop(held);
        assert!(lock_beside(&path, Duration::from_secs(1)).is_ok());
    }
}
