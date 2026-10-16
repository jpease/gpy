//! Shared test fixtures: git repos, temp dirs, and skip helpers.

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

/// Create a temporary directory, removed on drop.
pub fn create_temp_dir() -> TempDir {
    TempDir::new().expect("Failed to create temporary directory for test")
}

/// Path for a file in `dir` without creating it.
pub fn temp_file_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

/// Whether git is on `PATH`.
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Skip the test if git is unavailable.
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
