//! Reader for the shared vector files under `tests/fixtures/`.
//!
//! Rows are TAB-separated and never trimmed (` /lead-space` is a real value);
//! blank lines and lines starting with `#` are skipped. A cell that is exactly
//! `\e` is the empty string; any other cell uses the mini escape scheme the
//! file headers document.

use std::path::Path;

/// Whether `raw` is the whole-cell token for the empty string.
pub(crate) fn is_empty_token(raw: &str) -> bool {
    raw == "\\e"
}

/// Decode one cell: `\e` is empty; otherwise `\\` is one backslash, `\n` LF,
/// `\r` CR and `\t` tab.
pub(crate) fn cell(raw: &str) -> String {
    if is_empty_token(raw) {
        return String::new();
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            other => panic!("bad fixture escape in {raw:?}: \\{other:?}"),
        }
    }
    out
}

/// The rows of `tests/fixtures/<name>`, each split on TAB into raw cells.
pub(crate) fn rows(name: &str) -> Vec<Vec<String>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture.display()));
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}
