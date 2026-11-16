//! Agent startup: diagnostics, the start announcement to tracked shells,
//! timers, and shutdown signal handlers.

use crate::debug_log;
#[cfg(unix)]
use crate::ipc::registry::{announce_start_in, default_shell_dir};
use crate::{Error, Result};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::signal;

/// Connect attempts the start announcement makes before giving up; with
/// [`ANNOUNCE_CONNECT_POLL`], a 5 s wait for the socket to accept.
#[cfg(unix)]
const ANNOUNCE_CONNECT_ATTEMPTS: u32 = 100;

/// Pause between the start announcement's connect attempts.
#[cfg(unix)]
const ANNOUNCE_CONNECT_POLL: Duration = Duration::from_millis(50);

/// Interval of the instant-prompt cache age sweep after the one at startup.
const CACHE_SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// Boxed, pinned async shutdown signal future.
pub(super) type ShutdownSignal = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Logs startup diagnostics. The real bind (and its errors) happens in
/// [`crate::ipc::server::EndpointHandle::start`]; a pre-check would race.
pub(super) fn startup_checks(socket_path: &Path) {
    debug_log!("agent", "GPY Agent starting...");
    debug_log!("agent", "Socket path: {}", socket_path.display());
    debug_log!("agent", "Starting IPC server...");
}

/// Announce a started agent to every alive tracked shell: warm the
/// theme-export cache, then let the shell-tracking protocol raise `.reload`
/// if the export changed (config edited while no agent ran) and
/// `.reregister`. Lives here, not in `ipc::registry`, so the registry
/// depends on neither `theme` nor `cache`. Call once the socket accepts.
#[cfg(unix)]
pub(super) fn announce_start_to_tracked_shells(
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
    palette: &crate::template::Palette,
) {
    announce_start_in(&default_shell_dir(), || {
        crate::cache::write_theme_export_cache(theme_manager, config, palette)
    });
}

/// The default socket when `socket` overrides it with another path, else
/// `None`. The shells tracked under the runtime dir talk to the default socket.
#[cfg(unix)]
pub(super) fn overridden_default_socket(
    socket: &crate::paths::SocketPath,
) -> Option<std::path::PathBuf> {
    if !socket.is_override() {
        return None;
    }
    let runtime_dir = crate::paths::runtime_dir();
    let default =
        crate::paths::absolutize(crate::paths::socket_path_for(None, None, &runtime_dir).path());
    (default.as_path() != socket.path()).then_some(default)
}

/// Run `announce` (see [`announce_start_to_tracked_shells`]) once
/// `socket_path` accepts. Returns whether the tracked shells belong to another
/// agent, the verdict the re-nudge timer follows.
///
/// - Nothing is announced when the socket never accepts within the poll budget
///   or `server_stopped` is set first: no shell is woken for a daemon that
///   never listened.
/// - `default_socket` is the default socket when this agent's socket is an
///   override. The shells tracked under the runtime dir talk to that socket,
///   so while another agent answers there they are left alone.
#[cfg(unix)]
pub(super) fn announce_start_when_accepting(
    socket_path: &Path,
    default_socket: Option<&Path>,
    server_stopped: &AtomicBool,
    announce: impl FnOnce(),
) -> bool {
    let foreign_if_unknown = default_socket.is_some();
    if !wait_until_accepting(socket_path, server_stopped) {
        debug_log!(
            "agent",
            "Socket {} never accepted; skipping the start announcement",
            socket_path.display()
        );
        return foreign_if_unknown;
    }
    if let Some(default) = default_socket
        && !same_file(default, socket_path)
        && std::os::unix::net::UnixStream::connect(default).is_ok()
    {
        debug_log!(
            "agent",
            "Another agent serves {}; leaving the shells tracked for it alone",
            default.display()
        );
        return true;
    }
    announce();
    false
}

