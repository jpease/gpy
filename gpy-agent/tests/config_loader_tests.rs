//! TOML configuration loading and parsing.

#![allow(clippy::bool_comparison)]
#![allow(clippy::single_match_else)]

use gpy_agent::config::loader::{load_config_from_file, parse_config_toml, save_config};
use gpy_agent::config::location::get_config_paths;
use gpy_agent::config::validation::validate_config;
use gpy_agent::config::{Config, types};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// The config template shipped in release archives and installed by `install-dev.fish`.
const TEMPLATE: &str = include_str!("../../config/config.example.toml");

/// The template with every commented-out setting enabled.
///
/// A setting is `#key = value` (no space after `#`); prose comments start
/// with `# ` and stay comments.
fn template_settings() -> String {
    TEMPLATE
        .lines()
        .map(|line| match line.strip_prefix('#') {
            Some(rest) if is_setting(rest) => rest,
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `text` starts with a bare key followed by ` = `.
fn is_setting(text: &str) -> bool {
    let key_len = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .unwrap_or(text.len());
    key_len > 0 && text[key_len..].starts_with(" = ")
}

#[test]
fn test_config_default_enabled() {
    let config = Config::default();
    assert!(config.language.enabled);
}

#[test]
fn test_parse_empty_toml() {
    let config = parse_config_toml("").expect("parse empty toml");
    assert!(config.agent.enabled);
    assert!(config.language.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 5);
}

#[test]
fn test_config_template_agent_values() {
    let config = parse_config_toml(&template_settings()).unwrap();
    assert!(config.agent.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 5);
    assert!(config.agent.live_updates);
    assert!(config.agent.supervisor.enabled);
    assert_eq!(config.agent.supervisor.check_interval_seconds.get(), 30);
    assert_eq!(config.agent.supervisor.max_restart_attempts.get(), 5);
}

#[test]
fn test_config_template_git_values() {
    let config = parse_config_toml(&template_settings()).unwrap();
    assert!(config.git.enabled);
    assert!(config.git.show_upstream);
    assert_eq!(config.git.timeout_seconds.get(), 10);
}

#[test]
fn test_config_template_language_values() {
    let config = parse_config_toml(&template_settings()).unwrap();
    assert!(config.language.enabled);
    assert!(config.language.show_versions);
    assert_eq!(config.language.cache_ttl_hours.get(), 24);
    assert_eq!(config.language.enabled_languages, Vec::<String>::new());
}

#[test]
fn test_config_template_ui_values() {
    let config = parse_config_toml(&template_settings()).unwrap();
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
    validate_config(&config).expect("default config should validate");
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
    assert!(
        parse_config_toml(
            r"
[git]
timeout_seconds = 0
"
        )
        .is_err()
    );

    assert!(
        parse_config_toml(
            r"
[git]
timeout_seconds = 700
"
        )
        .is_err()
    );

    let config = parse_config_toml(
        r"
[git]
timeout_seconds = 30
",
    )
    .expect("timeout_seconds = 30 should parse");
    assert_eq!(config.git.timeout_seconds.get(), 30);
}

#[test]
fn test_parse_language_cache_ttl() {
    assert!(
        parse_config_toml(
            r"
[language]
cache_ttl_hours = 0
"
        )
        .is_err()
    );

    assert!(
        parse_config_toml(
            r"
[language]
cache_ttl_hours = 800
"
        )
        .is_err()
    );

    let config = parse_config_toml(
        r"
[language]
cache_ttl_hours = 48
",
    )
    .expect("cache_ttl_hours = 48 should parse");
    assert_eq!(config.language.cache_ttl_hours.get(), 48);
}
#[test]
fn test_parse_ui_path_length() {
    assert!(
        parse_config_toml(
            r"
[ui.directory]
max_length = 0
"
        )
        .is_err()
    );

    let config = parse_config_toml(
        r"
[ui.directory]
max_length = 120
",
    )
    .expect("max_length = 120 should parse");
    assert_eq!(config.ui.directory.max_length.get(), 120);
}
#[test]
fn test_config_validation_theme_valid() {
    let valid_themes = ["default", "text", "custom_theme"];

    for theme in valid_themes {
        let mut config = Config::default();
        config.ui.theme = types::ThemeName::new(theme.to_owned()).unwrap();

        let msg = format!("Theme '{theme}' should be valid");
        validate_config(&config).expect(&msg);
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
            .contains("invalid theme name")
    );
}

#[test]
fn test_load_config_from_nonexistent_file() {
    let result = load_config_from_file(Path::new("/nonexistent/config.toml"));
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

    fs::write(&config_file, template_settings()).unwrap();

    let config = load_config_from_file(&config_file).expect("load config from file");
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
    save_config(&nested_config_file, &config, &config, &["ui.show_icons"])
        .expect("save config to nested path");

    assert!(nested_config_file.exists());
}

#[test]
fn test_get_config_paths() {
    let paths = get_config_paths();

    assert_ne!(paths, Vec::<PathBuf>::new());

    assert!(paths.iter().any(|p| p.ends_with(".config/gpy/config.toml")));
}

#[test]
fn test_config_path_priority_order() {
    let paths = get_config_paths();

    // No project-local `.gpy.toml`: every candidate is absolute.
    assert!(
        paths.iter().all(|p| p.is_absolute()),
        "every config candidate must be absolute: {paths:?}"
    );
    assert!(!paths.iter().any(|p| p.ends_with(".gpy.toml")));

    assert!(paths.last().unwrap().ends_with(".config/gpy/config.toml"));
}

#[test]
fn test_config_boundary_values() {
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
fn test_config_template_is_valid() {
    let settings = template_settings();
    let config = parse_config_toml(&settings).unwrap();
    validate_config(&config).expect("config template should validate");
}

#[test]
fn test_config_partial_sections() {
    // Defaults fill missing values.
    let partial_toml = r#"
[agent]
enabled = false

[ui]
theme = "text"
"#;

    let config = parse_config_toml(partial_toml).expect("parse partial toml");
    assert!(!config.agent.enabled);
    assert_eq!(config.ui.theme.as_str(), "text");

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
    assert!(result.is_err());
}

#[test]
fn test_config_file_permissions() {
    let temp_dir = TempDir::new().unwrap();
    let config_file = temp_dir.path().join("permission_test.toml");

    let config = Config::default();
    save_config(&config_file, &config, &config, &["ui.show_icons"]).unwrap();

    let loaded = load_config_from_file(&config_file).expect("load config from file");
    assert!(loaded.ui.show_icons);
}
#[test]
fn test_config_utf8_handling() {
    let temp_dir = TempDir::new().unwrap();
    let config_file = temp_dir.path().join("utf8_test.toml");

    fs::write(&config_file, template_settings()).unwrap();

    let loaded = load_config_from_file(&config_file).expect("load config from utf8 file");
    assert!(loaded.agent.enabled);
}

/// Dotted leaf paths of `table`, one per scalar or array.
fn toml_leaf_paths(prefix: &str, table: &toml::Table) -> Vec<String> {
    let mut out = Vec::new();
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => out.extend(toml_leaf_paths(&path, inner)),
            _ => out.push(path),
        }
    }
    out
}

/// `table` without `language.icons` and `git.icons`: the template offers
/// optional icon overrides there, while the built-in defaults are unset.
fn without_icon_tables(mut table: toml::Table) -> toml::Table {
    if let Some(toml::Value::Table(language)) = table.get_mut("language") {
        language.remove("icons");
    }
    if let Some(toml::Value::Table(git)) = table.get_mut("git") {
        git.remove("icons");
    }
    table
}

/// The template sets exactly the keys `Config` serializes (no `Config` leaf
/// missing, no key serde would silently ignore) and every value equals its
/// built-in default.
#[test]
fn config_template_sets_every_config_key_at_its_default() {
    let settings = template_settings();
    let raw: toml::Table = settings.parse().unwrap();
    let defaults = toml::Table::try_from(Config::default()).unwrap();

    let template_keys = toml_leaf_paths("", &without_icon_tables(raw));
    let default_keys = toml_leaf_paths("", &without_icon_tables(defaults.clone()));
    let missing: Vec<&String> = default_keys
        .iter()
        .filter(|key| !template_keys.contains(key))
        .collect();
    let unknown: Vec<&String> = template_keys
        .iter()
        .filter(|key| !default_keys.contains(key))
        .collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "config.example.toml drifted from Config: settings missing from the template: \
         {missing:?}; template keys Config does not have: {unknown:?}"
    );

    let loaded = parse_config_toml(&settings).unwrap();
    assert_eq!(
        without_icon_tables(toml::Table::try_from(&loaded).unwrap()),
        without_icon_tables(defaults),
        "config.example.toml sets a value that differs from the built-in default"
    );
}

/// The commented `[git.icons]` values are the Unicode defaults.
#[test]
fn config_template_git_icons_are_the_unicode_defaults() {
    let loaded = parse_config_toml(&template_settings()).unwrap();
    assert_eq!(
        loaded.git.icons,
        gpy_agent::config::GitIcons::default().resolved(gpy_agent::config::GitIconSet::Unicode),
    );
}

/// The commented `[language.icons]` entries are the built-in defaults.
#[test]
fn config_template_language_icons_are_the_builtin_defaults() {
    let loaded = parse_config_toml(&template_settings()).unwrap();
    let mut template: Vec<(&str, &str)> = loaded
        .language
        .icons
        .icons
        .iter()
        .map(|(key, icon)| (key.as_str(), icon.as_str()))
        .collect();
    template.sort_unstable();
    let mut builtin: Vec<(&str, &str)> =
        gpy_agent::config::defaults::DEFAULT_LANGUAGE_ICONS.to_vec();
    builtin.sort_unstable();
    assert_eq!(template, builtin);
}

/// A copy of the template as shipped changes nothing, so installing it never
/// pins a setting that a later default change or `theme use --force` should own.
#[test]
fn config_template_as_shipped_is_the_default_config() {
    let loaded = parse_config_toml(TEMPLATE).unwrap();
    assert_eq!(
        toml::Table::try_from(&loaded).unwrap(),
        toml::Table::try_from(Config::default()).unwrap(),
    );
}

/// The commented examples for the two open-ended tables are valid when enabled.
#[test]
fn config_template_icon_and_plugin_examples_are_present() {
    let loaded = parse_config_toml(&template_settings()).unwrap();
    assert!(
        loaded.language.icons.icons.len() > 1,
        "template lists no [language.icons] examples"
    );
    assert!(
        TEMPLATE.contains("[plugin.<segment>]"),
        "template does not document [plugin.<segment>]"
    );
}

/// A non-UTF-8 candidate path is kept byte-for-byte, not lossily rewritten.
#[cfg(unix)]
#[test]
fn test_non_utf8_config_candidate_is_preserved() {
    use gpy_agent::config::location::config_candidates_for;
    use gpy_agent::paths::Os;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let config_file = PathBuf::from(OsStr::from_bytes(b"/tmp/conf\xff.toml"));
    assert!(config_file.to_str().is_none());

    let candidates = config_candidates_for(Some(&config_file), None, None, Os::Unix);
    assert_eq!(candidates, std::slice::from_ref(&config_file));
}

/// Load and save go through the path itself. APFS and HFS+ reject non-UTF-8
/// names, so only Linux-like file systems can create the file.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn test_non_utf8_config_path_round_trips() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let temp_dir = TempDir::new().unwrap();
    let config_file = temp_dir.path().join(OsStr::from_bytes(b"conf\xff.toml"));

    let config = Config::default();
    save_config(&config_file, &config, &config, &["ui.show_icons"]).unwrap();
    let loaded = load_config_from_file(&config_file).expect("load config from non-utf8 path");
    assert!(loaded.ui.show_icons);
}
