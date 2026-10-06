//! Comprehensive TOML configuration loading and parsing tests
//! Tests the critical config loading pipeline for semver safety

#![allow(clippy::unwrap_used)]
#![allow(clippy::bool_comparison)]
#![allow(clippy::single_match_else)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::loader::{load_config_from_file, parse_config_toml, save_config};
use gpy_agent::config::schema::{get_config_paths, get_example_config};
use gpy_agent::config::validation::validate_config;
use gpy_agent::config::{Config, types};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_config_default_enabled() {
    let config = Config::default();
    assert!(config.language.enabled);
}

#[test]
fn test_parse_empty_toml() {
    let result = parse_config_toml("");
    assert!(result.is_ok());

    let config = result.unwrap();
    // Should use defaults when empty
    assert!(config.agent.enabled);
    assert!(config.language.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 5);
}

#[test]
fn test_parse_example_config() {
    let example_toml = get_example_config();
    let result = parse_config_toml(example_toml);
    assert!(result.is_ok());
}

#[test]
fn test_example_config_agent_values() {
    let config = parse_config_toml(get_example_config()).unwrap();
    assert!(config.agent.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 5);
    assert!(config.agent.live_updates);
    assert!(config.agent.supervisor.enabled);
    assert_eq!(config.agent.supervisor.check_interval_seconds.get(), 30);
    assert_eq!(config.agent.supervisor.max_restart_attempts.get(), 5);
}

#[test]
fn test_example_config_git_values() {
    let config = parse_config_toml(get_example_config()).unwrap();
    assert!(config.git.enabled);
    assert!(config.git.show_upstream);
    assert_eq!(config.git.timeout_seconds.get(), 10);
}

#[test]
fn test_example_config_language_values() {
    let config = parse_config_toml(get_example_config()).unwrap();
    assert!(config.language.enabled);
    assert!(config.language.show_versions);
    assert_eq!(config.language.cache_ttl_hours.get(), 24);
    assert!(config.language.enabled_languages.is_empty());
}

#[test]
fn test_example_config_ui_values() {
    let config = parse_config_toml(get_example_config()).unwrap();
    assert!(config.ui.show_icons);
    assert_eq!(config.ui.theme.as_str(), "default");
    assert_eq!(
        config.ui.directory.display,
        types::DirectoryDisplay::Basename
    );
    assert_eq!(config.ui.directory.max_length.get(), 80);
}

#[test]
fn test_parse_directory_display_mode() {
    let toml = r#"
[ui.directory]
display = "full"
"#;
    let config = parse_config_toml(toml).unwrap();
    assert_eq!(config.ui.directory.display, types::DirectoryDisplay::Full);
}

#[test]
fn test_config_validation_success() {
    let config = Config::default();
    assert!(validate_config(&config).is_ok());
}

#[test]
fn test_parse_agent_timeout_zero() {
    let toml = r"
[agent]
timeout_seconds = 0
";
    let result = parse_config_toml(toml);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("agent timeout must be between 1 and 300")
    );
}

#[test]
fn test_parse_agent_timeout_too_large() {
    let toml = r"
[agent]
timeout_seconds = 500
";
    let result = parse_config_toml(toml);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("agent timeout must be between 1 and 300")
    );
}

#[test]
fn test_parse_git_timeout_boundaries() {
    // Test zero timeout
    assert!(
        parse_config_toml(
            r"
[git]
timeout_seconds = 0
"
        )
        .is_err()
    );

    // Test too large timeout
    assert!(
        parse_config_toml(
            r"
[git]
timeout_seconds = 700
"
        )
        .is_err()
    );

    // Test valid timeout
    assert!(
        parse_config_toml(
            r"
[git]
timeout_seconds = 30
"
        )
        .is_ok()
    );
}

#[test]
fn test_parse_language_cache_ttl() {
    // Test zero cache TTL
    assert!(
        parse_config_toml(
            r"
[language]
cache_ttl_hours = 0
"
        )
        .is_err()
    );

    // Test too large cache TTL
    assert!(
        parse_config_toml(
            r"
[language]
cache_ttl_hours = 800
"
        )
        .is_err()
    );

    // Test valid cache TTL
    assert!(
        parse_config_toml(
            r"
[language]
cache_ttl_hours = 48
"
        )
        .is_ok()
    );
}

