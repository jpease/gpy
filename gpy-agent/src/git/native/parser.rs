//! Parser for `git status --porcelain=v2 --branch` output.
//!
//! The native Git backend gathers branch metadata and file status in one
//! subprocess call, then this module converts Git's stable porcelain records
//! into GPY's prompt-focused status counts and per-file flags. Keeping parsing
//! separate from process execution makes edge cases easy to unit test.

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

/// The branch identity read from a `# branch.head` porcelain-v2 header line,
/// before any subprocess-based resolution.
///
/// `Detached`/`Initial` carry no further information because the porcelain
/// header line itself doesn't include the commit hash or branch name for
/// those cases — resolving them requires a `git rev-parse`/`symbolic-ref`
/// subprocess call, which is why they stay unresolved through the pure
/// parsing pass and are only resolved afterward, by the native git backend's
/// status-fetching entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchHead {
    /// An ordinary branch name, taken verbatim from the header line.
    Named(String),
    /// `(detached)` — HEAD points directly at a commit, not a branch.
    Detached,
    /// `(initial)` — a brand-new repository with no commits yet.
    Initial,
}

impl BranchHead {
    /// Parses a `# branch.head` header line's content (the text after the
    /// `# branch.head ` prefix) into a [`BranchHead`]. Pure: no subprocess
    /// calls, no filesystem access.
    #[must_use]
    fn parse(header_content: &str) -> Self {
        match header_content {
            "(detached)" => Self::Detached,
            "(initial)" => Self::Initial,
            named => Self::Named(named.to_owned()),
        }
    }
}

/// The accumulated result of parsing Git porcelain v2 status output.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags (cap/detached), not a state machine"
)]
pub struct V2ParseResult {
    /// The current branch name.
    pub branch: String,
    /// Whether HEAD is detached (not on a branch).
    pub detached: bool,
    /// Number of commits ahead of upstream.
    pub ahead: u32,
    /// Number of commits behind upstream.
    pub behind: u32,
    /// Whether the ahead count was capped due to `max_ahead_behind`.
    pub ahead_capped: bool,
    /// Whether the behind count was capped due to `max_ahead_behind`.
    pub behind_capped: bool,
    /// Number of staged changes.
    pub staged: u32,
    /// Number of unstaged changes.
    pub unstaged: u32,
    /// Number of untracked files.
    pub untracked: u32,
    /// Number of conflicted files.
    pub conflicts: u32,
    /// Map of file paths to their status.
    pub files: HashMap<PathBuf, FileStatus>,
}

/// The result of PURELY parsing Git porcelain v2 status output.
///
/// No subprocess calls, no filesystem access. `branch` is left unresolved
/// ([`BranchHead`], not a final `String`) because resolving `Detached`/
/// `Initial` needs a `git rev-parse`/`symbolic-ref` subprocess call; the
/// caller resolves it immediately after calling [`parse_v2_records`].
pub struct V2ParseRecords {
    /// The unresolved branch identity.
    pub branch: BranchHead,
    /// Number of commits ahead of upstream.
    pub ahead: u32,
    /// Number of commits behind upstream.
    pub behind: u32,
    /// Whether the ahead count was capped due to `max_ahead_behind`.
    pub ahead_capped: bool,
    /// Whether the behind count was capped due to `max_ahead_behind`.
    pub behind_capped: bool,
    /// Number of staged changes.
    pub staged: u32,
    /// Number of unstaged changes.
    pub unstaged: u32,
    /// Number of untracked files.
    pub untracked: u32,
    /// Number of conflicted files.
    pub conflicts: u32,
    /// Map of file paths to their status.
    pub files: HashMap<PathBuf, FileStatus>,
}

/// Accumulator state for parsing Git porcelain v2 output line by line.
pub struct V2ParseState {
    /// The unresolved branch identity.
    pub branch: BranchHead,
    /// Number of commits ahead of upstream.
    pub ahead: u32,
    /// Number of commits behind upstream.
    pub behind: u32,
    /// Whether the ahead count was capped due to `max_ahead_behind`.
    pub ahead_capped: bool,
    /// Whether the behind count was capped due to `max_ahead_behind`.
    pub behind_capped: bool,
    /// Number of staged changes.
    pub staged: u32,
    /// Number of unstaged changes.
    pub unstaged: u32,
    /// Number of untracked files.
    pub untracked: u32,
    /// Number of conflicted files.
    pub conflicts: u32,
    /// Map of file paths to their status.
    pub files: HashMap<PathBuf, FileStatus>,
}

