//! Pure parser for `git status --porcelain=v2 --branch -z` output.

use crate::git::FileStatus;
use std::collections::HashMap;
use std::path::PathBuf;

/// Represents a parsed changed file entry from Git porcelain v2 output.
pub struct ParsedChangedFile {
    /// The path to the changed file.
    pub path: PathBuf,
    /// The status of the changed file.
    pub status: FileStatus,
}

/// Branch identity from a `# branch.head` header. `Detached` needs a
/// subprocess to resolve, which the backend does after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchHead {
    /// An ordinary branch name, taken verbatim from the header line.
    Named(String),
    /// `(detached)` — HEAD points directly at a commit, not a branch.
    Detached,
}

impl BranchHead {
    /// Parses the text after the `# branch.head ` prefix.
    #[must_use]
    fn parse(header_content: &str) -> Self {
        match header_content {
            "(detached)" => Self::Detached,
            named => Self::Named(named.to_owned()),
        }
    }
}

/// Accumulator state for parsing Git porcelain v2 output line by line.
#[derive(Default)]
pub struct V2ParseState {
    /// The unresolved branch identity; `None` until a `# branch.head` line is seen.
    pub branch: Option<BranchHead>,
    /// Commit id from `# branch.oid`; `None` for an unborn branch or a missing header.
    pub oid: Option<String>,
    /// Number of commits ahead of upstream.
    pub ahead: u32,
    /// Number of commits behind upstream.
    pub behind: u32,
    /// Whether the ahead count was capped due to `max_ahead_behind`.
    pub ahead_capped: bool,
    /// Whether the behind count was capped due to `max_ahead_behind`.
    pub behind_capped: bool,
    /// Map of file paths to their status; sole source of the counts.
    pub files: HashMap<PathBuf, FileStatus>,
}

impl V2ParseState {
    /// Creates a new `V2ParseState` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an untracked file, merging with any existing entry
    /// (`git rm --cached f` reports both a staged deletion and untracked `f`).
    pub(crate) fn process_untracked(&mut self, path: PathBuf) {
        self.files.entry(path).or_default().untracked = true;
    }

    /// Records changed-file flags, merging with any existing entry.
    pub(crate) fn process_changed(&mut self, parsed: ParsedChangedFile) {
        let ParsedChangedFile { path, status } = parsed;
        let entry = self.files.entry(path).or_default();
        entry.staged |= status.staged;
        entry.unstaged |= status.unstaged;
        entry.untracked |= status.untracked;
        entry.conflicted |= status.conflicted;
    }
}

/// Parses UTF-8-decoded `git status --porcelain=v2 --branch -z` output.
#[must_use]
pub fn parse_v2_records(output: &str, max_ahead_behind: usize) -> V2ParseState {
    parse_v2_state(output, max_ahead_behind)
}

/// Parses the file records of `git status --porcelain=v2 -z` output, ignoring
/// any header lines.
#[must_use]
pub fn parse_v2_files(output: &str) -> HashMap<PathBuf, FileStatus> {
    parse_v2_state(output, 0).files
}

/// Runs the record loop shared by [`parse_v2_records`] and [`parse_v2_files`].
fn parse_v2_state(output: &str, max_ahead_behind: usize) -> V2ParseState {
    let mut state = V2ParseState::new();

    // A type `2` record is followed by a separate original-path record, which
    // must be consumed rather than parsed as its own entry.
    let records: Vec<&str> = output.split('\0').filter(|r| !r.is_empty()).collect();
    let mut index = 0_usize;
    while let Some(&record) = records.get(index) {
        if let Some(header_content) = record.strip_prefix("# branch.oid ") {
            state.oid = (header_content != "(initial)").then(|| header_content.to_owned());
        } else if let Some(header_content) = record.strip_prefix("# branch.head ") {
            state.branch = Some(BranchHead::parse(header_content));
        } else if let Some(header_content) = record.strip_prefix("# branch.ab ") {
            let (ahead, behind, ahead_capped, behind_capped) =
                parse_v2_ahead_behind(header_content, max_ahead_behind);
            state.ahead = ahead;
            state.behind = behind;
            state.ahead_capped = ahead_capped;
            state.behind_capped = behind_capped;
        } else if let Some(path) = parse_v2_untracked_line(record) {
            state.process_untracked(path);
        } else if let Some(parsed) = parse_v2_changed_line(record) {
            let is_rename_or_copy = record.starts_with("2 ");
            state.process_changed(parsed);
            if is_rename_or_copy {
                index = index.saturating_add(1);
            }
        }

        index = index.saturating_add(1);
    }
    state
}

