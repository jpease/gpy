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

    /// Identity of whatever is at `path` right now (one `stat`).
    ///
    /// # Errors
    ///
    /// Returns the `stat` error, e.g. `NotFound` when nothing is at `path`.
    pub fn at(path: &Path) -> std::io::Result<Self> {
        std::fs::metadata(path).map(|metadata| Self::from_metadata(&metadata))
    }
}
