//! Palette configuration type and TOML model.

use crate::config::types::ColorSpec;
use crate::template::{Color, Palette, parse_style};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A named color palette: `color-name -> concrete color value`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PaletteConfig {
    /// Palette name (informational; discovery uses the file stem).
    #[serde(default)]
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Color definitions. Values are concrete colors (hex / ANSI index / named).
    #[serde(default)]
    pub colors: BTreeMap<String, ColorSpec>,
}

impl PaletteConfig {
    /// Convert to a [`crate::template::Palette`] for the template engine.
    ///
    /// Each entry's value string is parsed through the style parser so hex,
    /// ANSI index, and named colors all resolve. Unparseable values are skipped
    /// (validation rejects them on load, so this is defensive).
    #[must_use]
    pub fn to_template_palette(&self) -> Palette {
        let map: HashMap<String, Color> = self
            .colors
            .iter()
            .filter_map(|(key, value)| {
                parse_style(value.as_str())
                    .ok()
                    .and_then(|style| style.fg)
                    .map(|color| (key.clone(), color))
            })
            .collect();
        Palette::new(map)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::PaletteConfig;
    use crate::template::Color;

    #[test]
    fn parses_colors_table_and_ignores_unknown_keys() {
        let toml_src = "\
name = \"nord\"
description = \"Nord\"
extra_unknown = 5
[colors]
frost = \"#88c0d0\"
accent = \"blue\"
idx = \"42\"
";
        let cfg: PaletteConfig = toml::from_str(toml_src).expect("parse");
        assert_eq!(cfg.name, "nord");
        assert_eq!(cfg.colors.len(), 3);
        assert_eq!(
            cfg.colors.get("frost").expect("frost key present").as_str(),
            "#88c0d0"
        );
    }

    #[test]
    fn rejects_invalid_color_value() {
        let toml_src = "[colors]\nbad = \"notacolor!!!\"\n";
        let parsed: Result<PaletteConfig, _> = toml::from_str(toml_src);
        assert!(parsed.is_err());
    }

    #[test]
    fn to_template_palette_maps_hex_index_named() {
        let toml_src = "[colors]\nfrost = \"#88c0d0\"\nidx = \"42\"\naccent = \"blue\"\n";
        let cfg: PaletteConfig = toml::from_str(toml_src).expect("parse");
        let pal = cfg.to_template_palette();
        assert_eq!(
            pal.get("frost"),
            Some(Color::Rgb {
                r: 0x88,
                g: 0xc0,
                b: 0xd0
            })
        );
        assert_eq!(pal.get("idx"), Some(Color::Ansi256(42)));
        assert_eq!(pal.get("accent"), Some(Color::Named("blue".to_owned())));
    }
}
