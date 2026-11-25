//! `tests/fixtures/instant_cache_naming_vectors.tsv` against the agent's cache
//! file naming: Rust is the oracle for every row it checks, so a hand-edited
//! expected value fails here as well as in the shell vector tests.

use super::name::{position_suffix, prev_bg_token};
use crate::formatter::{IsFirst, IsLast, SegmentPosition};
use crate::vector_cells::{cell, is_empty_token};

/// A `suffix` flag cell Rust can represent: `true`, `false` or `\e` (false).
/// Other spellings test the shells' "anything but `true` is false" rule.
fn flag(raw: &str) -> Option<bool> {
    match raw {
        "true" => Some(true),
        "false" => Some(false),
        _ if is_empty_token(raw) => Some(false),
        _ => None,
    }
}

#[test]
fn instant_cache_naming_shared_vectors() {
    let mut checked = 0_u32;
    for row in crate::vector_cells::rows("instant_cache_naming_vectors.tsv") {
        match row
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["token", prev_bg, expected, ..] => {
                let actual = if is_empty_token(prev_bg) {
                    prev_bg_token(None)
                } else {
                    prev_bg_token(Some(&cell(prev_bg)))
                };
                assert_eq!(actual, cell(expected), "token row {row:?}");
                checked = checked.saturating_add(1);
            }
            ["suffix", base, is_last, is_first, expected, ..] => {
                let (Some(last), Some(first)) = (flag(is_last), flag(is_first)) else {
                    continue;
                };
                let position = SegmentPosition::new(IsLast::from(last), IsFirst::from(first));
                let actual = format!("{}{}", cell(base), position_suffix(position));
                assert_eq!(actual, cell(expected), "suffix row {row:?}");
                checked = checked.saturating_add(1);
            }
            // Shell cache-read status codes and the git background file tail
            // (`git_bg_names_match_the_shared_vectors`) are not checked here.
            ["status" | "bgfile", ..] => {}
            _ => panic!("unknown or short row {row:?}"),
        }
    }
    assert!(checked > 0, "no instant-cache naming vectors were checked");
}
