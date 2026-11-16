//! IPC server: serves NDJSON git/language requests from shell clients over a
//! Unix socket, with connection limits, rate limiting, and per-read/per-write
//! timeouts (handler execution itself is not cancelled).

#[cfg(unix)]
mod connection;
/// Core IPC server endpoint implementation and request handling.
pub mod handle;
#[cfg(unix)]
pub(crate) mod socket_dir;
#[cfg(test)]
mod tests;

pub use handle::{EndpointDeps, EndpointHandle, WatcherRef};
#[cfg(unix)]
pub use socket_dir::DirRepair;