/// Parses `branch.ab` into `(ahead, behind, ahead_capped, behind_capped)`.
///
/// A non-zero `max_ahead_behind` is an inclusive ceiling (counts above it are
/// clamped and flagged); `0` means unlimited.
#[must_use]
pub fn parse_v2_ahead_behind(ab_str: &str, max_ahead_behind: usize) -> (u32, u32, bool, bool) {
    let parts: Vec<&str> = ab_str.split_whitespace().collect();
    if parts.len() == 2 {
        let ahead = parts
            .first()
            .and_then(|s| s.trim_start_matches('+').parse().ok())
            .unwrap_or(0);
        let behind = parts
            .get(1)
            .and_then(|s| s.trim_start_matches('-').parse().ok())
            .unwrap_or(0);

        if max_ahead_behind == 0 {
            return (ahead, behind, false, false);
        }

        let max_u32 = u32::try_from(max_ahead_behind).unwrap_or(u32::MAX);
        let ahead_capped = ahead > max_u32;
        let behind_capped = behind > max_u32;

        (
            ahead.min(max_u32),
            behind.min(max_u32),
            ahead_capped,
            behind_capped,
        )
    } else {
        (0, 0, false, false)
    }
}

/// Parses an XY status code into `(is_conflict, is_staged, is_unstaged)`.
/// X is the index side, Y the worktree; `U`, `AA`, and `DD` are conflicts.
#[must_use]
pub fn parse_v2_file_status(xy: &str) -> (bool, bool, bool) {
    const UNMODIFIED: char = '.';
    const UNTRACKED: char = '?';
    const UNMERGED: char = 'U';
    const ADDED: char = 'A';
    const DELETED: char = 'D';

    if xy.len() < 2 {
        return (false, false, false);
    }

    let x = xy.chars().next().unwrap_or(UNMODIFIED);
    let y = xy.chars().nth(1).unwrap_or(UNMODIFIED);

    let is_conflict = x == UNMERGED
        || y == UNMERGED
        || (x == ADDED && y == ADDED)
        || (x == DELETED && y == DELETED);

    let is_staged = x != UNMODIFIED && x != UNTRACKED;

    let is_unstaged = y != UNMODIFIED && y != UNTRACKED;

    (is_conflict, is_staged, is_unstaged)
}

/// Parses an untracked record (`? <path>`); `-z` paths are taken verbatim.
#[must_use]
pub fn parse_v2_untracked_line(line: &str) -> Option<PathBuf> {
    let filename = line.strip_prefix("? ")?;
    if filename.is_empty() {
        return None;
    }
    Some(PathBuf::from(filename))
}

