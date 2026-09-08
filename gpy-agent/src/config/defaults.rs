//! Default configuration values and constants
//!
//! This module contains all default values used throughout the configuration system.
//! These functions are used by serde's `#[serde(default = "...")]` attributes and
//! by the `Default` trait implementations for configuration structs.

// ============================================================================
// Boolean defaults
// ============================================================================

/// Default value for boolean flags (enabled)
pub(crate) const fn default_true() -> bool {
    true
}

// ============================================================================
// Git defaults
// ============================================================================

/// Default maximum ahead/behind count (0 = unlimited)
pub(crate) const fn default_max_ahead_behind() -> usize {
    100
}

use crate::config::types::{ColorSpec, Icon};

// ============================================================================
// Git icon defaults
// ============================================================================

/// Default git ahead indicator
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_ahead() -> Icon {
    Icon::new("↑").expect("Default icon must be valid")
}

/// Default git behind indicator
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_behind() -> Icon {
    Icon::new("↓").expect("Default icon must be valid")
}

/// Default git staged files indicator
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_staged() -> Icon {
    Icon::new("✚").expect("Default icon must be valid")
}

/// Default git unstaged files indicator
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_unstaged() -> Icon {
    Icon::new("✱").expect("Default icon must be valid")
}

/// Default git untracked files indicator
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_untracked() -> Icon {
    Icon::new("?").expect("Default icon must be valid")
}

/// Default git conflicts indicator
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_conflicts() -> Icon {
    Icon::new("✖").expect("Default icon must be valid")
}

/// Default git stash indicator (Unicode `icon_set`).
///
/// Nerd Font alternates for this and the two glyphs below are added when the
/// resolver consumes `[git] icon_set` (#245) — wiring icon-set selection in
/// before there is a render-time consumer would leave dead code.
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_stash() -> Icon {
    Icon::new("≡").expect("Default icon must be valid")
}

/// Default detached-HEAD indicator (Unicode `icon_set`)
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_detached() -> Icon {
    Icon::new("➦").expect("Default icon must be valid")
}

/// Default in-progress-operation indicator (Unicode `icon_set`)
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_git_icon_in_progress() -> Icon {
    Icon::new("↻").expect("Default icon must be valid")
}

// ============================================================================
// Language defaults
// ============================================================================

/// Default language display mode
pub(crate) const fn default_language_show() -> crate::config::types::LanguageDisplay {
    crate::config::types::LanguageDisplay::Icon
}

/// Default language filter mode
pub(crate) const fn default_language_filter() -> crate::config::types::LanguageFilter {
    crate::config::types::LanguageFilter::All
}

// ============================================================================
// UI defaults
// ============================================================================

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

// ============================================================================
// Delimiter defaults
// ============================================================================

/// Default delimiter background color (transparent)
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_delimiter_bg_transparent() -> ColorSpec {
    ColorSpec::new("transparent").expect("Hardcoded default color must be valid")
}

// ============================================================================
// Duration defaults
// ============================================================================

/// Default duration threshold in milliseconds
pub(crate) const fn default_duration_threshold() -> u64 {
    100
}

// ============================================================================
// Character segment defaults
// ============================================================================

/// Default success symbol for the character segment (prompt `❯`)
pub(crate) fn default_character_success_symbol() -> String {
    "❯".to_owned()
}

/// Default error symbol for the character segment (prompt `❯` — same glyph, different color)
pub(crate) fn default_character_error_symbol() -> String {
    "❯".to_owned()
}

/// Default foreground color when the last command succeeded
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_character_success_color() -> ColorSpec {
    ColorSpec::new("green").expect("Hardcoded default color must be valid")
}

/// Default foreground color when the last command failed
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_character_error_color() -> ColorSpec {
    ColorSpec::new("red").expect("Hardcoded default color must be valid")
}

// ============================================================================
// Status segment defaults
// ============================================================================

/// Default background color for successful command status
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_status_ok_bg() -> ColorSpec {
    ColorSpec::new("green").expect("Hardcoded default color must be valid")
}

/// Default text color for successful command status
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_status_ok_text() -> ColorSpec {
    ColorSpec::new("black").expect("Hardcoded default color must be valid")
}

/// Default background color for failed command status
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_status_fail_bg() -> ColorSpec {
    ColorSpec::new("red").expect("Hardcoded default color must be valid")
}

/// Default text color for failed command status
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_status_fail_text() -> ColorSpec {
    ColorSpec::new("black").expect("Hardcoded default color must be valid")
}

// ============================================================================
// UI theme defaults
// ============================================================================

/// Default user prompt icon
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_ui_prompt() -> Icon {
    Icon::new("❯").expect("Default icon must be valid")
}

/// Default root prompt icon
///
/// # Panics
/// Panics if the default icon is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_ui_root_prompt() -> Icon {
    Icon::new("!❯!").expect("Default icon must be valid")
}

/// Default root prompt color
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_ui_root_prompt_color() -> ColorSpec {
    ColorSpec::new(DEFAULT_UI_ROOT_PROMPT_COLOR).expect("Hardcoded default color must be valid")
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

// ============================================================================
// Color defaults
// ============================================================================

/// Default white color
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_color_white() -> ColorSpec {
    ColorSpec::new("white").expect("Hardcoded default color must be valid")
}

/// Default black color
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_color_black() -> ColorSpec {
    ColorSpec::new("black").expect("Hardcoded default color must be valid")
}

/// Default red color (username segment root/sudo warning pill)
///
/// # Panics
/// Panics if the default color is invalid.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn default_color_red() -> ColorSpec {
    ColorSpec::new("red").expect("Hardcoded default color must be valid")
}

// ============================================================================
// Hostname defaults
// ============================================================================

/// Default hostname trim delimiter: trims at the first `.` (FQDN -> short name).
pub(crate) fn default_hostname_trim_at() -> String {
    ".".to_owned()
}

// ============================================================================
// Public constants
// ============================================================================

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

// ============================================================================
// Embedded Theme Contents
// ============================================================================

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

/// Embedded default palette TOML (identity ANSI map until SP2 finalizes it).
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
