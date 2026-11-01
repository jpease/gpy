//! Test-only Markdown table reader for the doc-drift tests.
//!
//! Reads pipe tables outside code fences and keeps the nearest preceding
//! heading with each one. It handles only what the docs use: no alignment, no
//! nested structure, no inline Markdown.

/// One pipe table and the heading it sits under.
pub(crate) struct DocTable {
    /// Text of the nearest preceding heading, without its `#`s.
    pub heading: String,
    /// Header cells, trimmed.
    pub header: Vec<String>,
    /// Body rows, cells trimmed.
    pub rows: Vec<Vec<String>>,
}

impl DocTable {
    /// Index of the header cell equal to `name`.
    pub(crate) fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|cell| cell == name)
    }
}

/// Split one table line into trimmed cells; `\|` is a literal pipe.
fn cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let body = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cell.push('|');
            }
            '|' => out.push(std::mem::take(&mut cell).trim().to_owned()),
            _ => cell.push(c),
        }
    }
    // Text after the last pipe is a cell only when the line has no closing pipe.
    if !cell.trim().is_empty() {
        out.push(cell.trim().to_owned());
    }
    out
}

/// `true` for a delimiter row such as `|---|:--:|`.
fn is_delimiter(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed.contains('-')
        && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Heading text of an ATX heading line (`## Title`), if it is one.
fn heading_text(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        line.get(hashes..)?.strip_prefix(' ').map(str::trim)
    } else {
        None
    }
}

/// Every pipe table in `markdown`, in document order.
pub(crate) fn tables(markdown: &str) -> Vec<DocTable> {
    let mut lines = markdown.lines().peekable();
    let mut out = Vec::new();
    let mut heading = String::new();
    let mut fence: Option<&str> = None;
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some("~~~");
            continue;
        }
        if let Some(text) = heading_text(line) {
            text.clone_into(&mut heading);
            continue;
        }
        if line.starts_with('|') && lines.peek().is_some_and(|next| is_delimiter(next)) {
            lines.next();
            let header = cells(line);
            let mut rows = Vec::new();
            while let Some(row) = lines.next_if(|row| row.starts_with('|')) {
                rows.push(cells(row));
            }
            out.push(DocTable {
                heading: heading.clone(),
                header,
                rows,
            });
        }
    }
    out
}
