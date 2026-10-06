//! Derive a GPY `enabled_segments` order from Starship's top-level `format`.

use crate::config::defaults::default_enabled_segments;
use crate::import::starship::modules::canonical_language;
use crate::import::starship::{WarningKind, Warnings};

/// Starship's module that breaks the prompt onto a new line. It is not a
/// segment: it maps to the theme's `ui.two_line` instead.
const LINE_BREAK: &str = "line_break";

/// Starship's catch-all module: every module not named elsewhere in `format`.
/// GPY maps it to its default segment order.
const ALL: &str = "all";

/// Whether `format` breaks the prompt onto a second line: it references
/// `$line_break`, or `$all`, which expands to every module including it.
/// Escaped references (`\$line_break`) do not count.
#[must_use]
pub fn has_line_break(format: &str) -> bool {
    module_refs(format)
        .iter()
        .any(|module| module == LINE_BREAK || module == ALL)
}

/// Map a Starship module reference to its GPY segment name.
#[must_use]
pub fn map_module(name: &str) -> Option<&'static str> {
    match name {
        "directory" => Some("directory"),
        "git_branch" | "git_status" | "git_state" => Some("git"),
        "cmd_duration" => Some("duration"),
        "character" => Some("character"),
        "time" => Some("clock"),
        "hostname" => Some("hostname"),
        "username" => Some("username"),
        other => canonical_language(other).map(|_| "language"),
    }
}

/// Parse a top-level Starship `format` into an ordered, deduped GPY segment list.
#[must_use]
pub fn derive_segments(format: &str, warnings: &mut Warnings) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let mut warned: Vec<String> = Vec::new();
    for module in module_refs(format) {
        if module == LINE_BREAK {
            continue;
        }
        if module == ALL {
            for segment in default_enabled_segments() {
                if !result.contains(&segment) {
                    result.push(segment);
                }
            }
            continue;
        }
        match map_module(&module) {
            Some(segment) => {
                let owned = segment.to_owned();
                if !result.contains(&owned) {
                    result.push(owned);
                }
            }
            None => {
                if !warned.contains(&module) {
                    warnings.push(
                        WarningKind::UnsupportedModule,
                        format!("module '{module}' has no GPY equivalent; omitted from layout"),
                    );
                    warned.push(module);
                }
            }
        }
    }
    result
}

/// Extract `$name`/`${name}` references from a format string, in order.
fn module_refs(format: &str) -> Vec<String> {
    let chars: Vec<char> = format.chars().collect();
    let mut refs: Vec<String> = Vec::new();
    let mut index = 0_usize;
    while let Some(&ch) = chars.get(index) {
        if ch == '\\' {
            index = index.saturating_add(2_usize);
            continue;
        }
        if ch == '$' {
            index = index.saturating_add(1_usize);
            let mut name = String::new();
            let braced = chars.get(index) == Some(&'{');
            if braced {
                index = index.saturating_add(1_usize);
            }
            while let Some(&inner) = chars.get(index) {
                if braced && inner == '}' {
                    index = index.saturating_add(1_usize);
                    break;
                }
                if !(braced || inner.is_alphanumeric() || inner == '_') {
                    break;
                }
                name.push(inner);
                index = index.saturating_add(1_usize);
            }
            if !name.is_empty() {
                refs.push(name);
            }
            continue;
        }
        index = index.saturating_add(1_usize);
    }
    refs
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{derive_segments, has_line_break, map_module};
    use crate::config::defaults::default_enabled_segments;
    use crate::import::starship::Warnings;

    #[test]
    fn maps_module_groups() {
        assert_eq!(map_module("directory"), Some("directory"));
        assert_eq!(map_module("git_branch"), Some("git"));
        assert_eq!(map_module("git_status"), Some("git"));
        assert_eq!(map_module("cmd_duration"), Some("duration"));
        assert_eq!(map_module("rust"), Some("language"));
        assert_eq!(map_module("nodejs"), Some("language"));
        assert_eq!(map_module("time"), Some("clock"));
        assert_eq!(map_module("hostname"), Some("hostname"));
        assert_eq!(map_module("kubernetes"), None);
    }

    #[test]
    fn derives_ordered_deduped_segments() {
        let mut warnings = Warnings::new();
        let segments = derive_segments(
            "$directory$git_branch$git_status$rust$python$cmd_duration$character",
            &mut warnings,
        );
        assert_eq!(
            segments,
            vec!["directory", "git", "language", "duration", "character"]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn derives_hostname_ahead_of_directory() {
        let mut warnings = Warnings::new();
        let segments = derive_segments("$hostname$directory$character", &mut warnings);
        assert_eq!(segments, vec!["hostname", "directory", "character"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn warns_once_per_unsupported_module() {
        let mut warnings = Warnings::new();
        let segments = derive_segments("$directory$kubernetes$aws$kubernetes", &mut warnings);
        assert_eq!(segments, vec!["directory"]);
        assert_eq!(
            warnings.len(),
            2_usize,
            "kubernetes + aws, kubernetes only once"
        );
    }

    #[test]
    fn line_break_is_skipped_silently() {
        let mut warnings = Warnings::new();
        let segments = derive_segments("$directory$line_break$character", &mut warnings);
        assert_eq!(segments, vec!["directory", "character"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn all_expands_to_default_order_without_warning() {
        let mut warnings = Warnings::new();
        let segments = derive_segments("$all", &mut warnings);
        assert_eq!(segments, default_enabled_segments());
        assert!(warnings.is_empty());
    }

    #[test]
    fn all_keeps_position_and_dedupes() {
        let mut warnings = Warnings::new();
        let segments = derive_segments("$character$all", &mut warnings);
        let mut expected = vec!["character".to_owned()];
        expected.extend(default_enabled_segments());
        assert_eq!(segments, expected);
        assert!(warnings.is_empty());
    }

    #[test]
    fn detects_line_break_references() {
        assert!(has_line_break("$directory$line_break$character"));
        assert!(has_line_break("$directory${line_break}$character"));
        assert!(has_line_break("$all"));
        assert!(!has_line_break("$directory$character"));
        assert!(!has_line_break("$directory\\$line_break$character"));
    }
}
