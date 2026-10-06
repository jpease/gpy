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
    /// ANSI index, and named colors all resolve. A value that names another
    /// palette key (`orange = "brown"`) is replaced by that key's color.
    /// Entries that cannot be turned into a concrete color are skipped
    /// ([`Self::validate`] rejects them on load, so this is defensive).
    #[must_use]
    pub fn to_template_palette(&self) -> Palette {
        Palette::new(self.resolve_entries().0)
    }

    /// Check that every entry resolves to a concrete color.
    ///
    /// # Errors
    ///
    /// Returns a configuration error naming the first entry whose reference
    /// chain is cyclic (`orange -> brown -> orange`), dangling, or unparseable.
    pub fn validate(&self) -> crate::Result<()> {
        self.resolve_entries()
            .1
            .into_iter()
            .next()
            .map_or(Ok(()), |message| {
                Err(crate::Error::config(format!(
                    "Invalid palette '{}': {message}",
                    self.name
                )))
            })
    }

    /// Parse every entry, then follow palette references to a concrete color.
    ///
    /// Returns the resolved entries plus one message per entry that could not
    /// be resolved (those entries are absent from the map).
    fn resolve_entries(&self) -> (HashMap<String, Color>, Vec<String>) {
        let mut parsed: HashMap<&str, Color> = HashMap::new();
        let mut problems: Vec<String> = Vec::new();
        for (key, value) in &self.colors {
            match parse_style(value.as_str()).ok().and_then(|style| style.fg) {
                Some(color) => {
                    parsed.insert(key.as_str(), color);
                }
                None => problems.push(format!("color '{key}' has unusable value '{value}'")),
            }
        }
        let mut resolved: HashMap<String, Color> = HashMap::new();
        for (key, color) in &parsed {
            match Self::follow_references(key, color, &parsed) {
                Ok(concrete) => {
                    resolved.insert((*key).to_owned(), concrete);
                }
                Err(message) => problems.push(message),
            }
        }
        problems.sort();
        (resolved, problems)
    }

    /// Follow `color` through `parsed` while it is a palette reference.
    ///
    /// # Errors
    ///
    /// Returns a message naming the chain when it cycles or dangles.
    fn follow_references(
        key: &str,
        color: &Color,
        parsed: &HashMap<&str, Color>,
    ) -> Result<Color, String> {
        let mut chain: Vec<&str> = vec![key];
        let mut current = color;
        loop {
            let Color::Palette(target) = current else {
                return Ok(current.clone());
            };
            if chain.contains(&target.as_str()) {
                chain.push(target);
                return Err(format!("color reference cycle: {}", chain.join(" -> ")));
            }
            chain.push(target);
            current = parsed.get(target.as_str()).ok_or_else(|| {
                format!(
                    "color '{key}' refers to '{target}', which is not a palette color ({})",
                    chain.join(" -> ")
                )
            })?;
        }
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
    use crate::config::validation::colors::NAMED_COLORS;
    use crate::formatter::encode_ansi;
    use crate::palette::manager::BUILTIN_PALETTES;
    use crate::template::{
        ANSI_BASE_WORDS, BRIGHT_PREFIXES, Color, MapResolver, RenderContext, is_named_color, render,
    };

    /// Config-vocabulary names that a palette (not the template engine) defines.
    const PALETTE_DEFINED_NAMES: [&str; 2] = ["orange", "brown"];

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

    fn builtin_configs() -> Vec<(&'static str, PaletteConfig)> {
        BUILTIN_PALETTES
            .iter()
            .map(|(name, content)| (*name, toml::from_str(content).expect("builtin parses")))
            .collect()
    }

    #[test]
    fn builtin_palettes_resolve_every_entry_to_a_concrete_color() {
        for (name, cfg) in builtin_configs() {
            cfg.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
            let palette = cfg.to_template_palette();
            for key in cfg.colors.keys() {
                assert!(
                    matches!(palette.get(key), Some(color) if !matches!(color, Color::Palette(_))),
                    "palette '{name}' key '{key}' must resolve to a concrete color, got {:?}",
                    palette.get(key)
                );
            }
        }
    }

    #[test]
    fn palette_reference_cycle_is_rejected() {
        let cfg: PaletteConfig =
            toml::from_str("name = \"loop\"\n[colors]\norange = \"brown\"\nbrown = \"orange\"\n")
                .expect("parse");
        let message = cfg.validate().unwrap_err().to_string();
        assert!(message.contains("cycle"), "{message}");
        assert!(
            message.contains("orange -> brown -> orange")
                || message.contains("brown -> orange -> brown"),
            "{message}"
        );
        let selfref: PaletteConfig =
            toml::from_str("[colors]\norange = \"orange\"\n").expect("parse");
        assert!(selfref.validate().is_err());
    }

    #[test]
    fn palette_reference_to_another_key_is_followed() {
        let cfg: PaletteConfig =
            toml::from_str("[colors]\norange = \"brown\"\nbrown = \"130\"\n").expect("parse");
        cfg.validate().expect("valid");
        assert_eq!(
            cfg.to_template_palette().get("orange"),
            Some(Color::Ansi256(130))
        );
    }

    /// Every spelling `is_named_color` is meant to accept, built from the same
    /// base words and prefixes the matcher uses, plus the gray aliases.
    fn named_spellings() -> Vec<String> {
        let mut names: Vec<String> = vec!["gray".to_owned(), "grey".to_owned()];
        for word in ANSI_BASE_WORDS {
            names.push(word.to_owned());
            for prefix in BRIGHT_PREFIXES {
                names.push(format!("{prefix}{word}"));
            }
        }
        names
    }

    /// Two-way check tying the hand-kept named-color list to the shipped palettes.
    ///
    /// (1) every name the template engine treats as a builtin color renders to a
    /// real SGR color under every shipped palette;
    /// (2) every palette key that shadows a builtin color name is on that list,
    /// as is every config-vocabulary color name (bar the palette-defined ones).
    #[test]
    fn named_color_list_and_shipped_palettes_agree() {
        let resolver = MapResolver::from_pairs([("k", "v")]);
        for (palette_name, cfg) in builtin_configs() {
            let palette = cfg.to_template_palette();
            for name in named_spellings() {
                assert!(is_named_color(&name), "'{name}' must be a named color");
                let ctx = RenderContext::new(&resolver).with_palette(palette.clone());
                let spans = render(&format!("[x](fg:{name} bg:{name})"), &ctx)
                    .unwrap_or_else(|e| panic!("{palette_name}/{name}: {e}"));
                let sgr = encode_ansi(&spans);
                assert!(
                    sgr.starts_with("\x1b["),
                    "'{name}' renders no color under palette '{palette_name}': {sgr:?}"
                );
            }
            for key in cfg.colors.keys() {
                let shadows_builtin = NAMED_COLORS.contains(&key.as_str())
                    && !PALETTE_DEFINED_NAMES.contains(&key.as_str());
                if shadows_builtin || named_spellings().contains(key) {
                    assert!(
                        is_named_color(key),
                        "palette '{palette_name}' key '{key}' shadows a builtin color name \
                         but is missing from the named-color list"
                    );
                }
            }
        }
        for name in NAMED_COLORS {
            assert_eq!(
                is_named_color(name),
                !PALETTE_DEFINED_NAMES.contains(name),
                "config color name '{name}' and the named-color list disagree"
            );
        }
        for name in PALETTE_DEFINED_NAMES {
            assert!(NAMED_COLORS.contains(&name));
        }
    }
}
