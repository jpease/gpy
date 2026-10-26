//! Derive a GPY `enabled_segments` order from Starship's top-level `format`.

use crate::config::defaults::default_enabled_segments;
use crate::import::starship::format::{Token, tokens};
use crate::import::starship::modules::canonical_language;
use crate::import::starship::warn::{WarningKind, Warnings};

/// Starship's newline module; maps to `ui.two_line`, not a segment.
const LINE_BREAK: &str = "line_break";

/// Starship's catch-all module; maps to GPY's default segment order.
const ALL: &str = "all";

/// Whether `format` references `$line_break` or `$all` (escaped refs don't count).
#[must_use]
pub fn has_line_break(format: &str) -> bool {
    module_refs(format).any(|module| module == LINE_BREAK || module == ALL)
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
    let mut warned: Vec<&str> = Vec::new();
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
        match map_module(module) {
            Some(segment) => {
                if !result.iter().any(|existing| existing == segment) {
                    result.push(segment.to_owned());
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

/// The variable names a format string references, in order.
///
/// An empty `${}` names no module, so it is skipped.
fn module_refs(format: &str) -> impl Iterator<Item = &str> {
    tokens(format).into_iter().filter_map(|token| match token {
        Token::Var { name, .. } if !name.is_empty() => Some(name),
        Token::Var { .. } | Token::Literal(_) => None,
    })
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
    use crate::import::starship::format::retain_known_vars;

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
    #[test]
    fn unclosed_brace_is_literal_in_layout_and_module_formats() {
        let mut warnings = Warnings::new();
        assert_eq!(
            derive_segments("${rust$directory", &mut warnings),
            vec!["directory"]
        );
        assert!(!has_line_break("${line_break"));
        assert_eq!(
            retain_known_vars("${rust$directory", &["directory"], "test", &mut warnings),
            "${rust$directory"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn empty_braced_reference_names_no_module() {
        let mut warnings = Warnings::new();
        assert_eq!(
            derive_segments("${}$directory", &mut warnings),
            vec!["directory"]
        );
        assert!(warnings.is_empty());
    }
}
