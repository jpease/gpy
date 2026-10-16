//! `gpy __complete <kind>`: hidden command printing undecorated names, one per
//! line, so shell completion scripts share one discovery implementation.

use crate::palette::PaletteManager;
use crate::theme::ThemeManager;

/// Which value list `__complete` should emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteKind {
    /// Installed theme names.
    Theme,
    /// Installed palette names.
    Palette,
    /// Available prompt segment names (built-in + plugin-provided).
    Segment,
}

/// Return the plain candidate names for `kind`, in discovery order.
#[must_use]
pub fn names(kind: CompleteKind) -> Vec<String> {
    match kind {
        CompleteKind::Theme => ThemeManager::discover_available_themes()
            .into_iter()
            .map(|theme| theme.name)
            .collect(),
        CompleteKind::Palette => PaletteManager::discover_available_palettes()
            .into_iter()
            .map(|palette| palette.name)
            .collect(),
        CompleteKind::Segment => super::segments::available_segments(),
    }
}

/// Print one candidate name per line for `kind`.
pub fn run(kind: CompleteKind) {
    for name in names(kind) {
        println!("{name}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    fn assert_undecorated(values: &[String]) {
        for value in values {
            assert!(
                !value.contains('*'),
                "candidate {value:?} should not contain an active-marker '*'"
            );
            assert!(
                !value.contains('['),
                "candidate {value:?} should not contain a '[source]' suffix"
            );
            assert_eq!(
                value.trim(),
                value,
                "candidate {value:?} should not have leading/trailing whitespace"
            );
        }
    }

    #[test]
    fn segment_names_match_builtin_order_and_are_undecorated() {
        let segments = names(CompleteKind::Segment);
        let expected: Vec<&str> = crate::plugin::BUILTIN_ORDER
            .iter()
            .map(|segment| segment.as_str())
            .collect();
        assert_eq!(segments, expected);
        assert_undecorated(&segments);
    }

    #[test]
    fn theme_names_are_undecorated_and_nonempty() {
        let themes = names(CompleteKind::Theme);
        assert!(!themes.is_empty(), "expected at least one builtin theme");
        assert_undecorated(&themes);
    }

    #[test]
    fn palette_names_are_undecorated_and_nonempty() {
        let palettes = names(CompleteKind::Palette);
        assert!(
            !palettes.is_empty(),
            "expected at least one builtin palette"
        );
        assert_undecorated(&palettes);
    }
}
