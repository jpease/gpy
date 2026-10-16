//! Color and icon validation helpers for themes.

use super::{ValidationError, ValidationErrorKind};
use crate::Result;

pub(crate) const NAMED_COLORS: &[&str] = &[
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "brblack",
    "gray",
    "grey",
    "bright_red",
    "brred",
    "bright_green",
    "brgreen",
    "bright_yellow",
    "bryellow",
    "bright_blue",
    "brblue",
    "bright_magenta",
    "brmagenta",
    "bright_cyan",
    "brcyan",
    "bright_white",
    "brwhite",
    "orange",
    "brown",
    // Starship hyphenated literals map straight to ANSI in `named_code`, so accept them.
    "purple",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-purple",
    "bright-cyan",
    "bright-white",
];

const MAGIC_COLORS_REF: &[&str] = &["match_bg", "match_text", "transparent"];

/// Check if a color value is valid.
/// Valid: `#RRGGBB`, `0`–`255`, named colors, `match_bg`/`match_text`/`transparent`.
#[must_use]
pub fn is_valid_color(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let lower = value.to_lowercase();
    if MAGIC_COLORS_REF.contains(&lower.as_str()) {
        return true;
    }
    if value.starts_with('#') {
        let trimmed = value.trim_start_matches('#');
        if trimmed.len() == 6 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }
    }
    if value.parse::<u8>().is_ok() {
        return true;
    }
    NAMED_COLORS.contains(&lower.as_str())
}

/// Validate a single color value.
///
/// # Errors
/// Returns a configuration error if the provided value is not a recognised color.
pub fn validate_color(value: &str, field: &str, source: &str) -> Result<()> {
    if !is_valid_color(value) {
        return Err(ValidationError {
            field_path: field.to_owned(),
            kind: ValidationErrorKind::InvalidColor {
                value: value.to_owned(),
                theme: source.to_owned(),
            },
        }
        .into());
    }
    Ok(())
}

/// Validate an icon string.
///
/// # Errors
/// Returns a configuration error if the icon contains control characters.
pub fn validate_icon(value: &str, field: &str, source: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(ValidationError {
            field_path: field.to_owned(),
            kind: ValidationErrorKind::IconControlChars {
                theme: source.to_owned(),
            },
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    use super::is_valid_color;

    #[test]
    fn accepts_orange_and_brown_role_names() {
        assert!(
            is_valid_color("orange"),
            "orange must be a valid color name"
        );
        assert!(is_valid_color("brown"), "brown must be a valid color name");
    }

    #[test]
    fn accepts_hyphenated_ansi_names_the_renderer_already_understands() {
        // Must match the spellings `template::style::named_code` accepts.
        for name in [
            "purple",
            "bright-black",
            "bright-red",
            "bright-green",
            "bright-yellow",
            "bright-blue",
            "bright-purple",
            "bright-cyan",
            "bright-white",
        ] {
            assert!(is_valid_color(name), "{name} must be a valid color name");
        }
    }
}
