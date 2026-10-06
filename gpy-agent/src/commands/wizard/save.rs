//! Persists the wizard's selection through the same load/validate/save path
//! every other `gpy config`/`gpy theme`/`gpy palette` command uses.
//!
//! See `commands::utils::{active_config_path, load_active_config,
//! save_config_to, reload_agent_and_notify}` and `commands::config::set` for
//! the pattern this mirrors.
//!
//! `save` is split from [`save_to`] so the persistence logic is directly
//! unit-testable against an explicit path (a `TempDir`-backed file, no process
//! environment mutation, no subprocess) the same way
//! `commands::utils::load_active_config`/`save_config_to` already take an
//! explicit path rather than resolving it themselves. `save` is the one
//! `wizard/mod.rs`'s `run` actually calls, after restoring the terminal; it
//! resolves the real active config path.

use super::state::WizardState;
use crate::Result;
use crate::commands::palette;
use crate::commands::theme;
use crate::commands::utils::{active_config_path, load_active_config, save_config_to};

/// Apply, validate, and persist `state`'s selection onto the config at `path`.
///
/// Loads the config fresh from `path` (not `state`'s startup snapshot)
/// immediately before writing, same as `commands::config::set`, so a config
/// file edited by something else while the wizard was open isn't silently
/// clobbered by a stale in-memory copy — only the wizard's own
/// theme/palette/segment fields are overwritten; everything else in the
/// freshly-loaded config passes through untouched.
///
/// The explicit `theme::validate_by_name`/`palette::validate_by_name` calls
/// are not redundant with `save_config_to`'s own validation:
/// `save_config_to` (`config::loader::save_config` ->
/// `config::validation::config::validate_config`) checks cross-field
/// constraints (supervisor timing, `skip_paths`, segment name syntax) but
/// never touches `ui.theme`/`ui.palette` at all, and `ThemeName`/`PaletteName`
/// themselves (`config/types.rs`) only check that the string is a safe config
/// name (non-empty, no path separators, no control characters) — neither
/// checks that a theme/palette file with that name actually exists and
/// parses. `validate_by_name` is what catches a selection that names a
/// syntactically-fine but nonexistent or malformed theme/palette, so both
/// checks stay.
///
/// # Errors
///
/// Returns an error if the config at `path` exists but cannot be read,
/// parsed, or validated; if the selected theme or palette fails validation;
/// or if the write fails.
pub fn save_to(state: &WizardState, path: &str) -> Result<()> {
    let original = load_active_config(path)?;
    let mut config = original.clone();

    state.apply_to(&mut config);

    theme::validate_by_name(state.selected_theme())?;
    palette::validate_by_name(state.selected_palette())?;

    save_config_to(path, &original, &config, &[])?;
    Ok(())
}

