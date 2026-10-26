//! Palette configuration type and TOML model.

use crate::config::types::ColorSpec;
use crate::template::{Color, Palette, alias_owner, canonical_ansi_name, parse_style};
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
    /// Color definitions: hex, ANSI index, named, or another palette key.
    #[serde(default)]
    pub colors: BTreeMap<String, ColorSpec>,
}

impl PaletteConfig {
    /// Convert to a template [`Palette`], resolving key references; unresolvable entries are skipped.
    #[must_use]
    pub fn to_template_palette(&self) -> Palette {
        Palette::new(self.resolve_entries().0)
    }

    /// Check that every entry resolves to a concrete color.
    ///
    /// # Errors
    /// Returns an error naming the first cyclic, dangling, or unparseable entry.
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

    /// One message per ANSI color defined under several keys (`magenta` and
    /// `purple`): each key still answers its own spelling, and every other
    /// spelling of that color uses the key `alias_owner` picks.
    #[must_use]
    pub fn alias_warnings(&self) -> Vec<String> {
        let mut groups: BTreeMap<&'static str, Vec<&str>> = BTreeMap::new();
        for key in self.colors.keys() {
            if let Some(canonical) = canonical_ansi_name(key) {
                groups.entry(canonical).or_default().push(key);
            }
        }
        groups
            .into_iter()
            .filter(|(_, keys)| keys.len() > 1)
            .filter_map(|(canonical, keys)| {
                let owner = alias_owner(canonical, keys.iter().copied())?;
                Some(format!(
                    "colors {} all name the ANSI color {canonical}; each key keeps its own \
                     spelling, and other spellings of {canonical} use '{owner}'",
                    keys.iter()
                        .map(|key| format!("'{key}'"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
            .collect()
    }

    /// Resolve every entry; returns resolved colors plus one message per failure.
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::PaletteConfig;
    use crate::config::validation::colors::PALETTE_ROLE_COLORS;
    use crate::formatter::encode_ansi;
    use crate::palette::manager::BUILTIN_PALETTES;
    use crate::template::style::{ANSI_BASE_WORDS, BRIGHT_PREFIXES, is_named_color};
    use crate::template::{Color, MapResolver, RenderContext, canonical_ansi_name, render};

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

    /// Every ANSI spelling renders under every shipped palette, all aliases of one
    /// ANSI color resolve to the same palette entry, and every palette defines the
    /// role names theme colors may use.
    #[test]
    fn named_spellings_and_shipped_palettes_agree() {
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
                let canonical = canonical_ansi_name(&name).expect("named spelling");
                assert_eq!(
                    palette.get(&name),
                    palette.get(canonical),
                    "palette '{palette_name}': alias '{name}' and '{canonical}' disagree"
                );
            }
            for role in PALETTE_ROLE_COLORS {
                assert!(
                    palette.get(role).is_some(),
                    "palette '{palette_name}' must define role color '{role}'"
                );
            }
        }
    }

    #[test]
    fn palette_with_aliased_keys_loads_and_keeps_each_exact_spelling() {
        let cfg: PaletteConfig = toml::from_str(
            "name = \"dup\"\n[colors]\nmagenta = \"#111111\"\npurple = \"#222222\"\n\
             bright_magenta = \"#333333\"\nbrmagenta = \"#444444\"\n",
        )
        .expect("parse");
        cfg.validate()
            .expect("aliased keys must not fail palette loading");

        let resolver = MapResolver::from_pairs([("x", "x")]);
        let ctx = RenderContext::new(&resolver).with_palette(cfg.to_template_palette());
        let fg = |spelling: &str| {
            render(&format!("[$x](fg:{spelling})"), &ctx)
                .unwrap()
                .first()
                .and_then(|span| span.style.fg.clone())
        };
        let rgb = |v: u8| Some(Color::Rgb { r: v, g: v, b: v });
        // Exact keys render as they did before aliasing existed.
        assert_eq!(fg("magenta"), rgb(0x11));
        assert_eq!(fg("purple"), rgb(0x22));
        assert_eq!(fg("bright_magenta"), rgb(0x33));
        assert_eq!(fg("brmagenta"), rgb(0x44));
        // Other spellings: the canonical-spelled key, else the lexically smallest.
        assert_eq!(fg("bright-purple"), rgb(0x33));
        assert_eq!(fg("bright_purple"), rgb(0x33));

        let warnings = cfg.alias_warnings();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("'magenta'") && w.contains("use 'purple'")),
            "{warnings:?}"
        );
    }

    #[test]
    fn typed_alias_spelling_reaches_palette_entry_for_any_spelling() {
        let cfg: PaletteConfig =
            toml::from_str("[colors]\nbright_green = \"#00ff00\"\n").expect("parse");
        let palette = cfg.to_template_palette();
        for spelling in ["bright_green", "brgreen", "bright-green"] {
            assert_eq!(
                palette.get(spelling),
                Some(Color::Rgb { r: 0, g: 255, b: 0 }),
                "'{spelling}' must reach the palette's bright_green entry"
            );
        }
    }
}
