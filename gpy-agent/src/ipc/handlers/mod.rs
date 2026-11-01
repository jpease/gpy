//! IPC request handlers, one per domain (git, language, client, misc), and the
//! registry that routes each [`Message`] to a typed handler method.

use crate::ipc::{ClientDirectory, Message, Response};
use crate::warn_log;
use std::path::Path;
use std::sync::Arc;

/// Why a handler refused or failed a request.
///
/// Stringified into the wire-visible `Response::Error { message }`, so each
/// `Display` arm's text is part of the IPC payload.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// A segment this request needs is turned off in config.
    #[error("{0} segment disabled via config")]
    FeatureDisabled(&'static str),

    /// The request's path is not inside a git repository.
    #[error("Not in a git repository")]
    NotInRepository,

    /// The named operation exceeded its configured timeout.
    #[error("{0} timed out")]
    TimedOut(&'static str),

    /// The request named a PID that never registered.
    #[error("PID {0} not registered - please register first")]
    PidNotRegistered(u32),

    /// A git status computation failed (including a panicked background job).
    #[error("Git error: {0}")]
    Git(String),

    /// A named sub-operation of the request failed.
    #[error("{label} failed: {source}")]
    OperationFailed {
        /// The step that failed, e.g. "Config reload".
        label: &'static str,
        /// The upstream failure.
        source: crate::Error,
    },
}

/// Shared handles needed to write the instant-prompt cache and repaint live
/// shells on a real content change.
#[derive(Clone)]
pub struct RenderDeps {
    /// Live configuration, re-read on every publish to pick up hot reloads.
    pub config_manager: Arc<crate::config::manager::ConfigManager>,
    /// Active theme, shared with the agent event loop.
    pub theme_manager: Arc<crate::theme::ThemeManager>,
    /// Cached active palette, shared with the agent event loop.
    pub palette_cache: Arc<crate::palette::PaletteCache>,
    /// On-disk cache the shell reads without an IPC round trip.
    pub instant_cache: Arc<crate::cache::InstantPromptCache>,
    /// Live shells to repaint (SIGURG) when output changes.
    pub client_registry: Arc<ClientDirectory>,
}

/// Repaint live shells when `write_result` reports changed output; log (never
/// fail) on error. The change gate keeps no-op refreshes from waking terminals.
pub(crate) fn notify_if_changed(
    client_registry: &ClientDirectory,
    root: &Path,
    write_result: crate::Result<bool>,
    context: &str,
) {
    match write_result {
        Ok(true) => client_registry.notify_repaint_force(Some(root)),
        Ok(false) => {}
        // The instant cache is an optimization; don't fail the request.
        Err(e) => warn_log!(
            "cache",
            "Failed to write {context} instant-prompt cache: {e}"
        ),
    }
}

/// Run `job` off the calling thread: a blocking task on a tokio runtime, else
/// a plain OS thread.
pub(crate) fn spawn_detached<F>(job: F)
where
    F: FnOnce() + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(job);
    } else {
        std::thread::spawn(job);
    }
}

/// Registry of all request handlers.
pub struct HandlerRegistry {
    git: Arc<GitHandler>,
    language: Arc<LanguageHandler>,
    client: Arc<ClientHandler>,
}

impl HandlerRegistry {
    /// Create a registry with all handlers.
    #[must_use]
    pub const fn new(
        git: Arc<GitHandler>,
        language: Arc<LanguageHandler>,
        client: Arc<ClientHandler>,
    ) -> Self {
        Self {
            git,
            language,
            client,
        }
    }

    /// Route a message to the handler method for its variant.
    ///
    /// `Ping` and `Shutdown` are acknowledged here; `connection.rs` stops the
    /// server after the `Shutdown` reply is written.
    ///
    /// # Errors
    /// Fails if the handler fails to process the request.
    pub fn route(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::RepositoryStatus { path, segment } => {
                self.git.status(path, segment.prev_bg.as_deref())
            }
            Message::LanguageDetect {
                path,
                segment,
                virtual_env,
            } => self.language.detect(
                path,
                segment.is_last,
                segment.prev_bg.as_deref(),
                virtual_env.as_deref(),
            ),
            Message::DirectoryRequest {
                path, display_path, ..
            } => Ok(misc_handler::directory(path, display_path)),
            Message::ClockRequest { shell, .. } => Ok(misc_handler::clock(*shell)),
            Message::DurationRequest { duration_ms, .. } => {
                Ok(misc_handler::duration(*duration_ms))
            }
            Message::CharacterRequest { success, .. } => Ok(misc_handler::character(*success)),
            Message::ExitStatusRequest { success, .. } => Ok(misc_handler::exit_status(*success)),
            Message::RegisterClient {
                pid,
                cwd,
                shell,
                shell_version,
            } => Ok(self
                .client
                .register(*pid, cwd.clone(), *shell, shell_version.as_ref())),
            Message::UnregisterClient { pid } => Ok(self.client.unregister(*pid)),
            Message::WorkspaceUpdate { pid, cwd } => self.client.workspace_update(*pid, cwd),
            Message::Status => Ok(self.client.status()),
            Message::ConfigReload => self.client.config_reload(),
            Message::Ping | Message::Shutdown => Ok(Response::Ack),
        }
    }
}

/// Client lifecycle handler.
pub mod client_handler;
/// Git status handler.
pub mod git_handler;
/// Language detection handler.
pub mod language_handler;
/// Small self-contained segments (directory, clock, duration, character, exit status).
pub mod misc_handler;

pub use client_handler::ClientHandler;
pub use git_handler::GitHandler;
pub use language_handler::LanguageHandler;

#[cfg(test)]
mod tests {
    use super::HandlerError;

    #[test]
    fn operation_failed_names_the_step_and_the_cause() {
        let err = HandlerError::OperationFailed {
            label: "Config reload",
            source: crate::Error::config("missing field"),
        };

        assert_eq!(
            err.to_string(),
            "Config reload failed: Configuration error: missing field"
        );
    }
}