impl V2ParseState {
    /// Creates a new `V2ParseState` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Processes untracked file information, updating counts and file map.
    pub(crate) fn process_untracked(&mut self, path: PathBuf) {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "count of untracked files reported by one `git status` call; cannot realistically approach u32::MAX"
        )]
        {
            self.untracked += 1;
        }
        let status = FileStatus {
            untracked: true,
            ..Default::default()
        };
        self.files.insert(path, status);
    }

    /// Processes changed file information (staged/unstaged/conflicted), updating counts and file map.
    pub(crate) fn process_changed(&mut self, parsed: ParsedChangedFile) {
        let ParsedChangedFile { path, status } = parsed;

        if status.conflicted {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "count of conflicted files reported by one `git status` call; cannot realistically approach u32::MAX"
            )]
            {
                self.conflicts += 1;
            }
        } else {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "counts of staged/unstaged files reported by one `git status` call; cannot realistically approach u32::MAX"
            )]
            {
                if status.staged {
                    self.staged += 1;
                }
                if status.unstaged {
                    self.unstaged += 1;
                }
            }
        }

        self.files.insert(path, status);
    }

    /// Converts the accumulated state into a `V2ParseRecords`.
    #[must_use]
    pub fn into_records(self) -> V2ParseRecords {
        V2ParseRecords {
            branch: self.branch,
            ahead: self.ahead,
            behind: self.behind,
            ahead_capped: self.ahead_capped,
            behind_capped: self.behind_capped,
            staged: self.staged,
            unstaged: self.unstaged,
            untracked: self.untracked,
            conflicts: self.conflicts,
            files: self.files,
        }
    }
}

impl Default for V2ParseState {
    fn default() -> Self {
        Self {
            branch: BranchHead::Named(String::from("main")),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            files: HashMap::new(),
        }
    }
}

/// Parses `output` into a [`V2ParseRecords`].
///
/// `output` is `git status --porcelain=v2 --branch -z` bytes, already UTF-8
/// decoded. Pure: no subprocess calls, no filesystem access — unlike the
/// native git backend's combined parse-and-resolve status path, this never
/// needs a repo path or a timeout, since `Detached`/`Initial` branch heads
/// are left unresolved rather than looked up via a subprocess.
#[must_use]
pub fn parse_v2_records(output: &str, max_ahead_behind: usize) -> V2ParseRecords {
    let mut state = V2ParseState::new();

    // `--porcelain=v2 -z` emits NUL-terminated records (including headers), so
    // paths are never quoted and may legitimately contain spaces, tabs, or
    // newlines. A rename/copy (type `2`) record is immediately followed by a
    // separate NUL-delimited original-path record, which must be consumed
    // rather than parsed as its own entry.
    let records: Vec<&str> = output.split('\0').filter(|r| !r.is_empty()).collect();
    let mut index = 0_usize;
    while let Some(&record) = records.get(index) {
        if let Some(header_content) = record.strip_prefix("# branch.head ") {
            state.branch = BranchHead::parse(header_content);
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
                // Consume the trailing original-path record of a type `2` entry.
                index = index.saturating_add(1);
            }
        }

        index = index.saturating_add(1);
    }
    state.into_records()
}

/// Parses the `branch.ab` (ahead/behind) part of Git porcelain v2 output.
///
/// When `max_ahead_behind` is non-zero it is an inclusive ceiling: counts above
/// it are clamped to it and the corresponding `*_capped` flag is set, signalling
/// that the rendered value should carry a `+` suffix. A value exactly equal to
/// the maximum is reported verbatim (nothing was omitted). A maximum of `0` means
/// unlimited, so the full counts are always returned with both flags clear.
///
/// Returns: `(ahead, behind, ahead_capped, behind_capped)`
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

/// Parses a two-character Git porcelain v2 file status code (XY).
///
/// X (first char): staging area status
/// Y (second char): working tree status
///
/// Conflict patterns:
/// - `UU` = both modified
/// - `AA` = both added
/// - `DD` = both deleted
/// - Any `U` = unmerged conflict
///
/// Returns: (`is_conflict`, `is_staged`, `is_unstaged`)
#[must_use]
pub fn parse_v2_file_status(xy: &str) -> (bool, bool, bool) {
    // Git porcelain v2 status codes:
    // - `M` = modified
    // - `R` = renamed
    // - `C` = copied
    // - `?` = untracked
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

    // Conflicts: unmerged marker, or both added/deleted (merge conflicts)
    let is_conflict = x == UNMERGED
        || y == UNMERGED
        || (x == ADDED && y == ADDED)
        || (x == DELETED && y == DELETED);

    // Staged: first char shows staging area has changes
    let is_staged = x != UNMODIFIED && x != UNTRACKED;

    // Unstaged: second char shows working tree has changes
    let is_unstaged = y != UNMODIFIED && y != UNTRACKED;

    (is_conflict, is_staged, is_unstaged)
}

/// Parses a Git porcelain v2 untracked record (starts with `? `).
///
/// Records are taken from NUL-delimited `-z` output, so the path is used
/// verbatim (Git does not quote or escape paths in `-z` mode and a trailing
/// space could be a legitimate part of the filename).
///
/// Returns: `Some(path)` if parsed successfully.
#[must_use]
pub fn parse_v2_untracked_line(line: &str) -> Option<PathBuf> {
    let filename = line.strip_prefix("? ")?;
    if filename.is_empty() {
        return None;
    }
    Some(PathBuf::from(filename))
}

