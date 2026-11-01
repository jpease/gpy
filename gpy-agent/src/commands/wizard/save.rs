//! Persists the wizard's selection via the shared config load/validate/save
//! path, and its theme edits as an overlay (see [`theme_file`](super::theme_file)).

use super::state::WizardState;
use super::theme_file::{ThemeWrite, save_theme};
use crate::Result;
use crate::commands::palette;
use crate::commands::theme;
use crate::commands::utils::{active_config_path, load_active_config};
use crate::config::loader::save_config;
use std::path::{Path, PathBuf};

/// Apply, validate, and persist `state`'s selection onto the config at `path`.
///
/// Reloads `path` first so concurrent edits to unrelated fields survive.
/// `validate_by_name` is needed because config validation never checks that
/// the named theme/palette exists.
///
/// # Errors
///
/// Returns an error if loading, validation, or the write fails.
pub fn save_to(state: &WizardState, path: &Path) -> Result<()> {
    let original = load_active_config(path)?;
    let mut config = original.clone();

    state.apply_to(&mut config);

    theme::validate_by_name(state.selected_theme())?;
    palette::validate_by_name(state.selected_palette())?;

    save_config(path, &original, &config, &[])?;
    Ok(())
}

/// What [`save`] wrote.
#[derive(Debug)]
pub struct Saved {
    /// The config file.
    pub config: PathBuf,
    /// The theme file the edits went to; `None` when no edit changed a value.
    pub theme: Option<ThemeWrite>,
}

/// Persist the theme edits, then the selection to the active config path.
/// The theme file is written first, so the config is validated against it.
///
/// # Errors
///
/// Returns an error if the theme edits can't be written, the path cannot be
/// resolved, or [`save_to`] fails.
pub fn save(state: &WizardState) -> Result<Saved> {
    // Theme field edits go to a user file of the theme, never the builtin.
    let theme = match state.theme_edits() {
        Some(edits) => save_theme(state.selected_theme(), edits)?,
        None => None,
    };
    let config = active_config_path()?;
    save_to(state, &config)?;
    Ok(Saved { config, theme })
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::commands::wizard::state::{SegmentEdit, WizardState};
    use crate::config::Config;
    use crate::config::types::GitTimeout;
    use crate::theme::ThemeConfig;
    use std::fs;
    use tempfile::TempDir;

    /// The `TempDir` must outlive the returned path.
    fn write_starting_config(contents: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("gpy.toml");
        fs::write(&path, contents).expect("write starting config");
        (dir, path)
    }

    fn test_state(config: Config) -> WizardState {
        crate::commands::wizard::state::builtin_state(config)
    }

    #[test]
    fn save_to_persists_theme_palette_and_segments() {
        let (_dir, path) = write_starting_config(
            "[git]\nenabled = false\n\n[ui]\ntheme = \"default\"\npalette = \"default\"\nenabled_segments = [\"directory\"]\n",
        );

        let config = load_active_config(&path).expect("load starting config");
        let mut state = test_state(config);
        state.select_theme("default");
        state.select_palette("default");
        // Starts disabled, so this proves apply_to writes through.
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

        // Not an apply_to field; must survive untouched.
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
        state.apply_edit(SegmentEdit::NextDirectoryDisplay, &ThemeConfig::default());

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
        let theme = ThemeConfig::default();
        state.apply_edit(SegmentEdit::ToggleLanguageDisplay, &theme);
        state.apply_edit(SegmentEdit::ToggleLanguageShowVersions, &theme);

        save_to(&state, &path).expect("save_to should succeed");

        let saved = fs::read_to_string(&path).expect("read saved config");
        let reloaded: Config = toml::from_str(&saved).expect("parse saved config");

        assert_eq!(reloaded.language.display, LanguageDisplay::Text);
        assert!(!reloaded.language.show_versions);
    }
}
