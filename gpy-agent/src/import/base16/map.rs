//! Map a parsed base16/base24 scheme onto GPY palette role names (1:1, standard).

use super::{Base16Scheme, Hex, Result, SchemeSystem};
use crate::config::types::ColorSpec;
use crate::palette::PaletteConfig;
use std::collections::BTreeMap;

/// (slot index, GPY role) for the standard base24 accents + neutrals.
const ROLE_SLOTS: &[(u8, &str)] = &[
    (0x00, "black"),
    (0x03, "bright_black"),
    (0x05, "white"),
    (0x07, "bright_white"),
    (0x08, "red"),
    (0x09, "orange"),
    (0x0A, "yellow"),
    (0x0B, "green"),
    (0x0C, "cyan"),
    (0x0D, "blue"),
    (0x0E, "magenta"),
    (0x0F, "brown"),
    (0x12, "bright_red"),
    (0x13, "bright_yellow"),
    (0x14, "bright_green"),
    (0x15, "bright_cyan"),
    (0x16, "bright_blue"),
    (0x17, "bright_magenta"),
];

/// base16-only fallback: bright role ← accent slot when the base24 bright is absent.
const BRIGHT_FALLBACK: &[(&str, u8)] = &[
    ("bright_red", 0x08),
    ("bright_yellow", 0x0A),
    ("bright_green", 0x0B),
    ("bright_cyan", 0x0C),
    ("bright_blue", 0x0D),
    ("bright_magenta", 0x0E),
];

/// Convert a parsed scheme into a GPY [`PaletteConfig`].
///
/// # Errors
/// Returns an error if a mapped color value is not a valid [`ColorSpec`]
/// (should not happen: every value comes from a validated [`Hex`]).
pub fn to_palette_config(scheme: &Base16Scheme) -> Result<PaletteConfig> {
    let mut colors: BTreeMap<String, ColorSpec> = BTreeMap::new();

    for (index, role) in ROLE_SLOTS {
        if let Some(hex) = scheme.slots.get(index) {
            insert_role(&mut colors, role, hex)?;
        }
    }

    // base16 schemes carry no base12–17: derive bright accents from base08–0F.
    if scheme.system == SchemeSystem::Base16 {
        for (role, accent_index) in BRIGHT_FALLBACK {
            if !colors.contains_key(*role)
                && let Some(hex) = scheme.slots.get(accent_index)
            {
                insert_role(&mut colors, role, hex)?;
            }
        }
    }

    let description = scheme.variant.as_ref().map_or_else(
        || format!("Imported base scheme: {}", scheme.name),
        |variant| format!("Imported base scheme: {} ({variant})", scheme.name),
    );

    Ok(PaletteConfig {
        name: slugify(
            scheme
                .slug
                .as_deref()
                .filter(|slug| !slug.trim().is_empty())
                .unwrap_or(&scheme.name),
        ),
        description,
        colors,
    })
}

/// # Errors
/// Returns [`ImportError::Malformed`] if `ColorSpec` rejects the CSS hex string (impossible
/// in practice: every value comes from a validated [`Hex`]).
fn insert_role(colors: &mut BTreeMap<String, ColorSpec>, role: &str, hex: &Hex) -> Result<()> {
    let spec =
        ColorSpec::new(&hex.as_css()).map_err(|e| super::ImportError::Malformed(e.to_string()))?;
    colors.insert(role.to_owned(), spec);
    Ok(())
}

/// Lowercase, hyphenate, and strip unsafe characters for a palette file stem.
/// Unicode letters and digits are kept; every other run of characters becomes one `-`.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]
    #![allow(missing_docs)]
    use super::to_palette_config;
    use crate::import::base16::parse_scheme;

    const SCHEME: &str = "\
system: \"base24\"
name: \"Demo\"
palette:
  base08: \"#f38ba8\"
  base09: \"#fab387\"
  base0b: \"#a6e3a1\"
  base0e: \"#cba6f7\"
  base14: \"#a6e3a1\"
  base17: \"#cba6f7\"
";

    #[test]
    fn maps_accents_to_role_names() {
        let scheme = parse_scheme(SCHEME).unwrap();
        let cfg = to_palette_config(&scheme).unwrap();
        assert_eq!(
            cfg.colors.get("red").expect("red present").as_str(),
            "#f38ba8"
        );
        assert_eq!(
            cfg.colors.get("orange").expect("orange present").as_str(),
            "#fab387"
        );
        assert_eq!(
            cfg.colors.get("green").expect("green present").as_str(),
            "#a6e3a1"
        );
        assert_eq!(
            cfg.colors.get("magenta").expect("magenta present").as_str(),
            "#cba6f7"
        );
        assert_eq!(
            cfg.colors
                .get("bright_green")
                .expect("bright_green present")
                .as_str(),
            "#a6e3a1"
        );
        assert_eq!(
            cfg.colors
                .get("bright_magenta")
                .expect("bright_magenta present")
                .as_str(),
            "#cba6f7"
        );
    }

    #[test]
    fn import_scheme_round_trips_to_palette() {
        let cfg = crate::import::base16::import_scheme(SCHEME).unwrap();
        assert_eq!(cfg.name, "demo");
        assert!(cfg.colors.contains_key("red"));
    }

    fn rose_scheme(slug_line: &str) -> String {
        format!(
            "system: \"base16\"\nname: \"Rosé Pine\"\n{slug_line}palette:\n  base00: \"191724\"\n  base08: \"eb6f92\"\n"
        )
    }

    #[test]
    fn slug_field_is_preferred_over_name() {
        let cfg =
            crate::import::base16::import_scheme(&rose_scheme("slug: \"rose-pine\"\n")).unwrap();
        assert_eq!(cfg.name, "rose-pine");
    }

    #[test]
    fn non_ascii_name_is_kept_when_no_slug() {
        let cfg = crate::import::base16::import_scheme(&rose_scheme("")).unwrap();
        assert_eq!(cfg.name, "rosé-pine");
    }

    #[test]
    fn base16_only_scheme_derives_brights_from_accents() {
        let base16 = "system: \"base16\"\nname: \"x\"\npalette:\n  base0e: \"#cba6f7\"\n  base0b: \"#a6e3a1\"\n";
        let scheme = parse_scheme(base16).unwrap();
        let cfg = to_palette_config(&scheme).unwrap();
        // No base17/base14 present → bright_magenta/bright_green derive from base0e/base0b.
        assert_eq!(
            cfg.colors
                .get("bright_magenta")
                .expect("bright_magenta present")
                .as_str(),
            "#cba6f7"
        );
        assert_eq!(
            cfg.colors
                .get("bright_green")
                .expect("bright_green present")
                .as_str(),
            "#a6e3a1"
        );
    }
}
