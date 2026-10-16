//! Style model and token-string parser (`bold cyan`, `fg:#ff0000`, `bg:42`, ...).

use crate::template::{Result, TemplateError};
use std::collections::HashMap;
use std::sync::Arc;

/// A resolvable color value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Color {
    /// A named ANSI color (e.g. `red`, `bright-green`).
    Named(String),
    /// A 24-bit hex color.
    Rgb {
        /// Red channel.
        r: u8,
        /// Green channel.
        g: u8,
        /// Blue channel.
        b: u8,
    },
    /// A 0–255 palette index.
    Ansi256(u8),
    /// A reference into the active theme palette by name.
    Palette(String),
    /// The foreground color of the previous segment.
    PrevFg,
    /// The background color of the previous segment.
    PrevBg,
}

/// A text attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attr {
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// Underline.
    Underline,
    /// Dimmed/faint.
    Dimmed,
    /// Reverse video.
    Inverted,
    /// Blink.
    Blink,
    /// Hidden.
    Hidden,
    /// Strikethrough.
    Strikethrough,
}

/// A parsed style: optional fg/bg colors plus attributes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    /// Foreground color.
    pub fg: Option<Color>,
    /// Background color.
    pub bg: Option<Color>,
    /// Text attributes, in declaration order.
    pub attrs: Vec<Attr>,
}

/// A named-color palette resolved at evaluation time; `Arc` makes per-segment clones cheap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette {
    map: Arc<HashMap<String, Color>>,
}

impl Palette {
    /// Build a palette from a name→color map.
    #[must_use]
    pub fn new(map: HashMap<String, Color>) -> Self {
        Self { map: Arc::new(map) }
    }

    /// Look up a palette color by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Color> {
        self.map.get(name).cloned()
    }
}

/// Parse a space-separated style spec into a [`Style`], following Starship's
/// `parse_style_string` (case-insensitive except palette names; `none` clears all).
///
/// # Errors
/// Returns [`TemplateError::InvalidStyle`] / [`TemplateError::UnknownColor`].
pub fn parse_style(spec: &str) -> Result<Style> {
    let mut style = Style::default();
    for token in spec.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower == "none" || lower == "fg:none" {
            return Ok(Style::default());
        }
        if lower == "bg:none" {
            style.bg = None;
        } else if lower.starts_with("fg:") {
            style.fg = Some(parse_color(token.get(3_usize..).unwrap_or_default())?);
        } else if lower.starts_with("bg:") {
            style.bg = Some(parse_color(token.get(3_usize..).unwrap_or_default())?);
        } else if let Some(attr) = parse_attr(&lower) {
            style.attrs.push(attr);
        } else {
            style.fg = Some(parse_color(token)?);
        }
    }
    Ok(style)
}

/// Map a lowercased token to an [`Attr`], returning `None` if it is not an attribute keyword.
fn parse_attr(token: &str) -> Option<Attr> {
    match token {
        "bold" => Some(Attr::Bold),
        "italic" => Some(Attr::Italic),
        "underline" => Some(Attr::Underline),
        "dimmed" => Some(Attr::Dimmed),
        "inverted" => Some(Attr::Inverted),
        "blink" => Some(Attr::Blink),
        "hidden" => Some(Attr::Hidden),
        "strikethrough" => Some(Attr::Strikethrough),
        _ => None,
    }
}

/// Parse a color value from a bare token (no `fg:`/`bg:` prefix).
///
/// # Errors
/// Returns [`TemplateError::UnknownColor`] for malformed hex strings.
pub fn parse_color(text: &str) -> Result<Color> {
    let lower = text.to_ascii_lowercase();
    match lower.as_str() {
        "prev_fg" => return Ok(Color::PrevFg),
        "prev_bg" => return Ok(Color::PrevBg),
        // `default`/`transparent` = terminal fg/bg (ANSI 39/49), never a palette lookup.
        "default" | "transparent" => return Ok(Color::Named("default".to_owned())),
        _ => {}
    }
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Ok(index) = text.parse::<u8>() {
        return Ok(Color::Ansi256(index));
    }
    if is_named_color(&lower) {
        return Ok(Color::Named(lower));
    }
    // Unknown bare word: palette reference; keys are case-sensitive.
    Ok(Color::Palette(text.to_owned()))
}

/// Parse a 6-digit hex string (without the leading `#`) into an [`Color::Rgb`].
///
/// # Errors
/// Returns [`TemplateError::UnknownColor`] unless exactly 6 ASCII hex digits.
fn parse_hex(hex: &str) -> Result<Color> {
    if hex.len() != 6_usize || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(TemplateError::UnknownColor {
            name: format!("#{hex}"),
        });
    }
    let channel = |range: std::ops::Range<usize>| -> Result<u8> {
        let slice = hex.get(range).ok_or_else(|| TemplateError::UnknownColor {
            name: format!("#{hex}"),
        })?;
        u8::from_str_radix(slice, 16).map_err(|_| TemplateError::UnknownColor {
            name: format!("#{hex}"),
        })
    };
    Ok(Color::Rgb {
        r: channel(0..2)?,
        g: channel(2..4)?,
        b: channel(4..6)?,
    })
}