#[test]
fn test_parse_ui_path_length() {
    // Test zero path length
    assert!(
        parse_config_toml(
            r"
[ui.directory]
max_length = 0
"
        )
        .is_err()
    );

    // Test too large path length
    assert!(
        parse_config_toml(
            r"
[ui.directory]
max_length = 1500
"
        )
        .is_err()
    );

    // Test valid path length
    assert!(
        parse_config_toml(
            r"
[ui.directory]
max_length = 120
"
        )
        .is_ok()
    );
}

#[test]
fn test_config_validation_theme_valid() {
    let valid_themes = ["default", "text", "custom_theme"];

    for theme in valid_themes {
        let mut config = Config::default();
        config.ui.theme = types::ThemeName::new(theme.to_owned()).unwrap();

        let result = validate_config(&config);
        assert!(result.is_ok(), "Theme '{theme}' should be valid");
    }
}

#[test]
fn test_parse_theme_invalid() {
    let toml = r"
[ui]
theme = ''
";
    let result = parse_config_toml(toml);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("theme name must be non-empty")
    );
}

#[test]
fn test_load_config_from_nonexistent_file() {
    let result = load_config_from_file("/nonexistent/config.toml");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Failed to read config file")
    );
}

#[test]
fn test_load_config_from_file_success() {
    let temp_dir = TempDir::new().unwrap();
    let config_file = temp_dir.path().join("test_config.toml");

    // Write example config
    fs::write(&config_file, get_example_config()).unwrap();

    let result = load_config_from_file(config_file.to_str().unwrap());
    assert!(result.is_ok());

    let config = result.unwrap();
    assert!(config.agent.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 5);
}

#[test]
fn test_save_config_creates_directories() {
    let temp_dir = TempDir::new().unwrap();
    let nested_config_file = temp_dir
        .path()
        .join("nested")
        .join("deep")
        .join("config.toml");

    let config = Config::default();
    let result = save_config(
        nested_config_file.to_str().unwrap(),
        &config,
        &config,
        &["ui.show_icons"],
    );
    assert!(result.is_ok());

    // Verify nested directories were created
    assert!(nested_config_file.exists());
}

#[test]
fn test_get_config_paths() {
    let paths = get_config_paths();

    // Should return multiple paths in priority order
    assert!(!paths.is_empty());

    // Should include common config locations
    let paths_string = paths.join(" ");
    assert!(paths_string.contains(".config/gpy/config.toml") || paths_string.contains(".gpy.toml"));
}

#[test]
fn test_config_path_priority_order() {
    let paths = get_config_paths();

    // Should have at least 2 paths (home config and local override)
    // (Removed legacy ~/.gpy.toml path, so count reduced by 1)
    assert!(paths.len() >= 2);

    // Project-local config should be last (lowest priority)
    assert!(paths.last().unwrap().ends_with(".gpy.toml"));

    // User config paths should come before project-local
    let has_user_config = paths
        .iter()
        .any(|p| p.contains(".config/gpy") || p.contains("XDG_CONFIG"));
    assert!(has_user_config);
}

#[test]
fn test_config_boundary_values() {
    // Test exact boundary values
    assert!(types::AgentTimeout::new(300).is_some());
    assert!(types::AgentTimeout::new(301).is_none());

    assert!(types::GitTimeout::new(600).is_some());
    assert!(types::GitTimeout::new(601).is_none());

    assert!(types::CacheTtlHours::new(720).is_some());
    assert!(types::CacheTtlHours::new(721).is_none());

    assert!(types::MaxPathLength::new(1000).is_some());
    assert!(types::MaxPathLength::new(1001).is_none());
}

#[test]
fn test_example_config_is_valid() {
    // The example config should always pass validation
    let example_toml = get_example_config();
    let config = parse_config_toml(example_toml).unwrap();
    let result = validate_config(&config);
    assert!(result.is_ok());
}

#[test]
fn test_config_partial_sections() {
    // Test that partial TOML sections work (defaults fill in missing values)
    let partial_toml = r#"
[agent]
enabled = false

[ui]
theme = "text"
"#;

    let result = parse_config_toml(partial_toml);
    assert!(result.is_ok());

    let config = result.unwrap();

    // Now TOML parsing is implemented, these should work correctly:
    assert!(!config.agent.enabled);
    assert_eq!(config.ui.theme.as_str(), "text");

    // Should use defaults for unspecified values
    assert_eq!(config.agent.timeout_seconds.get(), 5); // default
    assert!(config.git.enabled); // default
}

