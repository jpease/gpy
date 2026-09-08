//! Configuration file loading and parsing
//!
//! Handles TOML parsing, file system access, and merging of configuration
//! sources with proper error handling.

use super::{Config, schema, validation};
use crate::{Error, Result};
use std::path::Path;

/// Config loading with XDG path discovery.
///
/// Load configuration from the first available config file. Always reads
/// fresh from disk (see #608 -- this used to cache the result in a process-
/// global static; that cache is gone, since every long-lived caller of
/// configuration state goes through `ConfigManager`'s own `Arc<RwLock<Arc<_>>>`
/// instead, and every remaining caller of this function is a short-lived,
/// one-shot CLI process for which a cache bought nothing but staleness).
///
/// # Errors
///
/// Returns an error if no valid configuration file is found or if the
/// configuration file cannot be read or parsed.
pub fn load_config() -> Result<Config> {
    schema::get_config_paths()
        .iter()
        .find(|path| Path::new(path).exists())
        .map_or_else(|| Ok(Config::default()), |path| load_config_from_file(path))
}

/// Load configuration from a specific TOML file
///
/// # Errors
///
/// Returns an error if the file cannot be read or if the TOML content
/// cannot be parsed into a valid configuration.
pub fn load_config_from_file(path: &str) -> Result<Config> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| Error::config(format!("Failed to read config file {path}: {e}")))?;

    parse_config_toml(&contents)
}

/// Parse TOML configuration content
///
/// # Errors
///
/// Returns an error if the TOML content is malformed or cannot be
/// deserialized into a valid configuration structure.
pub fn parse_config_toml(toml_content: &str) -> Result<Config> {
    // Parse TOML content with the toml crate
    let parsed_config: Config = toml::from_str(toml_content)
        .map_err(|e| Error::config(format!("Failed to parse TOML: {e}")))?;

    // Fill in any missing fields with defaults (functional transformation)
    let finalized_config = apply_defaults(parsed_config);

    // Validate the configuration
    validation::validate_config(&finalized_config)?;

    Ok(finalized_config)
}

/// Apply default values to missing configuration fields (functional transformation)
///
/// Currently a no-op pass-through: every field on [`Config`] is either a
/// plain type with a `#[serde(default = "...")]` fn, or a bounded newtype
/// (`AgentTimeout`, `GitTimeout`, `SupervisorCheckInterval`, etc.) that
/// supplies its own `Default` and validates itself at deserialize time via
/// `#[serde(try_from = "...")]`. Serde already fills in a missing key's
/// default before this function ever runs, so there is nothing left here to
/// backfill.
///
/// This used to also coerce an explicit `0` in
/// `agent.supervisor.check_interval_seconds`/`max_restart_attempts` to the
/// real default (`use_if_zero`), to work around those two fields being raw
/// `u64`/`u32` with no validated range. That silently accepted `0` while
/// `validate_config` hard-rejected `1..=4` for the same field — two invalid
/// values, handled inconsistently. #597 replaced the raw ints with
/// `types::SupervisorCheckInterval`/`types::SupervisorMaxRestartAttempts`, so
/// an explicit `0` (or any other out-of-range value) is now rejected at
/// deserialize time like every other bounded field, instead of silently
/// becoming the default. This is a disclosed behavior change: a saved config
/// with `check_interval_seconds = 0` (or `max_restart_attempts = 0`) that
/// used to load silently now fails config load with a clear error.
///
/// Kept as an explicit pipeline stage (rather than removed and inlined into
/// its one call site) so a future cross-field default that genuinely can't be
/// expressed as a per-field serde default has an obvious place to live.
const fn apply_defaults(config: Config) -> Config {
    config
}

/// Save configuration to file
///
/// # Errors
///
/// Returns an error if the configuration is invalid or if the file cannot be written.
pub fn save_config(config: &Config, path: &str) -> Result<()> {
    // Validate before saving
    validation::validate_config(config)?;

    // Serialize configuration to TOML
    let toml_content = toml::to_string_pretty(config)
        .map_err(|e| Error::config(format!("Failed to serialize config to TOML: {e}")))?;

    // Ensure parent directory exists
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("Failed to create config directory: {e}")))?;
    }

    // Write the configuration with a header comment
    let content_with_header = format!(
        "# GPY Agent Configuration\n# Generated automatically - edit carefully\n\n{toml_content}"
    );

    std::fs::write(path, content_with_header)
        .map_err(|e| Error::config(format!("Failed to write config file: {e}")))?;

    Ok(())
}

/// Load theme configuration from ~/.`config/gpy/themes/{theme_name}.toml`
///
/// Always reads and parses fresh from disk (see #608 -- this used to cache
/// the result in a process-global static; every caller is a short-lived,
/// one-shot CLI process for which a cache bought nothing but staleness).
///
/// # Errors
///
/// Returns an error if the theme file cannot be read or parsed, or if
/// `theme_name` does not resolve to a user theme, plugin theme, or builtin
/// (#572).
pub fn load_theme(theme_name: &str) -> Result<crate::theme::ThemeConfig> {
    // Resolve via the shared resolver (#572): the same resolution the daemon's
    // `ThemeManager::theme_path` uses, so oneshot can see plugin themes the
    // daemon can render, and an unresolvable name errors instead of silently
    // rendering a blank theme. This intentionally converges oneshot's
    // resolution with `crate::paths::config_root_for` (via
    // `ThemeManager::user_theme_path`), including for the previously-diverging
    // "XDG_CONFIG_HOME/HOME set to an empty string" edge case #477 preserved
    // in this function's now-deleted, hand-rolled `format!`-based predecessor
    // -- the daemon's own resolver already used `config_root_for` for that
    // case, so this unification makes oneshot match the daemon instead of
    // disagreeing with it, which is the point of #572.
    let theme_path = crate::theme::ThemeManager::resolve_path(theme_name).ok_or_else(|| {
        Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        ))
    })?;

    load_theme_from_path(&theme_path.display().to_string())
}

