//! Default configuration values and constants
//!
//! This module contains all default values used throughout the configuration system.
//! These functions are used by serde's `#[serde(default = "...")]` attributes and
//! by the `Default` trait implementations for configuration structs.

use crate::config::types::{ColorSpec, Icon};

/// Build an [`Icon`] from a hardcoded default literal.
///
/// # Panics
/// Panics if `s` is not a valid icon; callers pass only known-good literals.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
fn icon(s: &'static str) -> Icon {
    Icon::new(s).expect("Default icon must be valid")
}

/// Build a [`ColorSpec`] from a hardcoded default literal.
///
/// # Panics
/// Panics if `s` is not a valid color; callers pass only known-good literals.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
fn color(s: &'static str) -> ColorSpec {
    ColorSpec::new(s).expect("Hardcoded default color must be valid")
}

/// Default value for boolean flags (enabled)
pub(crate) const fn default_true() -> bool {
    true
}

/// Default maximum ahead/behind count (0 = unlimited)
pub(crate) const fn default_max_ahead_behind() -> usize {
    100
}

/// Default git ahead indicator
pub(crate) fn default_git_icon_ahead() -> Icon {
    icon("↑")
}

/// Default git behind indicator
pub(crate) fn default_git_icon_behind() -> Icon {
    icon("↓")
}

/// Default git staged files indicator
pub(crate) fn default_git_icon_staged() -> Icon {
    icon("✚")
}

/// Default git unstaged files indicator
pub(crate) fn default_git_icon_unstaged() -> Icon {
    icon("✱")
}

/// Default git untracked files indicator
pub(crate) fn default_git_icon_untracked() -> Icon {
    icon("?")
}

/// Default git conflicts indicator
pub(crate) fn default_git_icon_conflicts() -> Icon {
    icon("✖")
}

/// Default git stash indicator (Unicode `icon_set`).
///
/// Nerd Font alternates for this and the two glyphs below belong with the
/// resolver that consumes `[git] icon_set` (#245); there is no render-time
/// consumer yet.
pub(crate) fn default_git_icon_stash() -> Icon {
    icon("≡")
}

/// Default detached-HEAD indicator (Unicode `icon_set`)
pub(crate) fn default_git_icon_detached() -> Icon {
    icon("➦")
}

/// Default in-progress-operation indicator (Unicode `icon_set`)
pub(crate) fn default_git_icon_in_progress() -> Icon {
    icon("↻")
}

/// Default language display mode
pub(crate) const fn default_language_show() -> crate::config::types::LanguageDisplay {
    crate::config::types::LanguageDisplay::Icon
}

/// Default language filter mode
pub(crate) const fn default_language_filter() -> crate::config::types::LanguageFilter {
    crate::config::types::LanguageFilter::All
}

/// Default enabled segments in order
pub(crate) fn default_enabled_segments() -> Vec<String> {
    vec![
        "clock".to_owned(),
        "duration".to_owned(),
        "language".to_owned(),
        "directory".to_owned(),
        "git".to_owned(),
    ]
}

/// Default delimiter background color (transparent)
pub(crate) fn default_delimiter_bg_transparent() -> ColorSpec {
    color("transparent")
}

/// Default duration threshold in milliseconds
pub(crate) const fn default_duration_threshold() -> u64 {
    100
}

/// Default success symbol for the character segment (prompt `❯`)
pub(crate) fn default_character_success_symbol() -> String {
    "❯".to_owned()
}

/// Default error symbol for the character segment (prompt `❯` — same glyph, different color)
pub(crate) fn default_character_error_symbol() -> String {
    "❯".to_owned()
}

/// Default foreground color when the last command succeeded
pub(crate) fn default_character_success_color() -> ColorSpec {
    color("green")
}

/// Default foreground color when the last command failed
pub(crate) fn default_character_error_color() -> ColorSpec {
    color("red")
}

/// Default background color for successful command status
pub(crate) fn default_status_ok_bg() -> ColorSpec {
    color("green")
}

/// Default text color for successful command status
pub(crate) fn default_status_ok_text() -> ColorSpec {
    color("black")
}

