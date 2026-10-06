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

/// A named-color palette resolved at evaluation time.
///
/// The name→color map is stored behind an [`Arc`] so cloning a `Palette` (done
/// once per rendered segment on the hot path) is a refcount bump rather than a
/// deep copy of the whole map.
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

/// Parse a space-separated style spec into a [`Style`].
///
/// `none` resets to an empty style. A bare color token sets the foreground.
///
/// # Errors
///
/// Returns [`TemplateError::InvalidStyle`] / [`TemplateError::UnknownColor`].
pub fn parse_style(spec: &str) -> Result<Style> {
    let mut style = Style::default();
    for token in spec.split_whitespace() {
        if token.eq_ignore_ascii_case("none") {
            style = Style::default();
        } else if let Some(rest) = token.strip_prefix("fg:") {
            style.fg = Some(parse_color(rest)?);
        } else if let Some(rest) = token.strip_prefix("bg:") {
            style.bg = Some(parse_color(rest)?);
        } else if let Some(attr) = parse_attr(token) {
            style.attrs.push(attr);
        } else {
            style.fg = Some(parse_color(token)?);
        }
    }
    Ok(style)
}

/// Map a token to an [`Attr`], returning `None` if the token is not an attribute keyword.
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
/// `default` resolves to the terminal's own color (ANSI 39/49).
///
/// # Errors
///
/// Returns [`TemplateError::UnknownColor`] for malformed hex strings.
pub fn parse_color(text: &str) -> Result<Color> {
    match text {
        "prev_fg" => return Ok(Color::PrevFg),
        "prev_bg" => return Ok(Color::PrevBg),
        // `default` = the terminal's own fg/bg. Encoders map it to ANSI 39/49.
        // Kept consistent with eval.rs, whose prev-color fallback emits Named("default").
        //
        // `transparent` is GPY's theme spelling for "no color / terminal default".
        // The legacy renderer treats it that way, so the template engine must too:
        // otherwise a segment color of `transparent` (common in flat themes) would
        // fall through to a palette lookup and fail the whole template.
        "default" | "transparent" => return Ok(Color::Named("default".to_owned())),
        _ => {}
    }
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Ok(index) = text.parse::<u8>() {
        return Ok(Color::Ansi256(index));
    }
    if is_named_color(text) {
        return Ok(Color::Named(text.to_owned()));
    }
    // Unknown bare word: treat as a palette reference (resolved at eval time).
    Ok(Color::Palette(text.to_owned()))
}

/// Parse a 6-digit hex string (without the leading `#`) into an [`Color::Rgb`].
///
/// # Errors
///
/// Returns [`TemplateError::UnknownColor`] if the string is not exactly 6 ASCII hex digits.
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

/// Return `true` if `text` is one of the 16 standard ANSI color names.
fn is_named_color(text: &str) -> bool {
    const NAMES: &[&str] = &[
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "purple",
        "cyan",
        "white",
        "bright-black",
        "bright-red",
        "bright-green",
        "bright-yellow",
        "bright-blue",
        "bright-purple",
        "bright-cyan",
        "bright-white",
    ];
    NAMES.contains(&text)
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
    fn empty_spec_yields_empty_style() {
        assert_eq!(parse_style("").unwrap(), Style::default());
    }

    #[test]
    fn rejects_bad_hex() {
        assert!(parse_style("fg:#xyz").is_err());
    }

    #[test]
    fn parses_default_as_named_not_palette() {
        // `default` denotes the terminal's own color, not a palette lookup.
        let style = parse_style("fg:default bg:default").unwrap();
        assert_eq!(style.fg, Some(Color::Named("default".to_owned())));
        assert_eq!(style.bg, Some(Color::Named("default".to_owned())));
    }

    #[test]
    fn parses_transparent_as_terminal_default() {
        // `transparent` is GPY's flat-theme spelling for the terminal default;
        // it must not fall through to a (failing) palette lookup.
        let style = parse_style("fg:transparent bg:transparent").unwrap();
        assert_eq!(style.fg, Some(Color::Named("default".to_owned())));
        assert_eq!(style.bg, Some(Color::Named("default".to_owned())));
    }
}
