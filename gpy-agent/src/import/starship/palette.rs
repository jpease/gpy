//! Translate a Starship `[palettes.*]` selection into a GPY `PaletteConfig`.

use crate::config::types::ColorSpec;
use crate::import::starship::model::StarshipConfig;
use crate::import::starship::{WarningKind, Warnings};
use crate::palette::config::PaletteConfig;
use std::collections::{BTreeMap, HashMap};

/// Resolve the active Starship palette (`palette = "x"` → `[palettes.x]`).
///
/// Warns when the source defines no palette so callers know the emitted palette
/// is built solely from discovered module colors.
pub fn selected_palette<'a>(
    model: &'a StarshipConfig,
    warnings: &mut Warnings,
) -> Option<&'a HashMap<String, String>> {
    let Some(name) = model.palette.as_deref() else {
        warnings.push(
            WarningKind::UnrepresentableOption,
            "source defined no palette; emitting colors discovered from modules only",
        );
        return None;
    };
    model.palettes.get(name).map_or_else(
        || {
            warnings.push(
                WarningKind::UnrepresentableOption,
                format!("palette = \"{name}\" has no matching [palettes.{name}] table"),
            );
            None
        },
        Some,
    )
}

/// Build a [`PaletteConfig`] from the selected Starship palette plus colors
/// discovered during module translation. Invalid colors are dropped with a warning.
#[must_use]
pub fn translate_palette<S: ::std::hash::BuildHasher>(
    name: &str,
    selected: Option<&HashMap<String, String, S>>,
    discovered: &BTreeMap<String, String>,
    warnings: &mut Warnings,
) -> PaletteConfig {
    let mut colors: BTreeMap<String, ColorSpec> = BTreeMap::new();
    // `sink` is passed the caller's `warnings` at each call site; naming it
    // distinctly keeps it from shadowing the outer binding.
    let mut insert =
        |color_name: &str, value: &str, sink: &mut Warnings| match ColorSpec::new(value) {
            Ok(spec) => {
                colors.insert(color_name.to_owned(), spec);
            }
            Err(_) => sink.push(
                WarningKind::InvalidColor,
                format!("palette color '{color_name}' = '{value}' is not a valid color; skipped"),
            ),
        };
    if let Some(selected_colors) = selected {
        let mut sorted: Vec<(&String, &String)> = selected_colors.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (color_name, value) in sorted {
            insert(color_name, value, warnings);
        }
    }
    for (color_name, value) in discovered {
        insert(color_name, value, warnings);
    }
    PaletteConfig {
        name: name.to_owned(),
        description: "Imported from Starship".to_owned(),
        colors,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{selected_palette, translate_palette};
    use crate::config::types::ColorSpec;
    use crate::import::starship::model::parse;
    use crate::import::starship::{WarningKind, Warnings};
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn selects_named_palette() {
        let model = parse("palette = \"nord\"\n[palettes.nord]\nfrost = \"#88c0d0\"\n").unwrap();
        let mut warnings = Warnings::new();
        let selected = selected_palette(&model, &mut warnings).expect("palette resolved");
        assert_eq!(selected.get("frost").map(String::as_str), Some("#88c0d0"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn warns_when_no_palette_defined() {
        let model = parse("format = \"$directory\"\n").unwrap();
        let mut warnings = Warnings::new();
        assert!(selected_palette(&model, &mut warnings).is_none());
        assert!(
            warnings
                .iter()
                .any(|w| w.kind == WarningKind::UnrepresentableOption)
        );
    }

    #[test]
    fn translate_validates_and_merges_discovered_colors() {
        let mut selected = HashMap::new();
        selected.insert("frost".to_owned(), "#88c0d0".to_owned());
        selected.insert("broken".to_owned(), "notacolor".to_owned());
        let mut discovered = BTreeMap::new();
        discovered.insert("rust".to_owned(), "red".to_owned());
        let mut warnings = Warnings::new();

        let palette = translate_palette("nord", Some(&selected), &discovered, &mut warnings);

        assert_eq!(palette.name, "nord");
        assert_eq!(
            palette.colors.get("frost").map(ColorSpec::as_str),
            Some("#88c0d0")
        );
        assert_eq!(
            palette.colors.get("rust").map(ColorSpec::as_str),
            Some("red")
        );
        assert!(!palette.colors.contains_key("broken"));
        assert!(warnings.iter().any(|w| w.kind == WarningKind::InvalidColor));
    }
}
