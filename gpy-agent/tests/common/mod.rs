//! Shared test helpers for the integration-test binaries that `mod common;`.
//!
//! Binaries that only need one self-contained helper include that file
//! directly with `#[path = "common/<file>.rs"] mod <file>;` instead.

#![allow(
    dead_code,
    reason = "every binary that declares `mod common;` compiles the whole tree and uses part of it"
)]

#[cfg(unix)]
pub mod agent_sandbox;
pub mod child_guard;
pub mod cli_harness;
pub mod endpoint;
pub mod env_guard;
#[cfg(unix)]
pub mod fake_agent;
pub mod fixtures;
pub mod git;
#[cfg(unix)]
pub mod integration_harness;
#[cfg(unix)]
pub mod pty;
pub mod sgr;
pub mod sigurg;
pub mod skip;
pub mod wait;