/// Parses a Git porcelain v2 changed record (type `1`, `2`, or `u`).
///
/// The record types have distinct field layouts before the path:
/// - Type 1 (ordinary change): `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
///   — 8 metadata fields, then the path.
/// - Type 2 (rename/copy): `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>`
///   — 9 metadata fields (the extra `<Xscore>`), then the **destination** path.
/// - Type u (unmerged/conflict): `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
///   — 10 metadata fields (three stage hashes), then the path.
///
/// In `-z` mode a type `2` record's original path arrives as a separate
/// NUL-delimited record (handled by the caller), so the record parsed here
/// contains only the destination path, which is the correct file-map key. The
/// `\t` split additionally keeps this correct for newline-mode `<dest>\t<orig>`.
///
/// Returns: `Some(ParsedChangedFile)` if parsed successfully.
#[must_use]
pub fn parse_v2_changed_line(line: &str) -> Option<ParsedChangedFile> {
    let metadata_fields: usize = match line.as_bytes().first()? {
        b'1' => 8,
        b'2' => 9,
        b'u' => 10,
        _ => return None,
    };

    // Split off exactly the fixed metadata fields; the final piece is the path
    // remainder, kept intact even when the filename itself contains spaces.
    let mut fields = line.splitn(metadata_fields.checked_add(1)?, ' ');
    let _record_type = fields.next()?;
    let xy = fields.next()?;
    for _ in 0..metadata_fields.checked_sub(2)? {
        fields.next()?;
    }
    let path_field = fields.next()?;

    let dest = path_field.split('\t').next().unwrap_or(path_field);
    if dest.is_empty() {
        return None;
    }
    let path = PathBuf::from(dest);

    let (is_conflict, is_staged, is_unstaged) = parse_v2_file_status(xy);

    let mut status = FileStatus::default();
    if is_conflict {
        status.conflicted = true;
    }
    if is_staged {
        status.staged = true;
    }
    if is_unstaged {
        status.unstaged = true;
    }

    Some(ParsedChangedFile { path, status })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
#[allow(clippy::panic)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;

    // --- #181: max_ahead_behind capping -------------------------------------

    #[test]
    fn ahead_behind_below_limit_is_uncapped() {
        let (ahead, behind, ahead_capped, behind_capped) = parse_v2_ahead_behind("+1 -0", 2);
        assert_eq!((ahead, behind), (1, 0));
        assert!(!ahead_capped);
        assert!(!behind_capped);
    }

    #[test]
    fn ahead_behind_exactly_at_limit_is_not_capped() {
        // A value equal to the maximum omitted nothing, so it must not be flagged.
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

    // --- #180: changed-record path extraction -------------------------------

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
        // Regression for #180: the destination must be the file-map key, not the
        // destination joined with the original path.
        let record = "2 R. N... 100644 100644 100644 abc abc R100 new name.txt";
        let parsed = parse_v2_changed_line(record).expect("type 2 record parses");
        assert_eq!(parsed.path, PathBuf::from("new name.txt"));
        assert!(parsed.status.staged);
    }

    #[test]
    fn type2_rename_handles_tab_separated_original_path() {
        // Newline-mode safety net: `<dest>\t<orig>` keeps only the destination.
        let record = "2 R. N... 100644 100644 100644 abc abc R100 dest.txt\torig.txt";
        let parsed = parse_v2_changed_line(record).expect("type 2 record parses");
        assert_eq!(parsed.path, PathBuf::from("dest.txt"));
    }

    #[test]
    fn unmerged_record_is_parsed_as_a_conflict_with_correct_path() {
        // Regression guard: type `u` (unmerged) records have 10 metadata fields
        // (four modes + three stage hashes) before the path.
        let record = "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflicted file.txt";
        let parsed = parse_v2_changed_line(record).expect("unmerged record parses");
        assert_eq!(parsed.path, PathBuf::from("conflicted file.txt"));
        assert!(parsed.status.conflicted);
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
        // `-z` paths are unquoted; a literal tab must survive intact.
        let parsed = parse_v2_untracked_line("? weird\tname.txt").expect("untracked parses");
        assert_eq!(parsed, PathBuf::from("weird\tname.txt"));
    }

    #[test]
    fn untracked_record_requires_a_filename() {
        assert!(parse_v2_untracked_line("? ").is_none());
        assert!(parse_v2_untracked_line("not untracked").is_none());
    }

    // --- #604: pure branch-head parsing (no subprocess, no repo) ------------

    #[test]
    fn parse_v2_records_named_header_yields_branch_head_named() {
        let output = "# branch.head main\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, BranchHead::Named("main".to_owned()));
    }

    #[test]
    fn parse_v2_records_detached_header_yields_branch_head_detached() {
        let output = "# branch.head (detached)\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, BranchHead::Detached);
    }

    #[test]
    fn parse_v2_records_initial_header_yields_branch_head_initial() {
        let output = "# branch.head (initial)\0";
        let result = parse_v2_records(output, 0);
        assert_eq!(result.branch, BranchHead::Initial);
    }
}
