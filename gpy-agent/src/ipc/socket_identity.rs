//! Ownership token for a bound socket file.

use std::path::Path;

/// Identity of a bound socket file, used to prove ownership before unlinking.
///
/// Inode alone is insufficient: Linux reuses a freed inode number almost
/// immediately, so a departing agent would unlink its replacement's socket.
/// A reused inode gets a fresh `ctime`; `dev` covers cross-filesystem moves.
/// Any mismatch means "not ours": a stale socket is recoverable, a deleted live one is not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SocketIdentity {
    dev: u64,
    ino: u64,
    ctime: i64,
    ctime_nsec: i64,
}

impl SocketIdentity {
    /// Identity of the file described by `metadata`.
    pub fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        }
    }

    /// Identity of the socket at `path` right now (one `lstat`).
    ///
    /// Anything that is not a socket (a regular file, a directory, a symlink,
    /// even one to a socket) is reported as `NotFound`: no socket is there, so
    /// nothing at the path is ever treated as ours to unlink.
    ///
    /// # Errors
    ///
    /// Returns the `lstat` error, or `NotFound` when no socket is at `path`.
    pub fn at(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::FileTypeExt;
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} is not a socket", path.display()),
            ));
        }
        Ok(Self::from_metadata(&metadata))
    }
}