/// Base ANSI color words; `magenta` is the config/Fish spelling of `purple`.
pub const ANSI_BASE_WORDS: [&str; 9] = [
    "black", "red", "green", "yellow", "blue", "purple", "magenta", "cyan", "white",
];

/// Bright-variant prefixes: Starship `bright-X`, config/Fish `bright_X`/`brX`.
pub const BRIGHT_PREFIXES: [&str; 3] = ["bright-", "bright_", "br"];

/// Map a lowercase ANSI color spelling (incl. bright variants, `gray`/`grey`) to
/// its canonical encoder name, or `None`.
#[must_use]
pub fn canonical_ansi_name(name: &str) -> Option<&'static str> {
    if matches!(name, "gray" | "grey") {
        return Some("bright-black");
    }
    let (bright, word) = BRIGHT_PREFIXES
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .filter(|word| ANSI_BASE_WORDS.contains(word))
        .map_or((false, name), |word| (true, word));
    Some(match (bright, word) {
        (false, "black") => "black",
        (false, "red") => "red",
        (false, "green") => "green",
        (false, "yellow") => "yellow",
        (false, "blue") => "blue",
        (false, "purple" | "magenta") => "purple",
        (false, "cyan") => "cyan",
        (false, "white") => "white",
        (true, "black") => "bright-black",
        (true, "red") => "bright-red",
        (true, "green") => "bright-green",
        (true, "yellow") => "bright-yellow",
        (true, "blue") => "bright-blue",
        (true, "purple" | "magenta") => "bright-purple",
        (true, "cyan") => "bright-cyan",
        (true, "white") => "bright-white",
        _ => return None,
    })
}

/// Return `true` if `text` is an ANSI color spelling (see [`canonical_ansi_name`]).
pub fn is_named_color(text: &str) -> bool {
    canonical_ansi_name(text).is_some()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{Attr, Color, Style, parse_style};

    #[test]
    fn parses_attribute_and_bare_fg() {
        let style = parse_style("bold green").unwrap();
        assert_eq!(
            style,
            Style {
                fg: Some(Color::Named("green".to_owned())),
                bg: None,
                attrs: vec![Attr::Bold],
            }
        );
    }

    #[test]
    fn parses_fg_and_bg_prefixes() {
        let style = parse_style("fg:#ff0000 bg:blue").unwrap();
        assert_eq!(style.fg, Some(Color::Rgb { r: 255, g: 0, b: 0 }));
        assert_eq!(style.bg, Some(Color::Named("blue".to_owned())));
    }

    #[test]
    fn parses_ansi256_and_prev_refs() {
        let style = parse_style("fg:42 bg:prev_bg").unwrap();
        assert_eq!(style.fg, Some(Color::Ansi256(42)));
        assert_eq!(style.bg, Some(Color::PrevBg));
    }

    #[test]
    fn none_yields_empty_style() {
        assert_eq!(parse_style("none").unwrap(), Style::default());
    }

    #[test]
    fn tokens_are_case_insensitive() {
        let expected = Style {
            fg: Some(Color::Named("red".to_owned())),
            bg: None,
            attrs: vec![Attr::Bold],
        };
        assert_eq!(parse_style("Bold Red").unwrap(), expected);
        assert_eq!(parse_style("BOLD red").unwrap(), expected);
    }

    #[test]
    fn bg_none_resets_background() {
        assert_eq!(
            parse_style("fg:red bg:none").unwrap(),
            Style {
                fg: Some(Color::Named("red".to_owned())),
                bg: None,
                attrs: vec![],
            }
        );
    }

    #[test]
    fn none_overrides_all_tokens() {
        assert_eq!(
            parse_style("fg:red none fg:blue").unwrap(),
            Style::default()
        );
        assert_eq!(parse_style("fg:none bg:black").unwrap(), Style::default());
        assert_eq!(parse_style("bold FG:None red").unwrap(), Style::default());
    }

    #[test]
    fn palette_reference_keeps_original_case() {
        let style = parse_style("fg:MyPaletteKey").unwrap();
        assert_eq!(style.fg, Some(Color::Palette("MyPaletteKey".to_owned())));
    }

    #[test]
    fn empty_spec_yields_empty_style() {
        assert_eq!(parse_style("").unwrap(), Style::default());
    }

    #[test]
    fn rejects_bad_hex() {
        assert!(parse_style("fg:#xyz").is_err());
    }

    #[test]
    fn parses_default_as_named_not_palette() {
        let style = parse_style("fg:default bg:default").unwrap();
        assert_eq!(style.fg, Some(Color::Named("default".to_owned())));
        assert_eq!(style.bg, Some(Color::Named("default".to_owned())));
    }

    #[test]
    fn parses_transparent_as_terminal_default() {
        let style = parse_style("fg:transparent bg:transparent").unwrap();
        assert_eq!(style.fg, Some(Color::Named("default".to_owned())));
        assert_eq!(style.bg, Some(Color::Named("default".to_owned())));
    }
}
