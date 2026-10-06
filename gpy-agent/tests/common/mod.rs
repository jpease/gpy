//! Common test utilities and fixtures
//!
//! This module provides reusable test fixtures to eliminate duplicate test setup code
//! and accelerate test development across the GPY project.

pub mod cli_harness;
pub mod fixtures;

#[allow(unused_imports)]
pub use fixtures::{
    MockConfig, ServerGuard, TempSocket, create_temp_socket_path, gpy_test_root,
    wait_for_agent_ready,
};

#[allow(unused_imports)]
pub use cli_harness::{CliCommandResult, CliTestEnv, SharedCliTestEnv};
