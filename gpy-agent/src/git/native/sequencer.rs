//! Parsers for the in-progress-operation files git keeps under `.git`.

use super::REBASE_TODO_READ_CAP;
use crate::git::RepositoryState;

/// Parse a `msgnum` file (current step of an interactive rebase).
#[must_use]
pub(super) fn parse_msgnum(contents: &str) -> Option<u32> {
    contents.trim().parse().ok()
}

/// Count non-blank, non-comment lines (remaining steps) in `git-rebase-todo`,
/// reading at most `REBASE_TODO_READ_CAP` bytes; `None` past the cap or on I/O error.
///
/// Lines are counted as bytes so a non-UTF-8 commit subject still counts, and the
/// reader is bounded so one newline-free line cannot grow the buffer past the cap.
#[must_use]
pub(super) fn count_todo_steps<R: std::io::BufRead>(source: R) -> Option<u32> {
    use std::io::BufRead as _;

    let limit = u64::try_from(REBASE_TODO_READ_CAP).ok()?.saturating_add(1);
    let mut reader = source.take(limit);
    let mut line = Vec::new();
    let mut consumed = 0_usize;
    let mut count = 0_usize;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).ok()?;
        if read == 0 {
            break;
        }
        consumed = consumed.saturating_add(read);
        if consumed > REBASE_TODO_READ_CAP {
            return None;
        }
        let trimmed = line.trim_ascii();
        if !trimmed.is_empty() && !trimmed.starts_with(b"#") {
            count = count.saturating_add(1);
        }
    }
    u32::try_from(count).ok()
}

/// Classify a `sequencer/todo` file by its first non-blank, non-comment
/// line, mirroring git's `sequencer_get_last_command`.
#[must_use]
pub(super) fn parse_sequencer_command(contents: &str) -> Option<RepositoryState> {
    let line = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    match line.split_whitespace().next()? {
        "pick" | "p" => Some(RepositoryState::CherryPicking),
        "revert" => Some(RepositoryState::Reverting),
        _ => None,
    }
}
