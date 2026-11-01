//! Whether a running agent is alive, wedged, busy or gone, and what `start`
//! does about each: the ping rounds that decide, the wait for a stopping
//! agent, eviction of a wedged or outdated one, and the version verdict.

use super::client::{AgentClient, PingOutcome};
use super::{SocketCheck, StartNotice, StartOutcome};
use crate::Result;
use crate::VERSION;
use crate::deadline::Deadline;
use crate::ipc::AgentStatus;
use crate::ipc::socket_identity::SocketIdentity;
use std::cmp::Ordering;
use std::path::Path;
use std::time::Duration;

/// A parsed `MAJOR.MINOR.PATCH[-prerelease]` version.
type AgentVersion<'a> = ((u64, u64, u64), Option<&'a str>);

/// Parse an agent version, ignoring surrounding whitespace.
fn parse_agent_version(raw: &str) -> Option<AgentVersion<'_>> {
    let trimmed = raw.trim();
    let (core, pre) = match trimmed.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some(_) => return None,
        None => (trimmed, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(((major, minor, patch), pre))
}

/// Order two parsed versions; a pre-release sorts before its release.
fn compare_agent_versions(left: &AgentVersion<'_>, right: &AgentVersion<'_>) -> Ordering {
    left.0.cmp(&right.0).then_with(|| match (left.1, right.1) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(left_pre), Some(right_pre)) => left_pre.cmp(right_pre),
    })
}

/// Whether a running agent should be replaced: only if older or unparsable,
/// so a corrupt marker self-heals.
fn should_replace(running: &str, binary: &str) -> bool {
    let Some(running_version) = parse_agent_version(running) else {
        return true;
    };
    parse_agent_version(binary).map_or_else(
        || running.trim() != binary.trim(),
        |binary_version| {
            compare_agent_versions(&running_version, &binary_version) == Ordering::Less
        },
    )
}

/// Ping attempts before an existing socket is declared stale. A busy live agent
/// can miss one ping; unlinking its socket would orphan it. Refused
/// connects fail in microseconds, so retries are cheap for leftover sockets.
const PING_STALE_ATTEMPTS: u32 = 3;

/// Backoff slept between failed ping attempts.
const PING_STALE_BACKOFF: Duration = Duration::from_millis(200);

/// Max wait for a shutdown-signalled agent to stop responding before unlinking
/// its socket. Ownership-verified cleanup in the server protects the new socket
/// if the old agent exits late.
const SHUTDOWN_WAIT_MAX: Duration = Duration::from_secs(3);

/// Poll interval while waiting for the old agent to stop responding.
const SHUTDOWN_WAIT_POLL: Duration = Duration::from_millis(100);

/// What a round of pings established. Reading "never managed to ask" as death
/// would unlink a live agent's socket and fork a duplicate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AgentLiveness {
    /// An attempt was answered. The agent is alive.
    Responding,
    /// Every attempt reached the socket unanswered; the only stale verdict.
    NotResponding,
    /// Every attempt connected but got no valid reply: alive but wedged.
    /// Justifies eviction, not silent unlink.
    AcceptedNoReply,
    /// Every attempt that reached the socket was closed or reset without a
    /// reply: the accept loop runs but the agent sheds load. Not wedged.
    Busy,
    /// A ping failed locally; says nothing about the agent.
    Indeterminate,
}

/// Ping the agent, retrying before concluding anything.
fn ping_agent_liveness(client: &AgentClient) -> AgentLiveness {
    ping_agent_liveness_with(|| client.ping(), PING_STALE_BACKOFF)
}

/// [`ping_agent_liveness`] over an arbitrary ping, for tests. Any local
/// failure makes the round indeterminate: erring that way costs a retry,
/// the other way a running daemon.
fn ping_agent_liveness_with<F>(mut ping: F, backoff: std::time::Duration) -> AgentLiveness
where
    F: FnMut() -> Result<PingOutcome>,
{
    let mut local_failure = false;
    let mut saw_not_listening = false;
    let mut saw_accepted_no_reply = false;
    let mut saw_closed_unanswered = false;

    for attempt in 0..PING_STALE_ATTEMPTS {
        match ping() {
            Ok(PingOutcome::Answered) => return AgentLiveness::Responding,
            Ok(PingOutcome::NotListening) => saw_not_listening = true,
            Ok(PingOutcome::AcceptedNoReply) => saw_accepted_no_reply = true,
            Ok(PingOutcome::ClosedUnanswered) => saw_closed_unanswered = true,
            Err(_) => local_failure = true,
        }
        if attempt
            .checked_add(1)
            .is_some_and(|next| next < PING_STALE_ATTEMPTS)
        {
            std::thread::sleep(backoff);
        }
    }

    if local_failure {
        AgentLiveness::Indeterminate
    } else if saw_closed_unanswered && !saw_not_listening {
        // A dead agent's pending connections are reset once, then connects
        // are refused, so a refusal in the round denies "busy".
        AgentLiveness::Busy
    } else if saw_accepted_no_reply && !saw_not_listening {
        // A mixed round may be a dead agent replaced mid-round.
        AgentLiveness::AcceptedNoReply
    } else {
        AgentLiveness::NotResponding
    }
}