/// Parses a changed record (type `1`, `2`, or `u`), which have 8, 9, and 10
/// metadata fields before the path. For type `2` this is the destination path;
/// the original path is a separate `-z` record handled by the caller.
#[must_use]
pub fn parse_v2_changed_line(line: &str) -> Option<ParsedChangedFile> {
    let metadata_fields: usize = match line.as_bytes().first()? {
        b'1' => 8,
        b'2' => 9,
        b'u' => 10,
        _ => return None,
    };

    // The final piece keeps filenames containing spaces intact.
    let mut fields = line.splitn(metadata_fields.checked_add(1)?, ' ');
    let _record_type = fields.next()?;
    let xy = fields.next()?;
    for _ in 0..metadata_fields.checked_sub(2)? {
        fields.next()?;
    }
    let path_field = fields.next()?;

    if path_field.is_empty() {
        return None;
    }
    let path = PathBuf::from(path_field);

    let (is_conflict, is_staged, is_unstaged) = parse_v2_file_status(xy);

    // Conflicted X/Y letters describe merge sides, not index/worktree changes.
    let status = if is_conflict {
        FileStatus {
            conflicted: true,
            ..FileStatus::default()
        }
    } else {
        FileStatus {
            staged: is_staged,
            unstaged: is_unstaged,
            ..FileStatus::default()
        }
    };

    Some(ParsedChangedFile { path, status })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::StatusAggregate;
    use std::path::Path;

    #[test]
    fn ahead_behind_below_limit_is_uncapped() {
        let (ahead, behind, ahead_capped, behind_capped) = parse_v2_ahead_behind("+1 -0", 2);
        assert_eq!((ahead, behind), (1, 0));
        assert!(!ahead_capped);
        assert!(!behind_capped);
    }

    #[test]
    fn ahead_behind_exactly_at_limit_is_not_capped() {
        let (ahead, behind, ahead_capped, behind_capped) = parse_v2_ahead_behind("+2 -2", 2);
        assert_eq!((ahead, behind), (2, 2));
        assert!(!ahead_capped);
        assert!(!behind_capped);
    }

    #[test]
    fn ahead_behind_above_limit_is_clamped_and_flagged() {
        let (ahead, behind, ahead_capped, behind_capped) = parse_v2_ahead_behind("+3 -5", 2);
        assert_eq!((ahead, behind), (2, 2), "counts must be clamped to the max");
        assert!(ahead_capped);
        assert!(behind_capped);
    }

    #[test]
    fn ahead_behind_zero_max_is_unlimited() {
        let (ahead, behind, ahead_capped, behind_capped) = parse_v2_ahead_behind("+42 -99", 0);
        assert_eq!((ahead, behind), (42, 99));
        assert!(!ahead_capped);
        assert!(!behind_capped);
    }

    #[test]
    fn ahead_behind_malformed_header_yields_zeroes() {
        let (ahead, behind, ahead_capped, behind_capped) = parse_v2_ahead_behind("garbage", 2);
        assert_eq!((ahead, behind), (0, 0));
        assert!(!ahead_capped);
        assert!(!behind_capped);
    }

    #[test]
    fn type1_record_extracts_path_and_status() {
        let record = "1 M. N... 100644 100644 100644 abc abc src/main.rs";
        let parsed = parse_v2_changed_line(record).expect("type 1 record parses");
        assert_eq!(parsed.path, PathBuf::from("src/main.rs"));
        assert!(parsed.status.staged);
        assert!(!parsed.status.unstaged);
    }

    #[test]
    fn type1_record_preserves_spaces_in_path() {
        let record = "1 .M N... 100644 100644 100644 abc abc my notes file.txt";
        let parsed = parse_v2_changed_line(record).expect("type 1 record parses");
        assert_eq!(parsed.path, PathBuf::from("my notes file.txt"));
        assert!(!parsed.status.staged);
        assert!(parsed.status.unstaged);
    }

    #[test]
    fn type2_rename_uses_destination_path_only() {
        let record = "2 R. N... 100644 100644 100644 abc abc R100 new name.txt";
        let parsed = parse_v2_changed_line(record).expect("type 2 record parses");
        assert_eq!(parsed.path, PathBuf::from("new name.txt"));
        assert!(parsed.status.staged);
    }

    #[test]
    fn changed_record_keeps_tab_in_path() {
        // `-z` paths are unquoted; a TAB is filename content, not a separator.
        let record = "1 A. N... 000000 100644 100644 abc abc tab\tname.txt";
        let parsed = parse_v2_changed_line(record).expect("type 1 record parses");
        assert_eq!(parsed.path, PathBuf::from("tab\tname.txt"));
    }

    #[test]
    fn unmerged_record_is_parsed_as_a_conflict_with_correct_path() {
        // Type `u` has 10 metadata fields before the path.
        let record = "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflicted file.txt";
        let parsed = parse_v2_changed_line(record).expect("unmerged record parses");
        assert_eq!(parsed.path, PathBuf::from("conflicted file.txt"));
        assert!(parsed.status.conflicted);
        assert!(
            !parsed.status.staged && !parsed.status.unstaged,
            "a conflicted entry must not also count as staged or unstaged"
        );
    }

    #[test]
    fn changed_record_with_too_few_fields_is_rejected() {
        assert!(parse_v2_changed_line("1 M. N... 100644").is_none());
    }

    #[test]
    fn non_changed_record_type_is_rejected() {
        assert!(parse_v2_changed_line("? untracked.txt").is_none());
    }

    #[test]
    fn untracked_record_keeps_path_verbatim() {
        let parsed = parse_v2_untracked_line("? weird\tname.txt").expect("untracked parses");
        assert_eq!(parsed, PathBuf::from("weird\tname.txt"));
    }

    #[test]
    fn untracked_record_requires_a_filename() {
        assert!(parse_v2_untracked_line("? ").is_none());
        assert!(parse_v2_untracked_line("not untracked").is_none());
    }

    #[test]
    fn parse_v2_records_named_header_yields_branch_head_named() {
        let output = "# branch.head main\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, Some(BranchHead::Named("main".to_owned())));
    }

    #[test]
    fn parse_v2_records_detached_header_yields_branch_head_detached() {
        let output = "# branch.head (detached)\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, Some(BranchHead::Detached));
    }

    #[test]
    fn parse_v2_records_reads_branch_oid() {
        let output =
            "# branch.oid 8fb1ddef85e4672c6150364dcb5df6711fd16b65\0# branch.head (detached)\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, Some(BranchHead::Detached));
        assert_eq!(
            result.oid.as_deref(),
            Some("8fb1ddef85e4672c6150364dcb5df6711fd16b65")
        );
    }

    #[test]
    fn unborn_branch_has_named_head_and_no_oid() {
        let output = "# branch.oid (initial)\0# branch.head trunk\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, Some(BranchHead::Named("trunk".to_owned())));
        assert_eq!(result.oid, None);
    }

    #[test]
    fn records_without_branch_header_have_no_branch() {
        let result = parse_v2_records("? a.txt\0", 0);
        assert_eq!(result.branch, None);
    }

    #[test]
    fn parse_v2_files_ignores_headers_and_keeps_file_records() {
        let output =
            "# branch.head main\0? a.txt\x001 .M N... 100644 100644 100644 abc abc b.txt\0";
        let files = parse_v2_files(output);
        assert_eq!(files.len(), 2);
        assert!(
            files
                .get(&PathBuf::from("a.txt"))
                .is_some_and(|f| f.untracked)
        );
        assert!(
            files
                .get(&PathBuf::from("b.txt"))
                .is_some_and(|f| f.unstaged)
        );
    }

    #[test]
    fn aggregate_counts_each_v2_record_kind() {
        // Conflict, `git rm --cached f` (staged delete + untracked), worktree edit.
        let output = concat!(
            "# branch.head main\0",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc c.txt\0",
            "1 D. N... 100644 000000 000000 dddd 0000 f\0",
            "1 .M N... 100644 100644 100644 eeee eeee g\0",
            "? f\0",
        );
        let parsed = parse_v2_records(output, 0);
        let aggregate = StatusAggregate::from_file_statuses(parsed.files.values());
        assert_eq!(
            (
                aggregate.staged,
                aggregate.unstaged,
                aggregate.untracked,
                aggregate.conflicts
            ),
            (1, 1, 1, 1)
        );
    }

    /// `-z` status parsing keys renames by destination and clamps ahead/behind.
    /// The rename record's trailing original path must be skipped.
    #[test]
    fn parse_v2_output_handles_rename_and_clamps_counts() {
        let output = concat!(
            "# branch.head main\0",
            "# branch.ab +3 -1\0",
            "2 R. N... 100644 100644 100644 abc abc R100 new name.txt\0",
            "old name.txt\0",
            "? untracked.txt\0",
        );

        let result = parse_v2_records(output, 2);

        assert_eq!(result.branch, Some(BranchHead::Named("main".to_owned())));
        // Ahead 3 > max 2 -> clamped and capped; behind 1 <= max -> verbatim.
        assert_eq!(result.ahead, 2);
        assert!(result.ahead_capped);
        assert_eq!(result.behind, 1);
        assert!(!result.behind_capped);

        // The rename is one staged change, keyed by its destination.
        let aggregate = StatusAggregate::from_file_statuses(result.files.values());
        assert_eq!(aggregate.staged, 1);
        assert_eq!(aggregate.untracked, 1);
        assert!(result.files.contains_key(Path::new("new name.txt")));
        assert!(
            !result.files.contains_key(Path::new("old name.txt")),
            "the original-path record must be consumed, not stored as an entry"
        );
        assert!(
            !result
                .files
                .contains_key(Path::new("new name.txt old name.txt")),
            "destination and original paths must not be concatenated"
        );
        let renamed = result
            .files
            .get(Path::new("new name.txt"))
            .expect("renamed file present");
        assert!(renamed.staged);
    }
}
