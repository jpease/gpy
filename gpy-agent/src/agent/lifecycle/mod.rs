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
pub use client::{
    ping_agent_blocking, probe_status, query_agent_status, send_config_reload_command,
    send_shutdown_command, socket_is_stale,
};
#[cfg(unix)]
pub use liveness::{inspect_socket, wait_for_agent_shutdown};
pub use outcome::{
    ProtocolMismatch, SocketCheck, StartNotice, StartOutcome, StatusProbe, protocol_mismatch,
};
#[cfg(unix)]
pub(crate) use start::StartupHandoff;
#[cfg(unix)]
pub use start::start_background_agent;
#[cfg(not(unix))]
pub use unsupported::{
    ping_agent_blocking, probe_status, query_agent_status, send_config_reload_command,
    send_shutdown_command, socket_is_stale, start_background_agent, wait_for_agent_shutdown,
};

use crate::Result;

/// Create the default runtime directory when the socket lives there (no
/// `--socket`/`GPY_AGENT_SOCKET_PATH` override). Only the start path calls this.
///
/// # Errors
/// Returns an error naming the runtime directory if it cannot be created.
pub fn ensure_runtime_dir() -> Result<()> {
    if crate::paths::socket_is_overridden() {
        return Ok(());
    }
    let path = crate::paths::runtime_dir();
    std::fs::create_dir_all(&path).map_err(|e| crate::Error::Agent {
        message: format!("cannot create runtime directory {}", path.display()),
        source: Some(Box::new(e)),
    })
}

/// How the socket's directory may be repaired, by where [`crate::paths::resolve_socket_path`]
/// takes the path from: GPY's own runtime directory is tightened, while a
/// user-chosen `--socket` or `GPY_AGENT_SOCKET_PATH` directory is never chmod'ed.
#[cfg(unix)]
#[must_use]
pub fn socket_dir_repair() -> crate::ipc::server::DirRepair {
    if crate::paths::socket_is_overridden() {
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
