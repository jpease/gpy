//! Starship `style` strings resolved to GPY colors and attribute keywords.

use crate::config::types::ColorSpec;
use crate::template::{Attr, Color, TemplateError, parse_style};
use std::collections::HashMap;
use std::hash::BuildHasher;

/// The colors of a Starship `style` string, resolved to concrete GPY colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleColor {
    /// The style parsed. `fg`/`bg` are `None` when the style sets no such color.
    Resolved {
        /// Foreground color, a valid [`ColorSpec`] spelling.
        fg: Option<String>,
        /// Background color, a valid [`ColorSpec`] spelling.
        bg: Option<String>,
    },
    /// A token is not a color GPY can represent (the offending text).
    Invalid(String),
}

/// Resolve a Starship `style`'s colors, mapping palette references through `palette`.
#[must_use]
pub fn style_color<S: BuildHasher>(
    style: &str,
    palette: Option<&HashMap<String, String, S>>,
) -> StyleColor {
    let parsed = match parse_style(style) {
        Ok(parsed) => parsed,
        Err(
            TemplateError::UnknownColor { name: token } | TemplateError::InvalidStyle { token },
        ) => return StyleColor::Invalid(token),
        Err(_) => return StyleColor::Invalid(style.to_owned()),
    };
    // `Err` carries the offending color text, which is what `StyleColor::Invalid` reports.
    let resolve = |color: Option<Color>| -> Result<Option<String>, String> {
        let Some(parsed_color) = color else {
            return Ok(None);
        };
        let text = match parsed_color {
            Color::Named(name) => name,
            Color::Palette(name) => palette.and_then(|p| p.get(&name)).cloned().unwrap_or(name),
            Color::Rgb { r, g, b } => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Ansi256(index) => index.to_string(),
            Color::PrevFg => "prev_fg".to_owned(),
            Color::PrevBg => "prev_bg".to_owned(),
        };
        if ColorSpec::new(&text).is_ok() {
            Ok(Some(text))
        } else {
            Err(text)
        }
    };
    match (resolve(parsed.fg), resolve(parsed.bg)) {
        (Ok(fg), Ok(bg)) => StyleColor::Resolved { fg, bg },
        (Err(token), _) | (_, Err(token)) => StyleColor::Invalid(token),
    }
}

/// Attribute keywords of a style string, in declaration order.
#[must_use]
pub fn attr_tokens(style: &str) -> Vec<&'static str> {
    parse_style(style)
        .map(|parsed| {
            parsed
                .attrs
                .into_iter()
                .map(|attr| match attr {
                    Attr::Bold => "bold",
                    Attr::Italic => "italic",
                    Attr::Underline => "underline",
                    Attr::Dimmed => "dimmed",
                    Attr::Inverted => "inverted",
                    Attr::Blink => "blink",
                    Attr::Hidden => "hidden",
                    Attr::Strikethrough => "strikethrough",
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{StyleColor, attr_tokens, style_color};

    /// No selected Starship palette.
    const NO_PALETTE: Option<&std::collections::HashMap<String, String>> = None;

    #[test]
    fn style_color_skips_attributes_and_strips_prefixes() {
        let resolved = |fg: Option<&str>, bg: Option<&str>| StyleColor::Resolved {
            fg: fg.map(str::to_owned),
            bg: bg.map(str::to_owned),
        };
        assert_eq!(
            style_color("bold green", NO_PALETTE),
            resolved(Some("green"), None)
        );
        assert_eq!(
            style_color("fg:green", NO_PALETTE),
            resolved(Some("green"), None)
        );
        assert_eq!(
            style_color("Bold FG:Green", NO_PALETTE),
            resolved(Some("green"), None)
        );
        assert_eq!(
            style_color("fg:#ff0000", NO_PALETTE),
            resolved(Some("#ff0000"), None)
        );
        assert_eq!(
            style_color("bold fg:202", NO_PALETTE),
            resolved(Some("202"), None)
        );
        assert_eq!(
            style_color("bg:blue fg:white", NO_PALETTE),
            resolved(Some("white"), Some("blue"))
        );
        assert_eq!(
            style_color("bg:#212736", NO_PALETTE),
            resolved(None, Some("#212736"))
        );
        assert_eq!(style_color("bold", NO_PALETTE), resolved(None, None));
        assert_eq!(style_color("none", NO_PALETTE), resolved(None, None));
    }

    #[test]
    fn style_color_resolves_palette_aliases_and_rejects_unknown_names() {
        let palette: std::collections::HashMap<String, String> = [
            ("peach".to_owned(), "#fab387".to_owned()),
            ("green".to_owned(), "#00ff00".to_owned()),
        ]
        .into();
        assert_eq!(
            style_color("bold peach", Some(&palette)),
            StyleColor::Resolved {
                fg: Some("#fab387".to_owned()),
                bg: None
            }
        );
        assert_eq!(
            style_color("bg:peach", Some(&palette)),
            StyleColor::Resolved {
                fg: None,
                bg: Some("#fab387".to_owned())
            }
        );
        // Shadowing a builtin color name is the palette vocabulary's concern, not an alias.
        assert_eq!(
            style_color("green", Some(&palette)),
            StyleColor::Resolved {
                fg: Some("green".to_owned()),
                bg: None
            }
        );
        assert_eq!(
            style_color("peach", NO_PALETTE),
            StyleColor::Invalid("peach".to_owned())
        );
        assert_eq!(
            style_color("fg:#12345", NO_PALETTE),
            StyleColor::Invalid("#12345".to_owned())
        );
    }

    #[test]
    fn attr_tokens_returns_attributes_only() {
        assert_eq!(attr_tokens("red dimmed"), vec!["dimmed"]);
        assert_eq!(attr_tokens("bold italic red"), vec!["bold", "italic"]);
        assert_eq!(attr_tokens("red"), Vec::<&str>::new());
        assert_eq!(attr_tokens("bold"), vec!["bold"]);
    }

    #[test]
    fn attr_tokens_are_case_insensitive() {
        assert_eq!(attr_tokens("Bold green"), vec!["bold"]);
        assert_eq!(
            attr_tokens("ITALIC Underline red"),
            vec!["italic", "underline"]
        );
    }
}
