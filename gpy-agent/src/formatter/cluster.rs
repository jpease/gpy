//! Character-cluster boundaries for cutting text without splitting a flag, a
//! ZWJ sequence or a base character from its combining marks. Clusters are
//! approximated with `char` rules; no grapheme-segmentation table is carried.

/// Byte offset just past the character cluster starting at byte `start`.
pub(crate) fn cluster_end(text: &str, start: usize) -> usize {
    let mut chars = text.get(start..).unwrap_or("").chars().peekable();
    let Some(first) = chars.next() else {
        return start;
    };
    let mut len = first.len_utf8();
    if is_regional_indicator(first) && chars.peek().copied().is_some_and(is_regional_indicator) {
        len = len.saturating_add(chars.next().map_or_default(char::len_utf8));
    }
    while let Some(next) = chars.peek().copied() {
        if next == '\u{200D}' {
            // Zero-width joiner glues the following character on.
            chars.next();
            len = len.saturating_add(next.len_utf8());
            len = len.saturating_add(chars.next().map_or_default(char::len_utf8));
        } else if is_cluster_extender(next) {
            chars.next();
            len = len.saturating_add(next.len_utf8());
        } else {
            break;
        }
    }
    start.saturating_add(len)
}

const fn is_regional_indicator(c: char) -> bool {
    matches!(c, '\u{1F1E6}'..='\u{1F1FF}')
}

/// Combining marks, variation selectors, emoji skin-tone modifiers, tag chars.
const fn is_cluster_extender(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// The clusters of `text`, in order.
fn clusters(text: &str) -> impl Iterator<Item = &str> {
    let mut start = 0_usize;
    std::iter::from_fn(move || {
        if start >= text.len() {
            return None;
        }
        let end = cluster_end(text, start);
        let piece = text.get(start..end);
        start = end;
        piece
    })
}

/// How many clusters `text` holds.
pub(crate) fn count(text: &str) -> usize {
    clusters(text).count()
}

/// The first `keep` clusters of `text`.
pub(crate) fn head(text: &str, keep: usize) -> &str {
    let end: usize = clusters(text).take(keep).map(str::len).sum();
    text.get(..end).unwrap_or(text)
}

/// The last `keep` clusters of `text`.
pub(crate) fn tail(text: &str, keep: usize) -> &str {
    let skip = count(text).saturating_sub(keep);
    let start: usize = clusters(text).take(skip).map(str::len).sum();
    text.get(start..).unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::{count, head, tail};

    #[test]
    fn combining_marks_and_flags_stay_whole() {
        let text = "e\u{301}x\u{1F1FA}\u{1F1F8}y";
        assert_eq!(count(text), 4);
        assert_eq!(head(text, 1), "e\u{301}");
        assert_eq!(head(text, 3), "e\u{301}x\u{1F1FA}\u{1F1F8}");
        assert_eq!(tail(text, 2), "\u{1F1FA}\u{1F1F8}y");
        assert_eq!(tail(text, 9), text);
    }

    #[test]
    fn zwj_sequence_is_one_cluster() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(count(family), 1);
        assert_eq!(head(family, 1), family);
    }
}