/// Load theme directly from a known file path.
///
/// Exposed for testing and tooling that want to load themes without relying on
/// global configuration discovery.
///
/// # Errors
///
/// Returns a configuration error if the file cannot be read, parsed, or fails
/// schema validation. Also returns an error naming the theme when `path`
/// matches neither a real file nor a builtin theme name (#572) -- this used
/// to silently return `ThemeConfig::default()`.
pub fn load_theme_from_path(path: &str) -> Result<crate::theme::ThemeConfig> {
    use crate::theme::manager::BUILTIN_THEMES;

    let contents = if Path::new(path).exists() {
        std::fs::read_to_string(path)
            .map_err(|e| Error::config(format!("Failed to read theme file {path}: {e}")))?
    } else if let Some((_, content)) = BUILTIN_THEMES.iter().find(|(name, _)| {
        // `Path::file_stem`, not a hardcoded `/`-separator suffix match: `path`
        // is built from a `PathBuf` (via `.display()`), so on Windows it is
        // `\`-separated, and `path.ends_with("/default.toml")` never matched
        // there -- a non-existent user theme file silently fell all the way
        // through to the bare `ThemeConfig::default()` stub instead of the
        // embedded builtin, dropping every icon/color the real theme sets.
        // `Path::new(path)` parses separators per the HOST's convention, which
        // is correct here because `path` was always built on this same host.
        Path::new(path).file_stem().and_then(|s| s.to_str()) == Some(*name)
    }) {
        (*content).to_owned()
    } else {
        let theme_name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path);
        return Err(Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        )));
    };

    crate::theme::parse(&contents, path)
}

#[cfg(test)]
mod load_theme_from_path_tests {
    use super::load_theme_from_path;

    /// Regression lock for #482/discovered-live: `load_theme_from_path`'s builtin-theme
    /// fallback used to match via `path.ends_with(&format!("/{name}.toml"))`, a hardcoded `/`.
    ///
    /// `path` here is always built from a `PathBuf` via `.display()`, so on Windows it is
    /// `\`-separated and never matched -- confirmed on a real Windows VM
    /// (`scripts/test-windows-vm.sh`) that a non-existent user theme path like
    /// `C:\Users\x\AppData\Local\gpy\themes\default.toml` fell all the way through to the bare
    /// `ThemeConfig::default()` stub instead of the embedded `default` theme, dropping its
    /// `ui.prompt_close` icon (the symptom that surfaced as
    /// `clock_demo_spans_renders_closing_cap_on_the_real_default_theme` failing there).
    ///
    /// `std::path::MAIN_SEPARATOR` rather than a hardcoded `/`, so this test
    /// exercises the same separator convention `load_theme_from_path` itself
    /// resolves on whichever host it runs on -- meaningful proof on any
    /// platform, unlike a literal `\`-joined string tested from a Unix host
    /// (`Path` only parses `\` as a separator when the crate is actually
    /// compiled for Windows, so that specific case can only be verified by
    /// actually running there).
    ///
    /// # Panics
    ///
    /// Panics if a non-existent user theme path fails to fall back to the
    /// embedded builtin theme.
    #[test]
    fn nonexistent_user_theme_path_falls_back_to_embedded_builtin() {
        let sep = std::path::MAIN_SEPARATOR;
        let fake_user_theme_path = format!("nonexistent-dir{sep}gpy{sep}themes{sep}default.toml");

        let theme =
            load_theme_from_path(&fake_user_theme_path).expect("embedded fallback must succeed");

        // `UiTheme::default()` (the bare stub) sets `prompt_close` to
        // `Some(DelimiterConfig::simple(""))` -- an EMPTY icon, per
        // `default_ui_prompt_close()` -- so `Option::is_some()` can't tell
        // the embedded theme apart from the stub; both have a `Some`. The
        // icon TEXT is what actually differs: the stub's is empty, the real
        // embedded `default` theme's is the Nerd Font powerline glyph
        // U+E0B4 (confirmed against `config/themes/default.toml`, same as
        // `clock_demo_spans_renders_closing_cap_on_the_real_default_theme`).
        assert_eq!(
            theme.ui.get_prompt_close_delimiter(),
            "\u{e0b4}",
            "expected the embedded `default` theme's real closing-cap icon, got the bare \
             ThemeConfig::default() stub's empty one instead -- builtin-theme matching didn't fire"
        );
    }

    /// Regression lock for #572: this used to assert the OPPOSITE.
    ///
    /// A path matching no real file and no builtin theme name silently fell
    /// back to `ThemeConfig::default()`'s bare stub, which is exactly the
    /// silent-blank-theme bug #572 fixes (`ui.theme = "nrod"` rendering a
    /// blank prompt with no error). It now must error, naming the theme.
    ///
    /// # Panics
    ///
    /// Panics if a path that matches neither a real file nor any builtin
    /// theme name doesn't return an error naming the theme.
    #[test]
    fn path_matching_no_builtin_theme_name_returns_an_error() {
        let err = load_theme_from_path("nonexistent-dir/not-a-real-theme.toml")
            .expect_err("a path matching no builtin theme name must error, not silently succeed");

        let message = err.to_string();
        assert!(
            message.contains("not-a-real-theme"),
            "error message must name the theme (its file stem), got: {message}"
        );
    }
}
