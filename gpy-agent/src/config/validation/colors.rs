//! Color and icon validation helpers for themes.
//!
//! Typed theme colors and palette values accept exactly what the template style
//! parser (`parse_color`) resolves to a concrete color, plus the keywords below.

use super::{ValidationError, ValidationErrorKind};
use crate::Result;
use crate::template::{Color, config_color_name, parse_color};

/// Theme-only keywords: the shell renderers resolve them against the
/// neighbouring segment's colors; templates never see them.
const THEME_ONLY_COLORS: [&str; 2] = ["match_bg", "match_text"];

/// Accent role names every shipped palette defines; they resolve through the
/// active palette, not the ANSI table.
pub(crate) const PALETTE_ROLE_COLORS: [&str; 2] = ["orange", "brown"];

/// Canonical spelling of a theme color, or `None` if `value` is not one.
///
/// Hex becomes lowercase `#rrggbb`, an ANSI index its decimal form, an ANSI name
/// the Fish `set_color` spelling (`config_color_name`), and `default`/`transparent`
/// become `transparent`.
#[must_use]
pub fn canonical_color(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if THEME_ONLY_COLORS.contains(&lower.as_str()) || PALETTE_ROLE_COLORS.contains(&lower.as_str())
    {
        return Some(lower);
    }
    match parse_color(&lower).ok()? {
        Color::Named(name) if name == "default" => Some("transparent".to_owned()),
        Color::Named(name) => config_color_name(&name).map(str::to_owned),
        Color::Rgb { r, g, b } => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Ansi256(index) => Some(index.to_string()),
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => None,
    }
}

/// Whether `value` is a valid theme color (see [`canonical_color`]).
#[must_use]
pub fn is_valid_color(value: &str) -> bool {
    canonical_color(value).is_some()
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
    use super::{canonical_color, is_valid_color};
    use crate::template::{ANSI_BASE_WORDS, BRIGHT_PREFIXES, canonical_ansi_name};

    #[test]
    fn accepts_orange_and_brown_role_names() {
        assert!(
            is_valid_color("orange"),
            "orange must be a valid color name"
        );
        assert!(is_valid_color("brown"), "brown must be a valid color name");
    }

    /// Every ANSI spelling the style parser knows is a valid theme color whose
    /// canonical form names the same encoder color.
    #[test]
    fn every_parser_ansi_spelling_is_accepted_and_keeps_its_color() {
        let mut spellings: Vec<String> = vec!["gray".to_owned(), "grey".to_owned()];
        for word in ANSI_BASE_WORDS {
            spellings.push(word.to_owned());
            spellings.extend(
                BRIGHT_PREFIXES
                    .iter()
                    .map(|prefix| format!("{prefix}{word}")),
            );
        }
        for spelling in spellings {
            let canonical = canonical_color(&spelling.to_uppercase())
                .unwrap_or_else(|| panic!("'{spelling}' must be a valid theme color"));
            assert_eq!(
                canonical_ansi_name(&canonical),
                canonical_ansi_name(&spelling),
                "'{spelling}' canonicalized to '{canonical}', a different color"
            );
        }
    }

    #[test]
    fn hex_and_index_canonicalize_and_malformed_values_are_rejected() {
        assert_eq!(canonical_color("#A0B1C2").as_deref(), Some("#a0b1c2"));
        assert_eq!(canonical_color("042").as_deref(), Some("42"));
        assert_eq!(canonical_color("default").as_deref(), Some("transparent"));
        assert_eq!(canonical_color("Match_Bg").as_deref(), Some("match_bg"));
        for bad in [
            "##112233", "#12345", "112233", "256", "prev_fg", "nonsuch", "",
        ] {
            assert!(!is_valid_color(bad), "'{bad}' must be rejected");
        }
    }
}