/// Whether `first` and `second` name the same existing file (an override
/// spelling the default socket through a symlink reaches this agent itself).
#[cfg(unix)]
fn same_file(first: &Path, second: &Path) -> bool {
    std::fs::canonicalize(first).is_ok_and(|first_real| {
        std::fs::canonicalize(second).is_ok_and(|second_real| first_real == second_real)
    })
}

/// Poll `socket_path` until it accepts; `false` when it never does within
/// the budget or `server_stopped` is set first.
#[cfg(unix)]
fn wait_until_accepting(socket_path: &Path, server_stopped: &AtomicBool) -> bool {
    for _ in 0..ANNOUNCE_CONNECT_ATTEMPTS {
        if server_stopped.load(Ordering::Acquire) {
            return false;
        }
        if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
            return true;
        }
        std::thread::sleep(ANNOUNCE_CONNECT_POLL);
    }
    false
}

/// Creates a clock timer that ticks every second.
pub(super) fn create_clock_timer() -> tokio::time::Interval {
    let mut clock_timer = tokio::time::interval(Duration::from_secs(1));
    clock_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    clock_timer
}

/// Creates the 60 s pruning and shell re-nudge timer.
/// First tick is at +60 s: the restart-nudge task already covers t=0.
pub(super) fn create_pruning_timer() -> tokio::time::Interval {
    let mut pruning_timer = tokio::time::interval(Duration::from_mins(1));
    pruning_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    pruning_timer.reset();
    pruning_timer
}

/// Creates the instant-prompt cache sweep timer; the first tick is one
/// interval out, since `InstantPromptCache::new` sweeps at startup.
pub(super) fn create_cache_sweep_timer() -> tokio::time::Interval {
    let mut sweep_timer = tokio::time::interval(CACHE_SWEEP_INTERVAL);
    sweep_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    sweep_timer.reset();
    sweep_timer
}

/// Creates a shutdown future resolving on SIGINT or SIGTERM (what `kill` and
/// session managers send), so the watcher and socket get cleaned up.
#[cfg(unix)]
pub(super) fn create_shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .map_err(|e| Error::agent(format!("cannot install the SIGTERM handler: {e}")))?;

        tokio::select! {
            result = signal::ctrl_c() => {
                result.map_err(|e| {
                    Error::agent(format!("cannot install the signal handler: {e}"))
                })?;
            }
            _ = sigterm.recv() => {}
        }

        debug_log!("agent", "Received shutdown signal");
        Ok::<(), Error>(())
    })
}

/// Creates a shutdown future resolving on ctrl-c (no SIGTERM off Unix).
#[cfg(not(unix))]
pub(super) fn create_shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        signal::ctrl_c()
            .await
            .map_err(|e| Error::agent(format!("cannot install the signal handler: {e}")))?;
        debug_log!("agent", "Received shutdown signal");
        Ok::<(), Error>(())
    })
}

