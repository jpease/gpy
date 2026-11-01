//! Native Windows has no agent: IPC is Unix-socket only. These stand in for
//! the Unix `client` and `start` entry points the CLI calls, with the results
//! the CLI expects when no agent can ever run.

use super::{ConfigReload, StartNotice, StartOutcome, StatusProbe};
use crate::Result;
use crate::ipc::{AgentStatus, native_windows_unsupported};
use crate::paths::SocketPath;

/// A client of an agent that cannot exist on this platform.
pub struct AgentClient;

impl AgentClient {
    /// Build a client for the agent on `socket`.
    ///
    /// # Errors
    /// Never returns an error on this platform.
    pub fn new(_socket: &SocketPath) -> Result<Self> {
        Ok(Self)
    }

    /// Whether the agent answered a ping: no daemon, always `false`.
    ///
    /// # Errors
    /// Never returns an error on this platform.
    pub fn is_answering(&self) -> Result<bool> {
        Ok(false)
    }

    /// Ask the agent to shut down.
    ///
    /// # Errors
    /// Always returns an error on this platform.
    pub fn shutdown(&self) -> Result<()> {
        Err(native_windows_unsupported())
    }

    /// Query the agent's status.
    ///
    /// # Errors
    /// Always returns an error on this platform.
    pub fn status(&self) -> Result<AgentStatus> {
        Err(native_windows_unsupported())
    }

    /// Ask the running agent to reload its configuration: there is never a
    /// running agent.
    ///
    /// # Errors
    /// Never returns an error on this platform.
    pub fn reload_config(&self) -> Result<ConfigReload> {
        Ok(ConfigReload::NotRunning)
    }

    /// Native Windows has no daemon to probe; never claims a stale socket.
    #[must_use]
    pub fn socket_is_stale(&self) -> bool {
        false
    }

    /// Always "not confirmed stopped".
    #[must_use]
    pub fn wait_for_shutdown(&self) -> bool {
        false
    }

    /// Probe the agent's status: there is no daemon, so nothing is ever
    /// listening.
    #[must_use]
    pub fn probe_status(&self) -> StatusProbe {
        StatusProbe::NotListening(native_windows_unsupported())
    }
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
