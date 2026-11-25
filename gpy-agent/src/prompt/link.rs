//! Talking to the agent from `gpy prompt`: bounded data requests,
//! fire-and-forget refreshes behind the per-shell refresh-throttle stamps,
//! and the per-shell late window, both in `<runtime root>/refresh-throttle/`.
//! That directory holds script output and declared environment values, so
//! it is owner-only (`0700`) and its files are created owner-only (`0600`).

use crate::cache::InstantPromptCache;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How a bounded request ended (`__gpy_send_json`'s statuses).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// A complete, non-empty reply.
    Text(String),
    /// No agent listening, or agent mode off.
    Unreachable,
    /// Connected, but nothing usable in time: omit, never recompute.
    Late,
    /// An `{"error"` reply: permanent, take the local fallback.
    Refused(String),
}

/// Minimum gap between two refreshes of one bucket.
const REFRESH_GAP_MS: u128 = 500;

/// Seconds a connected-but-silent agent is left alone
/// (`__gpy_agent_late_secs`).
const LATE_SECS: u64 = 5;

/// `GPY_IPC_TIMEOUT_MS`, default 150.
fn warm_timeout_ms() -> u64 {
    env_ms("GPY_IPC_TIMEOUT_MS").unwrap_or(150)
}

/// `GPY_IPC_COLD_TIMEOUT_MS` (default 500), never below the warm budget.
pub(crate) fn cold_timeout_ms() -> u64 {
    env_ms("GPY_IPC_COLD_TIMEOUT_MS")
        .unwrap_or(500)
        .max(warm_timeout_ms())
}

/// A digits-only environment value.
fn env_ms(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse().ok())
}

/// One agent connection context for a render.
#[derive(Debug)]
pub(crate) struct Link {
    /// `__gpy_agent_enabled`.
    enabled: bool,
    /// The agent socket, when one resolves.
    socket: Option<PathBuf>,
    /// `<runtime root>/refresh-throttle`, when a runtime root exists.
    throttle_dir: Option<PathBuf>,
    /// The shell's PID, naming its stamps and late window.
    pid: u32,
}

impl Link {
    /// Resolve the socket and runtime directories for shell `pid`.
    pub(crate) fn new(enabled: bool, pid: u32) -> Self {
        let socket = crate::paths::SocketPath::resolve_unchecked(None)
            .ok()
            .map(|s| s.path().to_path_buf());
        let throttle_dir = crate::paths::runtime_dir()
            .ok()
            .map(|root| root.join("refresh-throttle"));
        Self {
            enabled,
            socket,
            throttle_dir,
            pid,
        }
    }

    /// `__gpy_agent_enabled`.
    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether agent mode is on and a socket file exists (`[[ -S ... ]]`).
    /// The file outlives an agent that crashed: [`Self::send`] and
    /// [`Self::maybe_refresh`] tell whether anything listens on it.
    pub(crate) fn socket_present(&self) -> bool {
        self.enabled
            && self
                .socket
                .as_deref()
                .and_then(|s| std::fs::metadata(s).ok())
                .is_some_and(|m| m.file_type().is_socket())
    }

    /// `__gpy_agent_late_active`: a late window for this socket is open.
    fn late_active(&self) -> bool {
        let (Some(dir), Some(socket)) = (&self.throttle_dir, &self.socket) else {
            return false;
        };
        let Ok(stamp) = std::fs::read_to_string(late_window_file(dir, self.pid)) else {
            return false;
        };
        let Some((until_secs, stamped_socket)) = stamp.split_once(' ') else {
            return false;
        };
        stamped_socket == socket.to_string_lossy()
            && until_secs
                .parse::<u64>()
                .is_ok_and(|until| now_secs() < until)
    }

    /// `__gpy_agent_late_mark`: open the late window for `socket` after
    /// `request` went unanswered. A path op (git, lang, directory) can be
    /// slow for its path alone, so only the others, which the agent answers
    /// from memory, count as a wedge.
    fn mark_late(&self, request: &serde_json::Value, socket: &Path) {
        let path_op = matches!(
            request.get("op").and_then(serde_json::Value::as_str),
            Some("git" | "lang" | "directory")
        );
        if !path_op {
            self.open_late_window(socket);
        }
    }

    /// Open the late window for `socket`: requests are [`Reply::Late`] at
    /// once until it closes.
    fn open_late_window(&self, socket: &Path) {
        let Some(dir) = self.throttle_dir.as_deref() else {
            return;
        };
        let file = late_window_file(dir, self.pid);
        let stamp = format!(
            "{} {}",
            now_secs().saturating_add(LATE_SECS),
            socket.to_string_lossy()
        );
        let _ = write_private(&file, stamp.as_bytes());
    }

