//! Talking to the agent from `gpy prompt`, as `bash/core/ipc.bash` does:
//! bounded data requests, fire-and-forget refreshes behind the shared
//! refresh-throttle stamps, and the per-shell late window. The stamp and
//! window files keep the shell's names, so both render paths share them.

use crate::cache::InstantPromptCache;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::FileTypeExt as _;
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
    Refused,
}

/// Minimum gap between two refreshes of one bucket.
const REFRESH_GAP_MS: u128 = 500;

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
        let Ok(stamp) = std::fs::read_to_string(dir.join(format!("{}.agent-late", self.pid)))
        else {
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

    /// A bounded request (`__gpy_send_json`): `request` is one JSON object.
    /// Only path ops are sent from here, so a late reply never opens the
    /// late window (`__gpy_agent_late_mark` skips git/lang/directory).
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
        let Ok(mut stream) = UnixStream::connect(socket) else {
            return Reply::Unreachable;
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
            Ok(_) if reply.ends_with('\n') => {}
            _ => return Reply::Late,
        }
        let decoded = reply.replace('\u{1e}', "\n");
        let text = decoded.trim_end_matches('\n');
        if text.is_empty() {
            Reply::Late
        } else if text.starts_with(r#"{"error""#) {
            Reply::Refused
        } else {
            Reply::Text(text.to_owned())
        }
    }

    /// `__gpy_maybe_refresh`: at most one refresh per 500 ms per bucket,
    /// sent without waiting for the reply. `root` is the path the shell
    /// keys the bucket by (its logical `$PWD`).
    pub(crate) fn maybe_refresh(
        &self,
        request: &serde_json::Value,
        bucket_parts: [&str; 3],
        root: &Path,
    ) {
        if !self.enabled {
            return;
        }
        let Some(dir) = &self.throttle_dir else {
            return;
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
        if now.saturating_sub(last) < REFRESH_GAP_MS {
            return;
        }
        let content = format!("{now} {bucket}");
        if std::fs::write(&stamp, &content).is_err()
            && (std::fs::create_dir_all(dir).is_err() || std::fs::write(&stamp, &content).is_err())
        {
            return;
        }
        self.fire(request);
    }

    /// Send `request` and hang up without reading: the agent still handles
    /// it, writing the cache (and ringing a registered shell).
    fn fire(&self, request: &serde_json::Value) {
        if !self.socket_present() {
            return;
        }
        let Some(socket) = self.socket.as_deref() else {
            return;
        };
        if let Ok(mut stream) = UnixStream::connect(socket) {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(warm_timeout_ms().max(1))));
            let _ = stream.write_all(format!("{request}\n").as_bytes());
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
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
    use super::stamp_name;

    /// Long buckets hash as the shell's FNV-1a loop does, so both render
    /// paths read and write the same stamps. The expected name is what
    /// `__gpy_refresh_stamp_name` prints for 201 × `a`.
    #[test]
    fn long_bucket_stamp_matches_the_shell_hash() {
        assert_eq!(stamp_name(42, "git.git_last..x"), "42.git.git_last..x");
        assert_eq!(stamp_name(42, &"a".repeat(201)), "42.#876056a4");
    }
}