#[test]
fn test_malformed_toml_error_handling() {
    let malformed_toml = r"
[agent
enabled = true
this is not valid toml
";

    let result = parse_config_toml(malformed_toml);
    // Now TOML parsing is implemented, this should return an error
    assert!(result.is_err());
}

#[test]
fn test_config_file_permissions() {
    let temp_dir = TempDir::new().unwrap();
    let config_file = temp_dir.path().join("permission_test.toml");

    // Create a config file
    let config = Config::default();
    save_config(
        config_file.to_str().unwrap(),
        &config,
        &config,
        &["ui.show_icons"],
    )
    .unwrap();

    // Verify we can read it back
    let result = load_config_from_file(config_file.to_str().unwrap());
    assert!(result.is_ok());
}

#[test]
fn test_config_utf8_handling() {
    let temp_dir = TempDir::new().unwrap();
    let config_file = temp_dir.path().join("utf8_test.toml");

    // Write config with UTF-8 content (comments in example config)
    fs::write(&config_file, get_example_config()).unwrap();

    let result = load_config_from_file(config_file.to_str().unwrap());
    assert!(result.is_ok());
}
#[allow(clippy::float_cmp)]
#[test]
fn shipped_config_toml_matches_builtin_defaults() {
    // Load the shipped config/config.toml file
    let shipped_config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/config.toml");
    let config = load_config_from_file(shipped_config_path).unwrap();

    // Verify language.confidence_threshold matches the built-in default
    assert_eq!(
        config.language.confidence_threshold.get(),
        types::ConfidenceThreshold::DEFAULT,
        "shipped config confidence_threshold must equal built-in default 0.1"
    );

    // Verify all shipped config values match defaults (except icons)
    let default_config = Config::default();

    // Convert both configs to TOML tables for comparison
    let shipped_value = toml::Value::try_from(&config).unwrap();
    let default_value = toml::Value::try_from(&default_config).unwrap();

    let mut shipped_table = shipped_value.as_table().unwrap().clone();
    let mut default_table = default_value.as_table().unwrap().clone();

    // Remove icon sections from comparison (icons are allowed to differ)
    if let Some(language_table) = shipped_table.get_mut("language")
        && let Some(table) = language_table.as_table_mut()
    {
        table.remove("icons");
    }
    if let Some(language_table) = default_table.get_mut("language")
        && let Some(table) = language_table.as_table_mut()
    {
        table.remove("icons");
    }
    if let Some(git_table) = shipped_table.get_mut("git")
        && let Some(table) = git_table.as_table_mut()
    {
        table.remove("icons");
    }
    if let Some(git_table) = default_table.get_mut("git")
        && let Some(table) = git_table.as_table_mut()
    {
        table.remove("icons");
    }

    assert_eq!(
        shipped_table, default_table,
        "shipped config must match defaults for all non-icon keys"
    );
}

/// Collect every dotted leaf path of `table` into `out` (#790).
fn toml_leaf_paths(prefix: &str, table: &toml::Table, out: &mut Vec<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => toml_leaf_paths(&path, inner, out),
            _ => out.push(path),
        }
    }
}

/// Every key the shipped config sets is a `Config` leaf (#790).
///
/// The reverse direction of `shipped_config_toml_matches_builtin_defaults`:
/// serde silently ignores unknown keys, so a typo or a removed field would
/// otherwise go unnoticed. Free-form icon maps are exempt.
#[test]
fn shipped_config_toml_sets_only_known_keys() {
    let shipped_config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/config.toml");
    let raw: toml::Table = fs::read_to_string(shipped_config_path)
        .unwrap()
        .parse()
        .unwrap();
    let mut known = Vec::new();
    let default_table = toml::Table::try_from(Config::default()).unwrap();
    toml_leaf_paths("", &default_table, &mut known);
    let mut shipped = Vec::new();
    toml_leaf_paths("", &raw, &mut shipped);
    let unknown: Vec<&String> = shipped
        .iter()
        .filter(|path| !path.starts_with("language.icons.") && !path.starts_with("git.icons."))
        .filter(|path| !known.contains(path))
        .collect();
    assert!(
        unknown.is_empty(),
        "config/config.toml sets keys Config does not have: {unknown:?}"
    );
}