/// Default background color for failed command status
pub(crate) fn default_status_fail_bg() -> ColorSpec {
    color("red")
}

/// Default text color for failed command status
pub(crate) fn default_status_fail_text() -> ColorSpec {
    color("black")
}

/// Default user prompt icon
pub(crate) fn default_ui_prompt() -> Icon {
    icon("❯")
}

/// Default root prompt icon
pub(crate) fn default_ui_root_prompt() -> Icon {
    icon("!❯!")
}

/// Default root prompt color
pub(crate) fn default_ui_root_prompt_color() -> ColorSpec {
    color(DEFAULT_UI_ROOT_PROMPT_COLOR)
}

/// Default prompt open delimiter
pub(crate) fn default_ui_prompt_open() -> String {
    " ".to_owned()
}

/// Default prompt close delimiter
pub(crate) const fn default_ui_prompt_close() -> String {
    String::new()
}

/// Default segment open delimiter
pub(crate) fn default_ui_segment_open_delimiter() -> String {
    " ".to_owned()
}

/// Default segment close delimiter
pub(crate) fn default_ui_segment_close_delimiter() -> String {
    " ".to_owned()
}

/// Default white color
pub(crate) fn default_color_white() -> ColorSpec {
    color("white")
}

/// Default black color
pub(crate) fn default_color_black() -> ColorSpec {
    color("black")
}

/// Default red color (username segment root/sudo warning pill)
pub(crate) fn default_color_red() -> ColorSpec {
    color("red")
}

/// Default hostname trim delimiter: trims at the first `.` (FQDN -> short name).
pub(crate) fn default_hostname_trim_at() -> String {
    ".".to_owned()
}

/// Default color for the user prompt icon when not specified in theme files
pub const DEFAULT_UI_PROMPT_COLOR: &str = "white";

/// Default color for the root prompt icon when not specified in theme files
pub const DEFAULT_UI_ROOT_PROMPT_COLOR: &str = "red";

/// Static constant for default prompt open delimiter
pub const DEFAULT_PROMPT_OPEN: &str = " ";

/// Static constant for default prompt close delimiter
pub const DEFAULT_PROMPT_CLOSE: &str = "";

/// Static constant for default segment open delimiter
pub const DEFAULT_SEGMENT_OPEN: &str = " ";

/// Static constant for default segment close delimiter
pub const DEFAULT_SEGMENT_CLOSE: &str = " ";

/// Embedded content of the default theme
pub const DEFAULT_THEME_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/default.toml"
));

/// Embedded content of the text theme
pub const TEXT_THEME_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/text.toml"
));

/// Embedded content of the Starship preset theme
pub const STARSHIP_THEME_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/starship.toml"
));

/// Embedded default palette TOML (identity ANSI map).
pub const DEFAULT_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/default.toml"
));

/// Embedded `starship` palette TOML (pairs with the starship theme).
pub const STARSHIP_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/starship.toml"
));

/// Embedded `catppuccin-latte` palette TOML.
pub const CATPPUCCIN_LATTE_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/catppuccin-latte.toml"
));

/// Embedded `catppuccin-frappe` palette TOML.
pub const CATPPUCCIN_FRAPPE_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/catppuccin-frappe.toml"
));

/// Embedded `catppuccin-macchiato` palette TOML.
pub const CATPPUCCIN_MACCHIATO_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/catppuccin-macchiato.toml"
));

/// Embedded `catppuccin-mocha` palette TOML.
pub const CATPPUCCIN_MOCHA_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/catppuccin-mocha.toml"
));

/// Embedded `nord` palette TOML.
pub const NORD_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/nord.toml"
));

/// Embedded `gruvbox-dark-medium` palette TOML.
pub const GRUVBOX_DARK_MEDIUM_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/gruvbox-dark-medium.toml"
));

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    #[test]
    fn default_palette_content_parses() {
        let cfg: crate::palette::PaletteConfig =
            toml::from_str(crate::config::defaults::DEFAULT_PALETTE_CONTENT)
                .expect("parse builtin");
        assert!(cfg.colors.contains_key("red"));
    }
}
