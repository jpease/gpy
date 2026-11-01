//! Native Windows has no agent: IPC is Unix-socket only. These stand in for
//! the Unix `client`, `liveness` and `start` entry points the CLI calls, with
//! the results the CLI expects when no agent can ever run.

use super::{AgentStatusInfo, ConfigReload, StartNotice, StartOutcome, StatusProbe};
use crate::Result;
use crate::ipc::native_windows_unsupported;
use crate::paths::SocketPath;
use std::path::Path;

/// Blocking ping: no daemon, always `false`.
///
/// # Errors
/// Never returns an error on this platform.
pub fn ping_agent_blocking(_socket_path: &Path) -> Result<bool> {
    Ok(false)
}

/// Send shutdown command to agent.
///
/// # Errors
/// Always returns an error on this platform.
pub async fn send_shutdown_command(_socket: &SocketPath) -> Result<()> {
    Err(native_windows_unsupported())
}

/// Ask the running agent to reload its configuration: there is never a
/// running agent.
///
/// # Errors
/// Never returns an error on this platform.
pub async fn send_config_reload_command(_socket: &SocketPath) -> Result<ConfigReload> {
    Ok(ConfigReload::NotRunning)
}

/// Native Windows has no daemon to probe; never claim a stale socket.
#[must_use]
pub fn socket_is_stale(_socket_path: &Path) -> bool {
    false
}

/// Query agent status.
///
/// # Errors
/// Always returns an error on this platform.
pub async fn query_agent_status(_socket: &SocketPath) -> Result<AgentStatusInfo> {
    Err(native_windows_unsupported())
}

/// Probe the agent's status: there is no daemon, so nothing is ever
/// listening.
#[must_use]
pub fn probe_status(_socket_path: &Path) -> StatusProbe {
    StatusProbe::NotListening(native_windows_unsupported())
}

/// Always "not confirmed stopped".
#[must_use]
pub fn wait_for_agent_shutdown(_socket_path: &Path) -> bool {
    false
}

/// Start the agent in background (native Windows: not supported).
///
/// # Errors
/// Always returns an error on this platform.
pub fn start_background_agent(
    _socket: &SocketPath,
    _on_notice: impl FnMut(StartNotice),
) -> Result<StartOutcome> {
    Err(native_windows_unsupported())
}
