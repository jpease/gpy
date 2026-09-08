//! Configuration Test Helpers
//!
//! Utilities for creating and manipulating test configurations.
#![allow(dead_code)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::expect_used)]
#![allow(clippy::str_to_string)]
#![allow(clippy::missing_const_for_fn)]

use gpy_agent::config::Config;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Test configuration wrapper with automatic cleanup
pub struct TestConfig {
    dir: TempDir,
    config_path: PathBuf,
    config: Config,
}

impl TestConfig {
    /// Create a new test configuration with defaults
    pub fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory for config");
        let config_path = dir.path().join("config.toml");
        let config = Config::default();

        Self {
            dir,
            config_path,
            config,
        }
    }

    /// Get the configuration path
    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// Get a reference to the configuration
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get a mutable reference to the configuration
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Write the configuration to disk
    pub fn write(&self) -> &Self {
        let toml_string =
            toml::to_string(&self.config).expect("Failed to serialize config to TOML");
        fs::write(&self.config_path, toml_string).expect("Failed to write config file");
        self
    }

    /// Create a configuration with git disabled
    #[must_use]
    pub fn with_git_disabled(mut self) -> Self {
        self.config.git.enabled = false;
        self
    }

    /// Create a configuration with language detection disabled
    #[must_use]
    pub fn with_language_disabled(mut self) -> Self {
        self.config.language.enabled = false;
        self
    }

    /// Create a configuration with live updates disabled
    #[must_use]
    pub fn with_live_updates_disabled(mut self) -> Self {
        self.config.agent.live_updates = false;
        self
    }

    /// Create a configuration with a custom timeout
    #[must_use]
    pub fn with_git_timeout(mut self, seconds: u64) -> Self {
        self.config.git.timeout_seconds =
            gpy_agent::config::types::GitTimeout::new(seconds).expect("Valid git timeout");
        self
    }

    /// Create a configuration with custom skip paths
    #[must_use]
    pub fn with_skip_paths(mut self, paths: Vec<String>) -> Self {
        self.config.git.skip_paths = paths;
        self
    }

    /// Create a configuration with a custom theme
    #[must_use]
    pub fn with_theme(mut self, theme: &str) -> Self {
        self.config.ui.theme =
            gpy_agent::config::types::ThemeName::new(theme.to_string()).expect("Valid theme name");
        self
    }

    /// Read configuration from disk
    pub fn read_from_disk() -> Result<Config, String> {
        // This would normally read from the actual config path
        // For tests, we just return a default
        Ok(Config::default())
    }
}

impl Default for TestConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a minimal valid TOML configuration string
pub fn minimal_config_toml() -> String {
    r#"
[agent]
enabled = true
timeout_seconds = 5
live_updates = true

[git]
enabled = true
show_upstream = true
timeout_seconds = 10
skip_paths = []

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []

[ui]
show_icons = true
theme = "default"
directory.max_length = 80
"#
    .to_string()
}

/// Create a TOML configuration with invalid syntax
pub fn invalid_config_toml() -> String {
    r"
[agent
enabled = true  # Missing closing bracket
"
    .to_string()
}

/// Create a TOML configuration with missing required fields
pub fn incomplete_config_toml() -> String {
    r"
[agent]
enabled = true
# Missing other required fields
"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_test_config() {
        let test_config = TestConfig::new();
        assert!(test_config.path().to_str().unwrap().contains("config.toml"));
    }

    #[test]
    fn test_with_git_disabled() {
        let test_config = TestConfig::new().with_git_disabled();
        assert!(!test_config.config().git.enabled);
    }

    #[test]
    fn test_with_language_disabled() {
        let test_config = TestConfig::new().with_language_disabled();
        assert!(!test_config.config().language.enabled);
    }

    #[test]
    fn test_with_git_timeout() {
        let test_config = TestConfig::new().with_git_timeout(20);
        assert_eq!(test_config.config().git.timeout_seconds.get(), 20);
    }

    #[test]
    fn test_write_config() {
        let test_config = TestConfig::new().with_git_disabled();
        test_config.write();
        assert!(test_config.path().exists());

        let contents = fs::read_to_string(test_config.path()).unwrap();
        assert!(contents.contains("[git]"));
        assert!(contents.contains("enabled = false"));
    }

    #[test]
    fn test_minimal_config_toml() {
        let toml = minimal_config_toml();
        assert!(toml.contains("[agent]"));
        assert!(toml.contains("[git]"));
        assert!(toml.contains("[language]"));
    }
}