/// Apply, validate, and persist the wizard's current selection to the real
/// active config path and return that path.
///
/// Prints nothing and does not reload the agent: the caller runs this after
/// the terminal is restored, then reports the saved path and calls
/// `reload_agent_and_notify` itself, so the feedback lands on the normal
/// screen instead of being discarded with the alternate screen (#801).
///
/// # Errors
///
/// Returns an error if the config path cannot be resolved, the freshly-loaded
/// config is invalid, the selected theme/palette fails validation, or the
/// write fails.
pub fn save(state: &WizardState) -> Result<String> {
    let path = active_config_path()?;
    save_to(state, &path)?;
    // Theme-level field edits (clock/duration/git/…) live in the per-theme
    // TOML file, not `config.toml`, so they persist through a separate path:
    // materialize a user-writable copy of the active theme and write the edits
    // into it, never touching the builtin source (#407). No-op when no theme
    // field was edited.
    if let Some(theme) = state.pending_theme() {
        crate::theme::ThemeManager::save_user_theme(state.selected_theme(), theme)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::commands::wizard::state::WizardState;
    use crate::config::Config;
    use crate::config::types::GitTimeout;
    use std::fs;
    use tempfile::TempDir;

    /// Write `contents` to a fresh temp-dir config file and return its path.
    ///
    /// Returned as a `String` (matching the `&str`-path signature `save_to`
    /// takes) alongside the `TempDir` guard that must outlive the returned
    /// path.
    fn write_starting_config(contents: &str) -> (TempDir, String) {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("gpy.toml");
        fs::write(&path, contents).expect("write starting config");
        let path_str = path.to_string_lossy().into_owned();
        (dir, path_str)
    }

    /// Fixed, deterministic stand-in for `WizardState::new`'s real discovery.
    ///
    /// See `state.rs`'s own `FIXED_THEMES`/`FIXED_PALETTES`/`FIXED_SEGMENTS`
    /// for the rationale (#607). Kept in sync with those exact values.
    fn test_state(config: Config) -> WizardState {
        WizardState::from_parts(
            config,
            vec![
                "default".to_owned(),
                "starship".to_owned(),
                "text".to_owned(),
            ],
            vec![
                "catppuccin-frappe".to_owned(),
                "catppuccin-latte".to_owned(),
                "catppuccin-macchiato".to_owned(),
                "catppuccin-mocha".to_owned(),
                "default".to_owned(),
                "gruvbox-dark-medium".to_owned(),
                "nord".to_owned(),
                "starship".to_owned(),
            ],
            vec![
                "clock".to_owned(),
                "duration".to_owned(),
                "language".to_owned(),
                "directory".to_owned(),
                "git".to_owned(),
                "status".to_owned(),
            ],
        )
    }

    #[test]
    fn save_to_persists_theme_palette_and_segments() {
        let (_dir, path) = write_starting_config(
            "[git]\nenabled = false\n\n[ui]\ntheme = \"default\"\npalette = \"default\"\nenabled_segments = [\"directory\"]\n",
        );

        let config = load_active_config(&path).expect("load starting config");
        let mut state = test_state(config);
        // Only "default" is guaranteed to be discovered in every test
        // environment (see state.rs's own tests for the same fallback
        // rationale), so exercise reselecting it plus a segment toggle
        // rather than depending on a second built-in theme/palette existing.
        state.select_theme("default");
        state.select_palette("default");
        // Starting config has git disabled; toggling flips it on so the
        // assertion below proves apply_to actually wrote the change through.
        state.toggle_segment("git");

        save_to(&state, &path).expect("save_to should succeed");

        let saved = fs::read_to_string(&path).expect("read saved config");
        let reloaded: Config = toml::from_str(&saved).expect("parse saved config");

        assert_eq!(reloaded.ui.theme.as_str(), "default");
        assert_eq!(reloaded.ui.palette.to_string(), "default");
        assert!(reloaded.git.enabled);
        assert!(
            reloaded
                .ui
                .enabled_segments
                .contains(&"directory".to_owned())
        );
    }

    #[test]
    fn save_to_round_trip_keeps_git_and_language_rendered() {
        // #692: saving without touching anything must keep git/language in
        // the list the prompt export iterates.
        let (_dir, path) = write_starting_config(
            "[ui]\ntheme = \"default\"\npalette = \"default\"\nenabled_segments = [\"clock\", \"duration\", \"language\", \"directory\", \"git\"]\n",
        );

        let config = load_active_config(&path).expect("load starting config");
        let state = test_state(config);

        save_to(&state, &path).expect("save_to should succeed");

        let saved = fs::read_to_string(&path).expect("read saved config");
        let reloaded: Config = toml::from_str(&saved).expect("parse saved config");

        assert_eq!(
            reloaded.ui.enabled_segments,
            vec!["clock", "duration", "language", "directory", "git"]
        );
        assert!(reloaded.git.enabled);
        assert!(reloaded.language.enabled);
    }

    #[test]
    fn save_to_preserves_untouched_config_fields() {
        let (_dir, path) = write_starting_config(
            "[git]\ntimeout_seconds = 42\n\n[ui]\ntheme = \"default\"\npalette = \"default\"\n",
        );

        let config = load_active_config(&path).expect("load starting config");
        assert_eq!(
            config.git.timeout_seconds,
            GitTimeout::new(42).expect("42 is a valid git timeout")
        );

        let mut state = test_state(config);
        state.toggle_segment("git");

        save_to(&state, &path).expect("save_to should succeed");

        let saved = fs::read_to_string(&path).expect("read saved config");
        let reloaded: Config = toml::from_str(&saved).expect("parse saved config");

        // git.timeout_seconds is not one of apply_to's fields (theme,
        // palette, git.enabled, language.enabled, ui.enabled_segments) and
        // must survive the save untouched.
        assert_eq!(
            reloaded.git.timeout_seconds,
            GitTimeout::new(42).expect("42 is a valid git timeout")
        );
    }

    #[test]
    fn save_to_persists_directory_display() {
        use crate::config::types::DirectoryDisplay;

        let (_dir, path) =
            write_starting_config("[ui]\ntheme = \"default\"\npalette = \"default\"\n");

        let config = load_active_config(&path).expect("load starting config");
        assert_eq!(config.ui.directory.display, DirectoryDisplay::Basename);

        let mut state = test_state(config);
        // Move onto the Segments section's "directory" entry — the only
        // place `cycle_directory_display` takes effect (see
        // `WizardState::focused_builtin`).
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("directory")
        {
            state.move_cursor_down();
        }
        state.cycle_directory_display(true);

        save_to(&state, &path).expect("save_to should succeed");

        let saved = fs::read_to_string(&path).expect("read saved config");
        let reloaded: Config = toml::from_str(&saved).expect("parse saved config");

        assert_eq!(reloaded.ui.directory.display, DirectoryDisplay::Abbreviated);
    }

    #[test]
    fn save_to_persists_language_display_and_show_versions() {
        use crate::config::types::LanguageDisplay;

        let (_dir, path) =
            write_starting_config("[ui]\ntheme = \"default\"\npalette = \"default\"\n");

        let config = load_active_config(&path).expect("load starting config");
        assert_eq!(config.language.display, LanguageDisplay::Icon);
        assert!(config.language.show_versions);

        let mut state = test_state(config);
        // Move onto the Segments section's "language" entry — the only
        // place `cycle_language_display`/`toggle_language_show_versions`
        // take effect (see `WizardState::focused_builtin`).
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("language")
        {
            state.move_cursor_down();
        }
        state.cycle_language_display(true);
        state.toggle_language_show_versions();

        save_to(&state, &path).expect("save_to should succeed");

        let saved = fs::read_to_string(&path).expect("read saved config");
        let reloaded: Config = toml::from_str(&saved).expect("parse saved config");

        assert_eq!(reloaded.language.display, LanguageDisplay::Text);
        assert!(!reloaded.language.show_versions);
    }

    // save_to_rejects_invalid_selection is intentionally omitted:
    // WizardState's own invariants make it untestable from outside the
    // module. `selected_theme`/`selected_palette` can only ever be set by
    // `select_theme`/`select_palette`, and both are no-ops unless the name is
    // already present in `available_themes`/`available_palettes` — themselves
    // populated straight from `ThemeManager::discover_available_themes`/
    // `PaletteManager::discover_available_palettes`, the exact sources
    // `theme::validate_by_name`/`palette::validate_by_name` check against.
    // There is no `WizardState` constructor or setter this test module can
    // reach that lands a bogus name in `selected_theme`/`selected_palette`
    // without bypassing `WizardState`'s own encapsulation (its fields are
    // private to `state.rs`), so this failure path has no way to be
    // triggered from a `save.rs` test without weakening that encapsulation
    // just to test around it.
}
