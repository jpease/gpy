//! IPC server implementation for agent-side communication
//!
//! Handles incoming connections from Fish shell clients and processes
//! requests for Git status and language detection.
//!
//! ## Architecture
//!
//! The server uses Tokio's async runtime with the following components:
//!
//! - **Connection Pool** - Max 200 concurrent connections via semaphore (configurable)
//! - **Request Timeout** - 5-second default timeout per request (configurable via `agent.timeout_seconds` in config)
//! - **Rate Limiting** - 1000 connections/second global limit across all clients (configurable via [`crate::security::GuardSettings`])
//! - **Security Guards** - PID validation, path sanitization, message size limits (64KB)
//!
//! ## Communication Protocol
//!
//! The server communicates over a Unix domain socket at:
//! - `$GPY_AGENT_SOCKET_PATH` (if set)
//! - `$XDG_RUNTIME_DIR/gpy/gpy.sock` (default)
//!
//! Messages use newline-delimited JSON (NDJSON):
//!
//! ```json
//! {"op":"git","cwd":"/path/to/repo","format":"json"}
//! ```
//!
//! ## Request Lifecycle
//!
//! 1. Client connects to Unix socket
//! 2. Server validates connection (PID, rate limit)
//! 3. Client sends NDJSON request
//! 4. Server parses and validates message
//! 5. Server processes request (cache lookup or fresh query)
//! 6. Server formats response (JSON/Fish ANSI/Fish source)
//! 7. Server sends response and optionally signals client PID
//! 8. Connection closes
//!
//! ## Example Usage
//!
//! ```rust
//! use gpy_agent::ipc::server::EndpointHandle;
//! use gpy_agent::ipc::ClientDirectory;
//! use gpy_agent::git::cache::GitStatusCache;
//! use gpy_agent::config::manager::ConfigManager;
//! use gpy_agent::theme::ThemeManager;
//! use gpy_agent::ipc::LatencyTracker;
//! use gpy_agent::cache::InstantPromptCache;
//! use gpy_agent::language::DetectionCache;
//! use std::sync::Arc;
//!
//! # fn main() -> gpy_agent::Result<()> {
//! // Create all required components
//! let registry = Arc::new(ClientDirectory::new());
//! let git_cache = Arc::new(GitStatusCache::new());
//! let config_manager = Arc::new(ConfigManager::with_defaults()?);
//! let theme_manager = Arc::new(ThemeManager::new("default")?);
//! let instant_cache = Arc::new(InstantPromptCache::new()?);
//! let latency_tracker = Arc::new(LatencyTracker::new(100));
//! let language_cache = DetectionCache::new();
//!
//! // Build the server with all required fields
//! let server = EndpointHandle::builder()
//!     .client_registry(registry)
//!     .git_cache(git_cache)
//!     .config_manager(config_manager)
//!     .theme_manager(theme_manager)
//!     .instant_cache(instant_cache)
//!     .latency_tracker(latency_tracker)
//!     .language_cache(language_cache)
//!     .build()?;
//!
//! // Server now handles requests in background (via `EndpointHandle::start`,
//! // an async method not called in this synchronous doctest)
//! # Ok(())
//! # }
//! ```

/// Builder pattern for constructing IPC server endpoints.
pub mod builder;
/// Per-connection request serving, split out from `handle` (#359).
// Serving an accepted connection is Unix-only: the only constructor of
// `ConnectionHandler` is `EndpointHandle::spawn_client_handler`, which is
// itself `cfg(unix)` because the transport is a Unix domain socket (#540).
#[cfg(unix)]
mod connection;
/// Core IPC server endpoint implementation and request handling.
pub mod handle;
#[cfg(test)]
mod tests;

pub use builder::EndpointHandleBuilder;
pub use handle::{EndpointHandle, WatcherRef, WatcherSlot};
