//! IPC server: serves NDJSON git/language requests from shell clients over a
//! Unix socket, with connection limits, rate limiting, and per-request timeouts.

/// Builder pattern for constructing IPC server endpoints.
pub mod builder;
#[cfg(unix)]
mod connection;
/// Core IPC server endpoint implementation and request handling.
pub mod handle;
#[cfg(test)]
mod tests;

pub use builder::{EndpointDeps, EndpointHandleBuilder};
pub use handle::{EndpointHandle, WatcherRef};
