//! Case-insensitive subsequence fuzzy matching for wizard list filtering.
//!
//! No external fuzzy-matching dependency: for a few dozen candidate items
//! (themes/palettes/segments) a hand-rolled subsequence matcher is enough to
//! feel "fzf-style" without adding a crate every match must trust.

/// `(span_length, start_index)` of a matched subsequence — smaller of
/// either is a better match, `span_length` dominating: a tight match
/// starting later still beats a scattered match starting earlier.
type MatchScore = (usize, usize);

/// Filter `items` to those whose characters (case-insensitively) contain
/// `query` as a subsequence, sorted so tighter/earlier matches rank first.
///
/// Ties keep original list order (stable sort) — an empty `query` is a
/// strict no-op: returns `0..items.len()` unchanged.
pub fn filter_and_sort(query: &str, items: &[String]) -> Vec<usize> {
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let mut scored: Vec<(usize, MatchScore)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| best_match(&query_lower, item).map(|score| (index, score)))
        .collect();

    scored.sort_by(|(a_index, a_score), (b_index, b_score)| {
        a_score.cmp(b_score).then(a_index.cmp(b_index))
    });

    scored.into_iter().map(|(index, _)| index).collect()
}

/// Find the earliest greedy subsequence match of `query_lower` (already
/// lowercased) within `candidate` (case-insensitively).
///
/// Returns its `(span_length, start_index)` score, or `None` if
/// `query_lower` isn't a subsequence of `candidate` at all.
fn best_match(query_lower: &[char], candidate: &str) -> Option<MatchScore> {
    let candidate_lower: Vec<char> = candidate.to_lowercase().chars().collect();

    let mut query_pos = 0_usize;
    let mut match_start: Option<usize> = None;
    let mut last = 0_usize;

    for (candidate_index, ch) in candidate_lower.iter().enumerate() {
        let Some(&query_char) = query_lower.get(query_pos) else {
            break;
        };
        if *ch == query_char {
            if match_start.is_none() {
                match_start = Some(candidate_index);
            }
            last = candidate_index;
            query_pos = query_pos.saturating_add(1);
        }
    }

    if query_pos < query_lower.len() {
        return None;
    }

    let resolved_start = match_start.unwrap_or(0);
    Some((
        last.saturating_sub(resolved_start).saturating_add(1),
        resolved_start,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_query_returns_all_indices_in_original_order() {
        let items = owned(&["dracula", "nord", "catppuccin-mocha"]);
        assert_eq!(filter_and_sort("", &items), vec![0, 1, 2]);
    }

    #[test]
    fn empty_items_returns_empty_regardless_of_query() {
        let items: Vec<String> = Vec::new();
        assert_eq!(filter_and_sort("cat", &items), Vec::<usize>::new());
    }

    #[test]
    fn exact_substring_match_is_included() {
        let items = owned(&["catppuccin-mocha"]);
        assert_eq!(filter_and_sort("cat", &items), vec![0]);
    }

    #[test]
    fn case_insensitive_match() {
        let items = owned(&["Catppuccin-Mocha"]);
        assert_eq!(filter_and_sort("CAT", &items), vec![0]);
    }

    #[test]
    fn non_matching_query_excludes_item() {
        let items = owned(&["nord", "dracula"]);
        assert_eq!(filter_and_sort("xyz", &items), Vec::<usize>::new());
    }

    #[test]
    fn scattered_subsequence_still_matches() {
        // "ccn" is a subsequence of "catppuccin" (c...c...n) even though
        // it isn't contiguous.
        let items = owned(&["catppuccin"]);
        assert_eq!(filter_and_sort("ccn", &items), vec![0]);
    }

    #[test]
    fn tighter_match_ranks_above_scattered_match() {
        // "cat" is a contiguous prefix of "catppuccin" (tight) but only a
        // scattered subsequence of "chocolate" (c-h-o-c-o-l-a-t-e).
        let items = owned(&["chocolate", "catppuccin"]);
        assert_eq!(filter_and_sort("cat", &items), vec![1, 0]);
    }

    #[test]
    fn earlier_start_ranks_above_later_start_at_equal_span() {
        // Both "muscat" and "catppuccin" contain "cat" as a tight 3-char
        // span; "catppuccin" matches it starting at index 0, "muscat" at 3.
        let items = owned(&["muscat", "catppuccin"]);
        assert_eq!(filter_and_sort("cat", &items), vec![1, 0]);
    }

    #[test]
    fn ties_preserve_original_list_order() {
        // Both start "cat" at index 0 with span 3 - identical score, so the
        // stable sort must keep them in original list order.
        let items = owned(&["catalog", "category"]);
        assert_eq!(filter_and_sort("cat", &items), vec![0, 1]);
    }
}
