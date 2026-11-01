//! Agent daemon lifecycle: runtime paths, socket discovery, compatibility
//! checks, and Unix daemon startup.
//!
//! This module wires the concerns together; each has its own file: `outcome`
//! (result types every platform shares), `client` (requests to a running
//! agent), `liveness` (is it alive, and what to do about it), `start` (the
//! daemon fork) and, off Unix, `unsupported`.

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod liveness;
mod outcome;
#[cfg(unix)]
mod start;
#[cfg(not(unix))]
mod unsupported;

#[cfg(unix)]
pub use client::AgentClient;
#[cfg(unix)]
pub use liveness::inspect_socket;
pub use outcome::{
    ProtocolMismatch, SocketCheck, StartNotice, StartOutcome, StatusProbe, protocol_mismatch,
};
#[cfg(unix)]
pub(crate) use start::StartupHandoff;
#[cfg(unix)]
pub use start::start_background_agent;
#[cfg(not(unix))]
pub use unsupported::{AgentClient, start_background_agent};

use crate::Result;
use crate::paths::SocketPath;

/// Create the default runtime directory when `socket` lives there (no
/// `--socket`/`GPY_AGENT_SOCKET_PATH` override). Only the start path calls this.
///
/// # Errors
/// Returns an error naming the runtime directory if it cannot be created.
pub fn ensure_runtime_dir(socket: &SocketPath) -> Result<()> {
    if socket.is_override() {
        return Ok(());
    }
    let path = crate::paths::runtime_dir();
    std::fs::create_dir_all(&path).map_err(|e| crate::Error::Agent {
        message: format!("cannot create runtime directory {}", path.display()),
        source: Some(Box::new(e)),
    })
}

/// How the directory of `socket` may be repaired, by where [`SocketPath`] took
/// the path from: GPY's own runtime directory is tightened, while a
/// user-chosen `--socket` or `GPY_AGENT_SOCKET_PATH` directory is never chmod'ed.
#[cfg(unix)]
#[must_use]
pub const fn socket_dir_repair(socket: &SocketPath) -> crate::ipc::server::DirRepair {
    if socket.is_override() {
        crate::ipc::server::DirRepair::Refuse
    } else {
        crate::ipc::server::DirRepair::Tighten
    }
}

/// What became of a config reload request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigReload {
    /// The agent loaded and applied the configuration.
    Applied,
    /// No agent is listening, so the configuration applies at the next start.
    NotRunning,
    /// The agent refused the configuration and kept its previous one; holds
    /// the agent's reason.
    Rejected(String),
}

/// Agent status information from IPC query.
#[derive(Debug)]
pub struct AgentStatusInfo {
    /// The version of the running agent.
    pub version: String,
    /// The protocol version the agent is using.
    pub protocol_version: u8,
    /// The number of repositories currently being watched by the agent.
    pub watched_repos: usize,
    /// The number of clients currently registered with the agent.
    pub registered_clients: usize,
    /// The number of entries in the agent's cache.
    pub cache_entries: usize,
    /// The agent's process ID, when it reports one.
    pub pid: Option<u32>,
}