/// Poll until the agent stops responding to pings, bounded by `SHUTDOWN_WAIT_MAX`.
/// Returns `false` if still answering (or only indeterminate) when the budget ends.
#[must_use]
pub(super) fn wait_for_agent_shutdown(client: &AgentClient) -> bool {
    let deadline = Deadline::after(SHUTDOWN_WAIT_MAX);
    loop {
        if ping_agent_liveness(client) == AgentLiveness::NotResponding {
            return true; // Reached the socket every time, never answered: confirmed stopped.
        }
        if deadline.expired() {
            return false; // Bounded fallback: still responding or unverifiable.
        }
        std::thread::sleep(SHUTDOWN_WAIT_POLL);
    }
}

/// What the identity-aware wait could establish about an evicted agent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShutdownWait {
    /// The path is gone or holds a different socket, which is not ours to remove.
    Released,
    /// The captured socket is still at the path and no longer answers.
    Stopped,
    /// The captured socket still answers (or cannot be verified) at the deadline.
    TimedOut,
}

/// Whether the socket captured as `identity` has left `socket_path`: the path
/// is missing or holds a different inode. Any other `stat` failure is not
/// evidence of release.
fn socket_released(socket_path: &Path, identity: SocketIdentity) -> bool {
    match SocketIdentity::at(socket_path) {
        Ok(current) => current != identity,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Wait until the socket captured as `identity` stops answering or leaves the
/// path, bounded by `SHUTDOWN_WAIT_MAX`. A replacement's answers end the wait.
pub(crate) fn wait_for_socket_release(
    client: &AgentClient,
    identity: SocketIdentity,
) -> ShutdownWait {
    let socket_path = client.socket();
    let deadline = Deadline::after(SHUTDOWN_WAIT_MAX);
    loop {
        if socket_released(socket_path, identity) {
            return ShutdownWait::Released;
        }
        if ping_agent_liveness(client) == AgentLiveness::NotResponding {
            // The round may have reached a replacement bound mid-round.
            return if socket_released(socket_path, identity) {
                ShutdownWait::Released
            } else {
                ShutdownWait::Stopped
            };
        }
        if deadline.expired() {
            return ShutdownWait::TimedOut;
        }
        std::thread::sleep(SHUTDOWN_WAIT_POLL);
    }
}

/// Remove the socket only while the path still holds the captured socket (the
/// check-then-unlink window is inherent).
fn remove_socket_if_unchanged(socket_path: &Path, identity: SocketIdentity) {
    if socket_released(socket_path, identity) {
        return;
    }
    let _ = std::fs::remove_file(socket_path);
}

/// Evict an unanswering or incompatible agent: request shutdown, wait for
/// release, then unlink only the captured socket. `previous` is the evicted
/// agent's version when known. Returns `Done(AlreadyRunning)` when a
/// responsive replacement already holds the path, so the caller must not fork
/// another.
fn evict_agent(
    client: &AgentClient,
    identity: SocketIdentity,
    previous: Option<String>,
    on_notice: &mut dyn FnMut(StartNotice),
) -> SocketCheck {
    let socket_path = client.socket();
    // Best-effort graceful shutdown.
    let _ = client.shutdown();

    // Proceed either way, but only ever unlink the captured socket.
    if wait_for_socket_release(client, identity) != ShutdownWait::Released {
        remove_socket_if_unchanged(socket_path, identity);
        return SocketCheck::Proceed { previous };
    }

    // A different, answering agent is the replacement: do not fork another.
    if SocketIdentity::at(socket_path).is_ok() && matches!(client.ping(), Ok(PingOutcome::Answered))
    {
        on_notice(StartNotice::AlreadyRunning);
        return SocketCheck::Done(StartOutcome::AlreadyRunning);
    }
    SocketCheck::Proceed { previous }
}

/// What `start` does about a responding agent, from its own status reply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// Same version and protocol: leave it.
    Keep,
    /// A newer version on this binary's protocol: leave it.
    KeepNewer,
    /// An older or unparsable version: replace it.
    ReplaceOlder,
    /// Another protocol, whatever its version: replace it.
    ReplaceProtocol,
}

/// Decide from the running agent's status: protocol first, then version.
fn verdict(running: &AgentStatus) -> Verdict {
    if running.protocol_version != crate::ipc::protocol::PROTOCOL_VERSION {
        Verdict::ReplaceProtocol
    } else if running.version.trim() == VERSION {
        Verdict::Keep
    } else if should_replace(&running.version, VERSION) {
        Verdict::ReplaceOlder
    } else {
        Verdict::KeepNewer
    }
}

/// Decide what to do about an agent that answered pings.
fn inspect_responding(
    client: &AgentClient,
    identity: SocketIdentity,
    on_notice: &mut dyn FnMut(StartNotice),
) -> SocketCheck {
    // A failed query after a successful ping cannot be verified; leave the
    // agent alone rather than fork a duplicate.
    let Ok(status) = client.status() else {
        on_notice(StartNotice::Unverifiable);
        return SocketCheck::Done(StartOutcome::Unverifiable);
    };
    match verdict(&status) {
        Verdict::Keep => {
            on_notice(StartNotice::AlreadyRunning);
            SocketCheck::Done(StartOutcome::AlreadyRunning)
        }
        Verdict::KeepNewer => {
            on_notice(StartNotice::NewerRunning {
                running: status.version.clone(),
            });
            SocketCheck::Done(StartOutcome::NewerRunning {
                running: status.version,
            })
        }
        Verdict::ReplaceOlder => {
            on_notice(StartNotice::VersionMismatch {
                running: status.version.clone(),
            });
            evict_agent(client, identity, Some(status.version), on_notice)
        }
        Verdict::ReplaceProtocol => {
            on_notice(StartNotice::ProtocolMismatch {
                running: status.protocol_version,
            });
            evict_agent(client, identity, Some(status.version), on_notice)
        }
    }
}

/// Inspect the socket before a start and clean up a stale one. `Proceed`
/// means the socket is free and start may fork.
#[must_use]
pub fn inspect_socket(client: &AgentClient, on_notice: &mut dyn FnMut(StartNotice)) -> SocketCheck {
    let socket_path = client.socket();
    // Every later wait and unlink concerns this socket, never a replacement.
    let Ok(identity) = SocketIdentity::at(socket_path) else {
        return SocketCheck::Proceed { previous: None };
    };

    match ping_agent_liveness(client) {
        // Wrongly treating a live agent as stale forks a duplicate; wrongly
        // treating a dead one as live costs one prompt. So leave an
        // unverifiable socket alone.
        AgentLiveness::Indeterminate => {
            on_notice(StartNotice::Unverifiable);
            SocketCheck::Done(StartOutcome::Unverifiable)
        }
        // Closing connections unanswered is load shedding by a live agent.
        AgentLiveness::Busy => {
            on_notice(StartNotice::Busy);
            SocketCheck::Done(StartOutcome::AlreadyRunning)
        }
        // Alive but wedged: evict rather than silently unlink.
        AgentLiveness::AcceptedNoReply => {
            on_notice(StartNotice::EvictingUnanswered);
            evict_agent(client, identity, None, on_notice)
        }
        AgentLiveness::Responding => inspect_responding(client, identity, on_notice),
        // Unresponsive across every retry; unlink unless already replaced.
        AgentLiveness::NotResponding => {
            on_notice(StartNotice::StaleSocket);
            remove_socket_if_unchanged(socket_path, identity);
            SocketCheck::Proceed { previous: None }
        }
    }
}

#[cfg(test)]
mod version_order_tests {
    use super::{AgentStatus, Verdict, should_replace, verdict};
    use crate::VERSION;
    use crate::ipc::protocol::PROTOCOL_VERSION;

    /// # Panics
    /// Panics when a case's verdict differs from the expected one.
    #[test]
    fn only_older_or_unparsable_running_versions_are_replaced() {
        let cases = [
            ("0.0.9", "0.1.0", true),
            ("9.9.9", "0.1.0", false),
            ("0.1.0", "0.1.0", false),
            ("0.1.0-rc1", "0.1.0", true),
            ("0.1.0", "0.1.0-rc1", false),
            ("0.0.0-old", "0.1.0", true),
            ("garbage", "0.1.0", true),
            ("0.1", "0.1.0", true),
            ("0.1.0-", "0.1.0", true),
            (" 0.1.0\n", "0.1.0", false),
            ("0.10.0", "0.9.0", false),
        ];
        for (running, binary, expected) in cases {
            assert_eq!(
                should_replace(running, binary),
                expected,
                "should_replace({running:?}, {binary:?})"
            );
        }
    }

    /// # Panics
    /// Panics when a case's verdict differs from the expected one.
    #[test]
    fn verdict_covers_protocol_and_version() {
        let other_protocol = PROTOCOL_VERSION.wrapping_add(1);
        let cases = [
            (VERSION, PROTOCOL_VERSION, Verdict::Keep),
            ("0.0.0-old", PROTOCOL_VERSION, Verdict::ReplaceOlder),
            ("garbage", PROTOCOL_VERSION, Verdict::ReplaceOlder),
            ("999.0.0", PROTOCOL_VERSION, Verdict::KeepNewer),
            (VERSION, other_protocol, Verdict::ReplaceProtocol),
            ("0.0.0-old", other_protocol, Verdict::ReplaceProtocol),
            ("999.0.0", other_protocol, Verdict::ReplaceProtocol),
        ];
        for (version, protocol_version, expected) in cases {
            let status = AgentStatus {
                version: version.to_owned(),
                protocol_version,
                watched_repos: 0,
                registered_clients: 0,
                cache_entries: 0,
                pid: None,
            };
            assert_eq!(
                verdict(&status),
                expected,
                "version {version:?}, protocol {protocol_version}"
            );
        }
    }
}

#[cfg(test)]
#[path = "liveness_tests.rs"]
mod tests;
