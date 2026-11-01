//! Loading a theme fresh from disk, by name or by path, for callers with no
//! `ThemeManager` state (oneshot rendering, `gpy theme` commands, `doctor`).

use super::{ChainLink, LoadedTheme, ThemeConfig, overlay, user_copy};
use crate::{Error, Result};
use std::path::Path;

/// Load a theme by name (user, plugin, or builtin), fresh from disk, merging
/// an overlay's chain.
///
/// # Errors
/// Returns an error if the theme is unresolvable or fails to parse.
pub fn load_theme(theme_name: &str) -> Result<ThemeConfig> {
    load_named_theme(theme_name).map(|loaded| loaded.theme)
}

/// [`load_theme`], keeping the chain the theme was merged from so a later
/// template check can name the file and line of a failing key
/// ([`LoadedTheme::template_error`]).
///
/// # Errors
/// Returns an error if the theme is unresolvable or fails to parse.
pub fn load_named_theme(theme_name: &str) -> Result<LoadedTheme> {
    // Same resolver as the daemon so oneshot agrees on plugin themes.
    let top = overlay::resolve_top(theme_name).ok_or_else(|| {
        Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        ))
    })?;
    load_top_annotated(top)
}

/// Load the chain starting at `top`, annotating a failure with the top file's
/// path. The one annotate step shared by [`load_named_theme`] and
/// `ThemeManager`'s load.
pub(crate) fn load_top_annotated(top: ChainLink) -> Result<LoadedTheme> {
    let path = top.path.clone();
    overlay::load_link(top).map_err(|error| user_copy::annotate_load_error(&path, error))
}

/// Load a theme from a path, falling back to the builtin with that stem.
/// An overlay's `extends` resolves as for a user theme named after the stem,
/// or, for a file in a plugin's `themes/` directory, as for that plugin.
///
/// This is the single funnel for theme load errors by path (CLI, `doctor`,
/// `theme validate`); loads by name go through [`load_named_theme`] or
/// `ThemeManager`, which annotate the same way. When the failing file is the
/// user copy of an embedded builtin, the returned error already ends with
/// [`user_copy::recovery_hint`] (it says the file overrides the builtin).
/// Callers MUST NOT append the hint again.
///
/// # Errors
/// Returns an error if the file can't be read or parsed, or matches no builtin.
#[cfg(any(test, feature = "test-support"))]
pub fn load_theme_from_path(path: &Path) -> Result<ThemeConfig> {
    load_theme_file(path).map(|loaded| loaded.theme)
}

/// Load a theme from a path like `load_theme_from_path` (test-only), keeping
/// the chain the theme was merged from.
///
/// # Errors
/// Returns an error if the file can't be read or parsed, or matches no builtin.
pub fn load_theme_file(path: &Path) -> Result<LoadedTheme> {
    overlay::load_path(path).map_err(|error| user_copy::annotate_load_error(path, error))
}

#[cfg(test)]
mod load_theme_from_path_tests {
    use super::load_theme_from_path;
    use std::path::Path;

    #[test]
    fn nonexistent_user_theme_path_falls_back_to_embedded_builtin() {
        let sep = std::path::MAIN_SEPARATOR;
        let fake_user_theme_path = format!("nonexistent-dir{sep}gpy{sep}themes{sep}default.toml");

        let theme = load_theme_from_path(Path::new(&fake_user_theme_path))
            .expect("embedded fallback must succeed");

        // The bare default stub's `prompt_close` is empty; the builtin's is U+E0B4.
        assert_eq!(
            theme.ui.get_prompt_close_delimiter(),
            "\u{e0b4}",
            "expected the embedded `default` theme's real closing-cap icon, got the bare \
             ThemeConfig::default() stub's empty one instead -- builtin-theme matching didn't fire"
        );
    }

    #[test]
    fn path_matching_no_builtin_theme_name_returns_an_error() {
        let err = load_theme_from_path(Path::new("nonexistent-dir/not-a-real-theme.toml"))
            .expect_err("a path matching no builtin theme name must error, not silently succeed");

        let message = err.to_string();
        assert!(
            message.contains("not-a-real-theme"),
            "error message must name the theme (its file stem), got: {message}"
        );
    }
}
