//! `gpy __complete <kind>` — hidden internal command for shell completion glue.
//!
//! Fish, Bash, and Zsh completion scripts all need the same plain lists of
//! installed theme names, palette names, and segment names. Rather than each
//! shell reimplementing that discovery logic (and drifting out of sync), they
//! shell out to this hidden subcommand and split its stdout on newlines.
//!
//! Output here is intentionally undecorated: no active-marker (`*`), no
//! `[source]` suffix, no header lines — just one name per line. The decorated,
//! human-facing equivalents live in [`super::theme::list`],
//! [`super::palette::list`], and [`super::segments::list`].

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
///
/// This is the pure, unit-testable core of `__complete`: no I/O beyond the
/// existing discovery calls, no printing.
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
///
/// Infallible: the underlying discovery calls never fail, so this returns `()`
/// rather than `Result` (clippy `unnecessary_wraps`). The caller wraps the
/// unit result in `Ok(())`, mirroring other infallible handlers such as
/// [`super::plugin::list`].
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
        assert_eq!(
            segments,
            vec![
                "clock",
                "duration",
                "language",
                "directory",
                "git",
                "status"
            ],
        );
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