/// Whether to send a clock broadcast this tick, and the new last-notified minute.
/// With `show_seconds` off, sends only when `current_minute` changed.
/// Returns `(should_send, new_last_notified_minute)`.
#[must_use]
pub const fn clock_signal_decision(
    show_seconds: bool,
    current_minute: u64,
    last_notified_minute: u64,
) -> (bool, u64) {
    if show_seconds {
        (true, current_minute)
    } else {
        let should_send = current_minute != last_notified_minute;
        (
            should_send,
            if should_send {
                current_minute
            } else {
                last_notified_minute
            },
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "cfg(all(test, unix)) is not recognized by clippy's in-tests config"
    )]

    use super::*;
    use nix::sys::signal::{Signal, raise};
    use serial_test::serial;

    /// The restart nudge must be the only nudge at t=0.
    #[tokio::test]
    async fn pruning_timer_first_tick_is_not_immediate() {
        let mut timer = create_pruning_timer();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), timer.tick())
                .await
                .is_err()
        );
    }

    /// Only a socket overriding the default with another path names the
    /// default socket, whose shells this agent must then not assume are its own.
    #[test]
    fn only_an_overriding_socket_names_the_default_socket() {
        let runtime = crate::paths::runtime_dir();
        let default = crate::paths::socket_path_for(None, None, &runtime);
        let default_path = crate::paths::absolutize(default.path());
        let same = crate::paths::socket_path_for(Some(default_path.as_path()), None, &runtime);
        let custom = crate::paths::socket_path_for(Some(Path::new("/x/gpy.sock")), None, &runtime);

        assert_eq!(overridden_default_socket(&default), None);
        assert_eq!(overridden_default_socket(&same), None);
        assert_eq!(overridden_default_socket(&custom), Some(default_path));
    }

    /// Bind a listening socket named `name` in `dir`.
    fn listen(dir: &Path, name: &str) -> (std::path::PathBuf, std::os::unix::net::UnixListener) {
        let path = dir.join(name);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        (path, listener)
    }

    /// A server that stopped before accepting gets no announcement; an
    /// override agent then leaves the tracked shells alone.
    #[test]
    fn stopped_server_announces_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("never.sock");
        let default_path = dir.path().join("default.sock");
        let stopped = AtomicBool::new(true);
        for (default_socket, expected) in [(None, false), (Some(default_path.as_path()), true)] {
            let mut announced = false;
            let foreign = announce_start_when_accepting(&socket, default_socket, &stopped, || {
                announced = true;
            });
            assert!(!announced, "a daemon that never listened must not announce");
            assert_eq!(foreign, expected);
        }
    }

    /// An accepting socket announces unless another agent serves the default socket.
    #[test]
    fn accepting_socket_announces_unless_the_default_socket_is_served() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (socket, _listener) = listen(dir.path(), "own.sock");
        let stopped = AtomicBool::new(false);
        let idle_default = dir.path().join("idle.sock");

        for default_socket in [None, Some(idle_default.as_path())] {
            let mut announced = false;
            let foreign = announce_start_when_accepting(&socket, default_socket, &stopped, || {
                announced = true;
            });
            assert!(announced, "the start must be announced");
            assert!(!foreign);
        }

        let (served_default, _default_listener) = listen(dir.path(), "default.sock");
        let mut announced = false;
        let foreign =
            announce_start_when_accepting(&socket, Some(&served_default), &stopped, || {
                announced = true;
            });
        assert!(!announced, "the default agent's shells must be left alone");
        assert!(foreign);
    }

    /// Poll `signal` once, which installs its handlers, and confirm it pends.
    async fn arm(signal: &mut ShutdownSignal) {
        assert!(
            tokio::time::timeout(Duration::ZERO, signal).await.is_err(),
            "no shutdown signal has been raised yet"
        );
    }

    /// Safe to self-raise: `create_shutdown_signal` installs its own handler first.
    #[tokio::test]
    #[serial(sigterm_self_signal)]
    async fn create_shutdown_signal_resolves_on_sigterm() {
        let mut signal = create_shutdown_signal();
        arm(&mut signal).await;

        raise(Signal::SIGTERM).expect("raise SIGTERM");

        let result = tokio::time::timeout(Duration::from_secs(2), signal)
            .await
            .expect("shutdown signal should resolve after SIGTERM");
        assert!(
            result.is_ok(),
            "shutdown signal future should resolve Ok on SIGTERM"
        );
    }

    /// SIGINT must keep working alongside SIGTERM.
    #[tokio::test]
    #[serial(sigterm_self_signal)]
    async fn create_shutdown_signal_resolves_on_sigint() {
        let mut signal = create_shutdown_signal();
        arm(&mut signal).await;

        raise(Signal::SIGINT).expect("raise SIGINT");

        let result = tokio::time::timeout(Duration::from_secs(2), signal)
            .await
            .expect("shutdown signal should resolve after SIGINT");
        assert!(
            result.is_ok(),
            "shutdown signal future should resolve Ok on SIGINT"
        );
    }
}
