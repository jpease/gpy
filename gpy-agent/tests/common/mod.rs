//! Shared test helpers for the integration-test binaries that `mod common;`.
//!
//! Binaries that only need one helper include that file directly with
//! `#[path = "common/<file>.rs"] mod <file>;` instead.

pub mod cli_harness;
pub mod endpoint;
pub mod env_guard;
pub mod fixtures;
pub mod git;
pub mod wait;

#[allow(unused_imports)]
pub use fixtures::{
    MockConfig, ServerGuard, TempSocket, create_temp_socket_path, gpy_test_root,
    wait_for_agent_ready,
};

#[allow(unused_imports)]
pub use cli_harness::{CliCommandResult, CliTestEnv, SharedCliTestEnv};

#[allow(unused_imports)]
pub use endpoint::{endpoint_builder, wait_for_socket};

#[allow(unused_imports)]
pub use env_guard::EnvGuard;

#[allow(unused_imports)]
pub use wait::{wait_until, wait_until_async};
