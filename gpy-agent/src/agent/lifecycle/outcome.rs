//! Platform-independent result types of the lifecycle commands, so callers
//! (and the Windows build of the CLI) can match on them without the Unix
//! client.

use crate::ipc::AgentStatus;
use crate::ipc::protocol::PROTOCOL_VERSION;

/// What one `status` query found.
#[derive(Debug)]
pub enum StatusProbe {
    /// The agent answered with its status.
    Answered(AgentStatus),
    /// The agent closed or reset the connection without replying: it is
    /// running its accept loop but shedding load.
    Busy,
    /// Nothing is listening on the socket (connection refused, or the file is
    /// gone).
    NotListening(crate::Error),
    /// The agent accepted the connection but gave no usable reply.
    NoReply(crate::Error),
    /// The probe could not be made (for example the socket is not accessible).
    Failed(crate::Error),
}

/// A running agent whose protocol differs from this binary's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMismatch {
    /// The protocol version the running agent reports.
    pub running: u8,
    /// The protocol version this binary speaks.
    pub expected: u8,
}

/// The mismatch between the running agent's protocol and this binary's, if any.
#[must_use]
pub const fn protocol_mismatch(agent_protocol: u8) -> Option<ProtocolMismatch> {
    if agent_protocol == PROTOCOL_VERSION {
        None
    } else {
        Some(ProtocolMismatch {
            running: agent_protocol,
            expected: PROTOCOL_VERSION,
        })
    }
}

/// What `start_background_agent` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    /// Nothing was running; a daemon with this PID now answers.
    Started {
        /// The daemon's process ID.
        pid: u32,
    },
    /// A running agent of another version or protocol was replaced.
    Replaced {
        /// The version the evicted agent reported.
        previous: String,
        /// The new daemon's process ID.
        pid: u32,
    },
    /// A compatible agent already answers, or a busy one was left alone.
    AlreadyRunning,
    /// A newer agent with this binary's protocol is running and was kept.
    NewerRunning {
        /// The newer agent's version.
        running: String,
    },
    /// The socket's owner could not be verified; it was left alone and
    /// nothing was started.
    Unverifiable,
    /// `agent.enabled = false`: nothing was started.
    Disabled,
}

/// What inspecting the socket before a start established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketCheck {
    /// The socket is free (absent, stale and removed, or the running agent was
    /// evicted): start may fork. `previous` is the evicted agent's version when
    /// known.
    Proceed {
        /// The evicted agent's version, when it reported one.
        previous: Option<String>,
    },
    /// Nothing is to be started.
    Done(StartOutcome),
}

/// A progress or advisory message during `start`, for the caller to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartNotice {
    /// `agent.enabled = false`.
    Disabled,
    /// The socket's owner could not be verified and was left alone.
    Unverifiable,
    /// An agent accepts connections but never answers; it is being evicted.
    EvictingUnanswered,
    /// The running agent is older than this binary; it is being replaced.
    VersionMismatch {
        /// The running agent's version.
        running: String,
    },
    /// The running agent speaks another protocol; it is being replaced.
    ProtocolMismatch {
        /// The running agent's protocol version.
        running: u8,
    },
    /// A socket nobody answers on is being removed.
    StaleSocket,
    /// A compatible agent already answers.
    AlreadyRunning,
    /// A running agent closes connections without answering and was left alone.
    Busy,
    /// A newer agent is running and was kept.
    NewerRunning {
        /// The newer agent's version.
        running: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{PROTOCOL_VERSION, protocol_mismatch};

    #[test]
    fn protocol_mismatch_is_none_for_the_binarys_own_protocol() {
        assert_eq!(protocol_mismatch(PROTOCOL_VERSION), None);
        let mismatch = protocol_mismatch(PROTOCOL_VERSION.wrapping_add(1))
            .expect("a different protocol is a mismatch");
        assert_eq!(mismatch.expected, PROTOCOL_VERSION);
        assert_eq!(mismatch.running, PROTOCOL_VERSION.wrapping_add(1));
    }
}
