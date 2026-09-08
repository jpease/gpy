//! Typed base16/base24 scheme model + a hand-rolled parser for the narrow
//! tinted-theming YAML subset (no external YAML dependency).

use super::{Hex, ImportError, Result};
use std::collections::BTreeMap;

/// Which tinted-theming system a scheme declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeSystem {
    /// 16 slots (base00–base0F).
    Base16,
    /// 24 slots (base00–base17).
    Base24,
}

/// A parsed scheme: metadata plus slot index → color. Valid by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base16Scheme {
    /// Declared system (defaults to base16 when absent).
    pub system: SchemeSystem,
    /// Human-readable scheme name.
    pub name: String,
    /// Light/dark variant when declared.
    pub variant: Option<String>,
    /// Slot index (0x00–0x17) → color.
    pub slots: BTreeMap<u8, Hex>,
}

/// Strip surrounding quotes and inline ` #` comments from a scalar value.
fn clean_scalar(raw: &str) -> String {
    let no_comment = match raw.split_once(" #") {
        Some((head, _)) => head,
        None => raw,
    };
    no_comment
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_owned()
}

/// Parse `baseNN` → slot index (0x00–0x17). Returns `None` for non-slot keys.
fn slot_index(key: &str) -> Option<u8> {
    let digits = key.strip_prefix("base")?;
    u8::from_str_radix(digits, 16)
        .ok()
        .filter(|n| *n <= 0x17_u8)
}

/// Parse a base16/base24 scheme from the tinted-theming YAML subset.
///
/// Accepts the modern `palette:`-nested layout and the legacy flat layout.
/// Lines outside the supported grammar (other than blanks/comments) are ignored
/// at the top level but bad slot colors are rejected.
///
/// # Errors
/// Returns [`ImportError`] when no slots are found or a slot color is invalid.
pub fn parse_scheme(input: &str) -> Result<Base16Scheme> {
    let mut system = SchemeSystem::Base16;
    let mut name: Option<String> = None;
    let mut variant: Option<String> = None;
    let mut slots: BTreeMap<u8, Hex> = BTreeMap::new();

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line == "palette:" {
            continue;
        }
        let Some((key_raw, value_raw)) = line.split_once(':') else {
            continue;
        };
        let key = key_raw.trim();
        let value = clean_scalar(value_raw);
        if value.is_empty() {
            continue;
        }
        match key {
            "system" => {
                system = if value.eq_ignore_ascii_case("base24") {
                    SchemeSystem::Base24
                } else {
                    SchemeSystem::Base16
                };
            }
            "name" | "scheme" => name = Some(value),
            "variant" => variant = Some(value),
            other => {
                if let Some(index) = slot_index(other) {
                    slots.insert(index, Hex::parse(&value)?);
                }
            }
        }
    }

    if slots.is_empty() {
        return Err(ImportError::Malformed(
            "no baseNN color slots found".to_owned(),
        ));
    }
    Ok(Base16Scheme {
        system,
        name: name.unwrap_or_else(|| "imported".to_owned()),
        variant,
        slots,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    use super::{SchemeSystem, parse_scheme};

    const BASE24_NESTED: &str = "\
system: \"base24\"
name: \"Demo\"
variant: \"dark\"
palette:
  base00: \"1e1e2e\"
  base08: \"#f38ba8\"
  base09: \"#fab387\"
  base17: \"cba6f7\"
";

    const BASE16_FLAT_LEGACY: &str = "\
scheme: \"Legacy\"
base00: \"000000\"
base08: \"ff0000\"
";

    #[test]
    fn parses_nested_base24_palette() {
        let scheme = parse_scheme(BASE24_NESTED).unwrap();
        assert_eq!(scheme.system, SchemeSystem::Base24);
        assert_eq!(scheme.name, "Demo");
        assert_eq!(scheme.variant.as_deref(), Some("dark"));
        assert_eq!(
            scheme.slots.get(&0x00).expect("slot 0x00 present").as_css(),
            "#1e1e2e"
        );
        assert_eq!(
            scheme.slots.get(&0x09).expect("slot 0x09 present").as_css(),
            "#fab387"
        );
        assert_eq!(
            scheme.slots.get(&0x17).expect("slot 0x17 present").as_css(),
            "#cba6f7"
        );
    }

    #[test]
    fn parses_legacy_flat_layout() {
        let scheme = parse_scheme(BASE16_FLAT_LEGACY).unwrap();
        assert_eq!(scheme.system, SchemeSystem::Base16);
        assert_eq!(scheme.name, "Legacy");
        assert_eq!(
            scheme.slots.get(&0x08).expect("slot 0x08 present").as_css(),
            "#ff0000"
        );
    }

    #[test]
    fn rejects_bad_hex_in_slot() {
        let bad = "system: \"base16\"\nname: \"x\"\npalette:\n  base00: \"zzzzzz\"\n";
        assert!(parse_scheme(bad).is_err());
    }
}
