//! Identity of a bound socket file: the single ownership token shared by the
//! IPC server, the lifecycle start/eviction path and the version marker (#723).

use std::path::Path;

/// Identity of a bound socket file, used to prove ownership before unlinking.
///
/// The inode number alone is not enough. On Linux, deleting a file and
/// immediately creating another at the same path reuses the freed inode number
/// essentially every time (5 of 5 in a container check), so an ino-only
/// comparison told a departing agent that the *replacement* agent's socket was
/// its own — and it unlinked it, which is exactly the restart race
/// `EndpointHandle::unlink_socket_if_owned` exists to prevent. macOS APFS
/// never reuses inode numbers, so the gap was invisible there and only ever
/// showed on Linux CI.
///
/// A reused inode still carries a fresh `ctime`, so pairing the two closes the
/// hole. The device id guards the (rarer) case of the path moving between
/// filesystems. Every field is captured from a single `stat`.
///
/// The comparison is deliberately strict: any mismatch means "not ours", and
/// the caller leaves the file alone. Being wrong in that direction leaves a
/// stale socket behind, which startup already detects and clears; being wrong
/// in the other direction destroys a live agent's endpoint.
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