    /// A bounded request (`__gpy_send_json`): `request` is one JSON object.
    /// Segments send their path ops through it, and the session its
    /// `register`, `workspace`, `ping` and `unregister`. While a late window
    /// is open every request is [`Reply::Late`] at once, so a wedged agent
    /// costs one budget per window rather than one per request; a
    /// connection left without a reply line opens the window, and a reply
    /// in time closes it.
    pub(crate) fn send(&self, request: &serde_json::Value, timeout_ms: Option<u64>) -> Reply {
        if !self.socket_present() {
            return Reply::Unreachable;
        }
        if self.late_active() {
            return Reply::Late;
        }
        let Some(socket) = self.socket.as_deref() else {
            return Reply::Unreachable;
        };
        let mut stream = match connect_now(socket) {
            Connect::Open(stream) => stream,
            Connect::Refused => return Reply::Unreachable,
            // The agent is not accepting: whatever the op, it is wedged.
            Connect::Busy => {
                self.open_late_window(socket);
                return Reply::Late;
            }
        };
        let budget = Duration::from_millis(timeout_ms.unwrap_or_else(warm_timeout_ms).max(1));
        let line = format!("{request}\n");
        if stream.set_write_timeout(Some(budget)).is_err()
            || stream.write_all(line.as_bytes()).is_err()
            || stream.shutdown(std::net::Shutdown::Write).is_err()
            || stream.set_read_timeout(Some(budget)).is_err()
        {
            return Reply::Late;
        }
        let mut reply = String::new();
        match BufReader::new(stream).read_line(&mut reply) {
            Ok(_) if reply.ends_with('\n') => {
                if let Some(dir) = &self.throttle_dir {
                    clear_late_window(dir, self.pid);
                }
            }
            _ => {
                self.mark_late(request, socket);
                return Reply::Late;
            }
        }
        let decoded = reply.replace('\u{1e}', "\n");
        let text = decoded.trim_end_matches('\n');
        if text.is_empty() {
            Reply::Late
        } else if text.starts_with(r#"{"error""#) {
            Reply::Refused(text.to_owned())
        } else {
            Reply::Text(text.to_owned())
        }
    }

    /// `__gpy_maybe_refresh`: at most one refresh per 500 ms per bucket,
    /// sent without waiting for the reply. `root` is the path the shell
    /// keys the bucket by (its logical `$PWD`). `false` when no agent
    /// listens (agent mode off, no socket, or a connect refused): nothing
    /// will refresh the entry, so the caller does the work itself.
    pub(crate) fn maybe_refresh(
        &self,
        request: &serde_json::Value,
        bucket_parts: [&str; 3],
        root: &Path,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(dir) = &self.throttle_dir else {
            return self.socket_present();
        };
        let [op, suffix, extra] = bucket_parts;
        let bucket = format!(
            "{op}.{suffix}.{extra}.{}",
            InstantPromptCache::cache_key(root)
        );
        let stamp = dir.join(stamp_name(self.pid, &bucket));
        let now = now_ms();
        let last = std::fs::read_to_string(&stamp)
            .ok()
            .and_then(|s| {
                let (ms, stamped) = s.split_once(' ')?;
                (stamped == bucket)
                    .then(|| ms.parse::<u128>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        // Only a refresh that reached the agent leaves its stamp.
        if now.saturating_sub(last) < REFRESH_GAP_MS {
            return true;
        }
        let content = format!("{now} {bucket}");
        if write_private(&stamp, content.as_bytes()).is_err() {
            return self.socket_present();
        }
        let reached = self.fire(request);
        if !reached {
            let _ = std::fs::remove_file(&stamp);
        }
        reached
    }

    /// Send `request` and hang up without reading: the agent still handles
    /// it, writing the cache (and ringing a registered shell). Skipped while
    /// a late window is open, so a wedged agent's backlog is not filled.
    /// `false` when no agent listens.
    fn fire(&self, request: &serde_json::Value) -> bool {
        if !self.socket_present() {
            return false;
        }
        if self.late_active() {
            return true;
        }
        let Some(socket) = self.socket.as_deref() else {
            return false;
        };
        match connect_now(socket) {
            Connect::Open(mut stream) => {
                let _ =
                    stream.set_write_timeout(Some(Duration::from_millis(warm_timeout_ms().max(1))));
                let _ = stream.write_all(format!("{request}\n").as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                true
            }
            Connect::Busy => {
                self.open_late_window(socket);
                true
            }
            Connect::Refused => false,
        }
    }
}

/// How [`connect_now`] ended.
enum Connect {
    /// Connected (accepted or queued), in blocking mode again.
    Open(UnixStream),
    /// Nothing listening, or the socket could not be created.
    Refused,
    /// The agent's accept backlog is full: it is not accepting.
    Busy,
}

/// Connect to `socket` without waiting. A Unix-socket connect completes at
/// once while the listener's accept backlog has room; when it is full, a
/// blocking connect waits with no bound on Linux, where the non-blocking one
/// fails with `EAGAIN` instead (macOS refuses either way).
fn connect_now(socket: &Path) -> Connect {
    use nix::errno::Errno;
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect};

    let Ok(addr) = UnixAddr::new(socket) else {
        return Connect::Refused;
    };
    let Ok(fd) = nix::sys::socket::socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    ) else {
        return Connect::Refused;
    };
    let stream = UnixStream::from(fd);
    // As `UnixStream::connect` does: script children never inherit it.
    if fcntl(&stream, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).is_err()
        || stream.set_nonblocking(true).is_err()
    {
        return Connect::Refused;
    }
    match connect(stream.as_raw_fd(), &addr) {
        Ok(()) => {}
        Err(Errno::EAGAIN | Errno::EINPROGRESS) => return Connect::Busy,
        Err(_) => return Connect::Refused,
    }
    if stream.set_nonblocking(false).is_err() {
        return Connect::Refused;
    }
    Connect::Open(stream)
}

/// `<refresh-throttle>/<pid>.agent-late`: the late window's stamp,
/// `<until epoch seconds> <socket>`.
fn late_window_file(throttle_dir: &Path, pid: u32) -> PathBuf {
    throttle_dir.join(format!("{pid}.agent-late"))
}

/// `__gpy_agent_late_clear`: close shell `pid`'s late window; the agent is
/// known to be answering again.
pub(crate) fn clear_late_window(throttle_dir: &Path, pid: u32) {
    let _ = std::fs::remove_file(late_window_file(throttle_dir, pid));
}

/// Write `bytes` to `file` in the refresh-throttle directory, creating the
/// file owner-only (`0600`) and, when missing, the directory owner-only
/// (`0700`).
///
/// # Errors
/// Returns an error if the directory cannot be created or the file written.
pub(crate) fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let write = || {
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(file)?
            .write_all(bytes)
    };
    match write() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let Some(dir) = file.parent() else {
                return Err(e);
            };
            crate::cache::instant_prompt::files::create_private_dir(dir)?;
            write()
        }
        result => result,
    }
}

