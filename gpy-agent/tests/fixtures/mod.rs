//! Test Fixtures Library
//!
//! Reusable test helpers and fixtures for gpy-agent tests.
//! This library provides utilities for creating test repositories,
//! mock configurations, and test IPC clients.

#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub mod config_helpers;
pub mod git_repo;
pub mod ipc_helpers;

pub use config_helpers::TestConfig;
pub use git_repo::TestRepo;
pub use ipc_helpers::TestIpcClient;

/// Create a temporary directory for testing that will be cleaned up automatically
pub fn create_temp_dir() -> TempDir {
    TempDir::new().expect("Failed to create temporary directory for test")
}

/// Create a temporary file path (without creating the file)
pub fn temp_file_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

/// Check if git is available on the system
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Skip test if git is not available
#[macro_export]
macro_rules! skip_if_no_git {
    () => {
        if !$crate::fixtures::git_available() {
            eprintln!("Skipping test: git not available");
            return;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_temp_dir() {
        let dir = create_temp_dir();
        assert!(dir.path().exists());
        assert!(dir.path().is_dir());
    }

    #[test]
    fn test_temp_file_path() {
        let dir = create_temp_dir();
        let path = temp_file_path(&dir, "test.txt");
        assert_eq!(path, dir.path().join("test.txt"));
        assert!(!path.exists()); // File should not be created yet
    }
}