/// `__gpy_refresh_stamp_name`: `<pid>.<bucket>`, or `<pid>.#<fnv1a32>` past
/// 200 bytes.
fn stamp_name(pid: u32, bucket: &str) -> String {
    if bucket.len() <= 200 {
        return format!("{pid}.{bucket}");
    }
    let hash = bucket.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    });
    format!("{pid}.#{hash:08x}")
}

/// Epoch seconds.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Epoch milliseconds.
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
    use super::{Connect, Link, Reply, connect_now, late_window_file, stamp_name};
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::time::{Duration, Instant};

    /// The first file a shell writes creates the refresh-throttle directory
    /// owner-only, and the file is owner-only too (#1287).
    #[test]
    fn a_new_throttle_directory_and_its_files_are_owner_only() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("refresh-throttle");
        let link = Link {
            enabled: true,
            socket: Some(root.path().join("agent.sock")),
            throttle_dir: Some(dir.clone()),
            pid: 42,
        };
        link.open_late_window(&root.path().join("agent.sock"));
        let mode = |path: &std::path::Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&dir), 0o700, "the directory");
        assert_eq!(mode(&late_window_file(&dir, 42)), 0o600, "the stamp");
    }

    /// Long buckets hash as the shell's FNV-1a loop does, so both render
    /// paths read and write the same stamps. The expected name is what
    /// `__gpy_refresh_stamp_name` prints for 201 × `a`.
    #[test]
    fn long_bucket_stamp_matches_the_shell_hash() {
        assert_eq!(stamp_name(42, "git.git_last..x"), "42.git.git_last..x");
        assert_eq!(stamp_name(42, &"a".repeat(201)), "42.#876056a4");
    }

    /// A listener that never accepts, with every backlog slot taken: the
    /// request answers at once instead of waiting in `connect` (#1257).
    #[test]
    fn a_full_accept_backlog_never_blocks_a_request() {
        use nix::sys::socket::{
            AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, listen, socket,
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.sock");
        let listener = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .expect("socket");
        bind(
            listener.as_raw_fd(),
            &UnixAddr::new(&path).expect("address"),
        )
        .expect("bind");
        listen(&listener, Backlog::new(1_i32).expect("backlog")).expect("listen");
        let mut queued = Vec::new();
        while let Connect::Open(stream) = connect_now(&path) {
            queued.push(stream);
            assert!(queued.len() < 64, "the backlog never filled");
        }
        let link = Link {
            enabled: true,
            socket: Some(path),
            throttle_dir: Some(dir.path().join("refresh-throttle")),
            pid: 42,
        };

        let started = Instant::now();
        let reply = link.send(&serde_json::json!({"op": "status"}), Some(50));

        assert!(started.elapsed() < Duration::from_secs(2), "{reply:?}");
        // Linux reports a full backlog as `EAGAIN`; macOS refuses outright.
        let expected = if cfg!(target_os = "linux") {
            Reply::Late
        } else {
            Reply::Unreachable
        };
        assert_eq!(reply, expected);
    }
}
