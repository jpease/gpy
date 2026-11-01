//! The `gpy-init` block `gpy setup` writes into shell rc files.
//!
//! The marker lines are the contract `gpy uninstall` removes the block by:
//! [`BLOCK_START`], a `# <header>` line, the body, then [`BLOCK_END`].

use crate::{Error, Result};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

/// First line of the rc-file block; `gpy uninstall` matches on it.
pub(super) const BLOCK_START: &str = "# >>> gpy-init >>>";
/// Last line of the rc-file block.
pub(super) const BLOCK_END: &str = "# <<< gpy-init <<<";
/// Tag in the block header for an rc file `gpy setup` itself created, so the
/// uninstaller may delete it again.
pub(super) const CREATED_NOTE: &str = " (gpy-created-file)";

/// What [`ensure_block`] did to an rc file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockChange {
    /// A new block was appended (the file may have been created).
    Added,
    /// The file already held the current block, or a start marker with no
    /// complete block after it; nothing was written.
    Unchanged,
    /// An older block's body was replaced in place.
    Upgraded,
    /// The block's body is neither the current nor an older text; left alone.
    KeptEdited,
}

/// What [`remove_blocks`] found and did in one rc file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent facts the caller reports one by one; an enum per fact would add nothing"
)]
pub(super) struct RemoveOutcome {
    /// At least one complete block was removed.
    pub(super) removed: bool,
    /// The file carried [`CREATED_NOTE`] and was deleted once empty.
    pub(super) deleted_file: bool,
    /// A [`BLOCK_START`] with no matching [`BLOCK_END`] was left in place.
    pub(super) unmatched_start: bool,
    /// A [`BLOCK_END`] with no [`BLOCK_START`] before it was left in place.
    pub(super) unmatched_end: bool,
}

/// Byte range of a block's body lines, from the first body line through the
/// newline that ends the last one.
struct BlockSpan {
    body_start: usize,
    body_end: usize,
}

/// Append a `gpy-init` block to `rc`, creating it when missing; `false` when
/// a block is already there. A file this call creates gets [`CREATED_NOTE`].
pub(super) fn append_block(rc: &Path, header: &str, body: &str) -> Result<bool> {
    use std::io::Write;

    let existing = match fs::read_to_string(rc) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    if existing
        .as_deref()
        .is_some_and(|text| text.lines().any(|line| line == BLOCK_START))
    {
        return Ok(false);
    }
    if let Some(parent) = rc.parent() {
        fs::create_dir_all(parent)?;
    }
    let note = if existing.is_none() { CREATED_NOTE } else { "" };
    let mut file = fs::OpenOptions::new().create(true).append(true).open(rc)?;
    write!(
        file,
        "\n{BLOCK_START}\n# {header}{note}\n{body}\n{BLOCK_END}\n"
    )?;
    Ok(true)
}

/// Whether `rc` holds a [`BLOCK_START`] line. An unreadable file reports `false`.
pub(super) fn has_block(rc: &Path) -> bool {
    fs::read(rc).is_ok_and(|bytes| has_start(&bytes))
}

/// Make `rc` carry the block with `body`. A missing block is appended. A block
/// whose body is one of `legacy_bodies` has just its body lines replaced; every
/// other byte of the file stays. Any other body is left alone.
pub(super) fn ensure_block(
    rc: &Path,
    header: &str,
    body: &str,
    legacy_bodies: &[String],
) -> Result<BlockChange> {
    let raw = match fs::read(rc) {
        Ok(read) => Some(read),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    let Some(bytes) = raw.filter(|candidate| has_start(candidate)) else {
        return Ok(if append_block(rc, header, body)? {
            BlockChange::Added
        } else {
            BlockChange::Unchanged
        });
    };
    let Some(span) = find_block(&bytes) else {
        return Ok(BlockChange::Unchanged);
    };
    let (Some(head), Some(current), Some(tail)) = (
        bytes.get(..span.body_start),
        bytes.get(span.body_start..span.body_end),
        bytes.get(span.body_end..),
    ) else {
        return Ok(BlockChange::Unchanged);
    };
    let trimmed = current.strip_suffix(b"\n").unwrap_or(current);
    if trimmed == body.as_bytes() {
        return Ok(BlockChange::Unchanged);
    }
    if !legacy_bodies
        .iter()
        .any(|legacy| legacy.as_bytes() == trimmed)
    {
        return Ok(BlockChange::KeptEdited);
    }
    let mut updated = Vec::with_capacity(bytes.len().saturating_add(body.len()));
    updated.extend_from_slice(head);
    updated.extend_from_slice(body.as_bytes());
    updated.push(b'\n');
    updated.extend_from_slice(tail);
    crate::fs_util::write_atomically(rc, &updated)?;
    Ok(BlockChange::Upgraded)
}

/// Remove every complete `gpy-init` block from `rc`, with the blank line (or,
/// for a block that ends a file lacking a trailing newline, the newline) that
/// [`append_block`] put before it, so the file returns to its pre-setup bytes.
/// Works on bytes, so any encoding is edited correctly. Orphaned markers stay
/// in place and are reported. A file tagged [`CREATED_NOTE`] that is left
/// empty is deleted.
///
/// The file is replaced atomically. Where the rename is refused (a bind-mounted
/// file, a writable file in a read-only directory) the same bytes are written
/// in place instead.
///
/// # Errors
/// Fails, naming `rc`, when it cannot be read, rewritten or deleted.
pub(super) fn remove_blocks(rc: &Path) -> Result<RemoveOutcome> {
    let bytes =
        fs::read(rc).map_err(|err| Error::io_context(format!("reading {}", rc.display()), &err))?;
    let stripped = strip_blocks(&bytes);
    let mut outcome = stripped.outcome;
    if !outcome.removed {
        return Ok(outcome);
    }
    if stripped.created && stripped.bytes.iter().all(u8::is_ascii_whitespace) {
        fs::remove_file(rc)
            .map_err(|err| Error::io_context(format!("removing {}", rc.display()), &err))?;
        outcome.deleted_file = true;
        return Ok(outcome);
    }
    let rewrite_error =
        |err: &std::io::Error| Error::io_context(format!("rewriting {}", rc.display()), err);
    match crate::fs_util::write_atomically(rc, &stripped.bytes) {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::PermissionDenied | ErrorKind::ResourceBusy | ErrorKind::CrossesDevices
            ) =>
        {
            fs::write(rc, &stripped.bytes).map_err(|write_err| rewrite_error(&write_err))?;
        }
        Err(err) => return Err(rewrite_error(&err)),
    }
    Ok(outcome)
}

/// `text` without its complete blocks, and what was seen on the way.
struct Stripped {
    bytes: Vec<u8>,
    /// `removed`, `unmatched_start` and `unmatched_end` are set; `deleted_file` is not.
    outcome: RemoveOutcome,
    created: bool,
}

/// Cut every complete block out of `text`; see [`remove_blocks`].
fn strip_blocks(text: &[u8]) -> Stripped {
    let lines: Vec<&[u8]> = text.split_inclusive(|byte| *byte == b'\n').collect();
    let mut cut = vec![false; lines.len()];
    let mut chomp = vec![false; lines.len()];
    let mut found = Stripped {
        bytes: Vec::with_capacity(text.len()),
        outcome: RemoveOutcome::default(),
        created: false,
    };
    let mut open: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        let marker = trim_eol(line);
        if marker == BLOCK_START.as_bytes() {
            // A second start before any end orphans the first.
            found.outcome.unmatched_start |= open.is_some();
            open = Some(index);
        } else if marker == BLOCK_END.as_bytes() {
            let Some(start) = open.take() else {
                found.outcome.unmatched_end = true;
                continue;
            };
            found.outcome.removed = true;
            found.created |= lines
                .get(start.saturating_add(1)..index)
                .and_then(|inner| inner.first())
                .is_some_and(|header| trim_eol(header).ends_with(CREATED_NOTE.as_bytes()));
            let ends_file = index.saturating_add(1) == lines.len();
            mark_block(&lines, start, index, ends_file, &mut cut, &mut chomp);
        }
    }
    found.outcome.unmatched_start |= open.is_some();
    for ((line, skipped), chomped) in lines.iter().zip(&cut).zip(&chomp) {
        if *skipped {
            continue;
        }
        let keep = if *chomped {
            line.strip_suffix(b"\n").unwrap_or(*line)
        } else {
            *line
        };
        found.bytes.extend_from_slice(keep);
    }
    found
}

/// Flag the block `start..=end` for removal, plus the line before it that
/// [`append_block`] added: a blank line, or the newline of a last line that
/// had none when the block ends the file.
fn mark_block(
    lines: &[&[u8]],
    start: usize,
    end: usize,
    ends_file: bool,
    cut: &mut [bool],
    chomp: &mut [bool],
) {
    let count = end.saturating_sub(start).saturating_add(1);
    for flag in cut.iter_mut().skip(start).take(count) {
        *flag = true;
    }
    let Some(previous) = start.checked_sub(1) else {
        return;
    };
    if cut.get(previous) != Some(&false) {
        return;
    }
    if lines.get(previous).is_some_and(|line| *line == b"\n") {
        if let Some(flag) = cut.get_mut(previous) {
            *flag = true;
        }
    } else if ends_file && let Some(flag) = chomp.get_mut(previous) {
        *flag = true;
    }
}

/// `line` without its trailing `\n` and `\r`.
fn trim_eol(line: &[u8]) -> &[u8] {
    let without_lf = line.strip_suffix(b"\n").unwrap_or(line);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

/// Whether any line of `text` is the [`BLOCK_START`] marker.
pub(super) fn has_start(text: &[u8]) -> bool {
    text.split(|byte| *byte == b'\n')
        .any(|line| trim_eol(line) == BLOCK_START.as_bytes())
}

/// The first complete block in `text`: a start marker, a `# ` header line, body
/// lines, then the end marker. `None` when there is no such block.
fn find_block(text: &[u8]) -> Option<BlockSpan> {
    enum Stage {
        Seek,
        Header,
        Body,
    }
    let mut stage = Stage::Seek;
    let mut position = 0usize;
    let mut body_start = 0usize;
    for raw in text.split_inclusive(|byte| *byte == b'\n') {
        let line_start = position;
        position = position.saturating_add(raw.len());
        let line = trim_eol(raw);
        match stage {
            Stage::Seek => {
                if line == BLOCK_START.as_bytes() {
                    stage = Stage::Header;
                }
            }
            Stage::Header => {
                if !line.starts_with(b"# ") {
                    return None;
                }
                body_start = position;
                stage = Stage::Body;
            }
            Stage::Body => {
                if line == BLOCK_END.as_bytes() {
                    return Some(BlockSpan {
                        body_start,
                        body_end: line_start,
                    });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = "source \"/x/gpy.bash\"";
    const CURRENT: &str = "case $- in\n    *i*) source \"/x/gpy.bash\" ;;\nesac";

    fn legacy() -> Vec<String> {
        vec![LEGACY.to_owned()]
    }

    #[test]
    fn append_block_is_idempotent_and_tags_created_files() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");

        assert!(append_block(&rc, "hdr", "body").unwrap());
        assert!(!append_block(&rc, "hdr", "body").unwrap());

        let text = fs::read_to_string(&rc).unwrap();
        assert_eq!(text.matches(BLOCK_START).count(), 1);
        assert!(text.contains("# hdr (gpy-created-file)"), "{text}");
    }

    #[test]
    fn append_block_keeps_existing_content_and_skips_the_created_tag() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, "export A=1\n").unwrap();

        assert!(append_block(&rc, "hdr", "body").unwrap());

        let text = fs::read_to_string(&rc).unwrap();
        assert!(text.starts_with("export A=1\n"));
        assert!(!text.contains("gpy-created-file"));
    }

    #[test]
    fn ensure_block_appends_then_leaves_the_current_block_alone() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");

        assert_eq!(
            ensure_block(&rc, "hdr", CURRENT, &legacy()).unwrap(),
            BlockChange::Added
        );
        let first = fs::read(&rc).unwrap();
        assert_eq!(
            ensure_block(&rc, "hdr", CURRENT, &legacy()).unwrap(),
            BlockChange::Unchanged
        );
        assert_eq!(fs::read(&rc).unwrap(), first);
    }

    #[test]
    fn ensure_block_upgrades_a_legacy_body_and_keeps_surrounding_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        let before = "export A=1\r\n\n# >>> gpy-init >>>\n# GPY shell integration\n";
        let after = "# <<< gpy-init <<<\nalias b=2";
        fs::write(&rc, format!("{before}{LEGACY}\n{after}")).unwrap();

        let change = ensure_block(&rc, "hdr", CURRENT, &legacy()).unwrap();

        assert_eq!(change, BlockChange::Upgraded);
        assert_eq!(
            fs::read_to_string(&rc).unwrap(),
            format!("{before}{CURRENT}\n{after}")
        );
    }

    #[test]
    #[cfg(unix)]
    fn ensure_block_writes_through_a_symlinked_rc_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real_rc");
        let link = dir.path().join("rc");
        fs::write(
            &target,
            format!("# >>> gpy-init >>>\n# hdr\n{LEGACY}\n# <<< gpy-init <<<\n"),
        )
        .unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let change = ensure_block(&link, "hdr", CURRENT, &legacy()).unwrap();

        assert_eq!(change, BlockChange::Upgraded);
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert!(fs::read_to_string(&target).unwrap().contains(CURRENT));
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn ensure_block_reports_an_edited_block() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        let text = "# >>> gpy-init >>>\n# hdr\nsource \"/x/gpy.bash\" # mine\n# <<< gpy-init <<<\n";
        fs::write(&rc, text).unwrap();

        let change = ensure_block(&rc, "hdr", CURRENT, &legacy()).unwrap();

        assert_eq!(change, BlockChange::KeptEdited);
        assert_eq!(fs::read_to_string(&rc).unwrap(), text);
    }

    #[test]
    fn ensure_block_leaves_an_orphaned_start_alone() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        let text = "export A=1\n# >>> gpy-init >>>\n# hdr\nsource \"/x/gpy.bash\"\n";
        fs::write(&rc, text).unwrap();

        let change = ensure_block(&rc, "hdr", CURRENT, &legacy()).unwrap();

        assert_eq!(change, BlockChange::Unchanged);
        assert_eq!(fs::read_to_string(&rc).unwrap(), text);
    }

    #[test]
    #[cfg(unix)]
    fn ensure_block_keeps_the_original_when_the_directory_is_read_only() {
        use std::os::unix::fs::PermissionsExt;

        if nix::unistd::Uid::effective().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        let text = format!("# >>> gpy-init >>>\n# hdr\n{LEGACY}\n# <<< gpy-init <<<\n");
        fs::write(&rc, &text).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();

        let result = ensure_block(&rc, "hdr", CURRENT, &legacy());
        let after = fs::read_to_string(&rc).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err(), "a read-only directory must fail the write");
        assert_eq!(after, text);
    }

    const BLOCK: &str = "# >>> gpy-init >>>\n# hdr\nbody\n# <<< gpy-init <<<\n";

    #[test]
    fn remove_blocks_round_trips_append_block() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, "export A=1\n").unwrap();
        append_block(&rc, "hdr", "body").unwrap();

        let outcome = remove_blocks(&rc).unwrap();

        assert!(outcome.removed && !outcome.deleted_file);
        assert_eq!(fs::read(&rc).unwrap(), b"export A=1\n");
    }

    #[test]
    fn remove_blocks_round_trips_append_block_on_a_file_without_a_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, "export A=1").unwrap();
        append_block(&rc, "hdr", "body").unwrap();

        remove_blocks(&rc).unwrap();

        assert_eq!(fs::read(&rc).unwrap(), b"export A=1");
    }

    #[test]
    fn remove_blocks_reports_an_orphaned_start() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        let text = "a\n# >>> gpy-init >>>\n# hdr\nb\n";
        fs::write(&rc, text).unwrap();

        let outcome = remove_blocks(&rc).unwrap();

        assert!(!outcome.removed && outcome.unmatched_start && !outcome.unmatched_end);
        assert_eq!(fs::read_to_string(&rc).unwrap(), text);
    }

    #[test]
    fn remove_blocks_reports_a_second_start_as_orphaning_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, format!("# >>> gpy-init >>>\nx\n\n{BLOCK}")).unwrap();

        let outcome = remove_blocks(&rc).unwrap();

        assert!(outcome.removed && outcome.unmatched_start);
        assert_eq!(fs::read_to_string(&rc).unwrap(), "# >>> gpy-init >>>\nx\n");
    }

    #[test]
    fn remove_blocks_deletes_a_created_file_left_empty() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        append_block(&rc, "hdr", "body").unwrap();

        let outcome = remove_blocks(&rc).unwrap();

        assert!(outcome.removed && outcome.deleted_file);
        assert!(!rc.exists());
    }

    #[test]
    fn remove_blocks_keeps_a_missing_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, format!("p\n\n{BLOCK}last")).unwrap();

        remove_blocks(&rc).unwrap();

        assert_eq!(fs::read_to_string(&rc).unwrap(), "p\nlast");
    }

    #[test]
    fn remove_blocks_edits_a_non_utf8_file_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        let mut bytes = b"# caf\xe9\n\n".to_vec();
        bytes.extend_from_slice(BLOCK.as_bytes());
        bytes.extend_from_slice(b"tail \xff\n");
        fs::write(&rc, &bytes).unwrap();

        remove_blocks(&rc).unwrap();

        assert_eq!(fs::read(&rc).unwrap(), b"# caf\xe9\ntail \xff\n");
    }

    #[test]
    fn remove_blocks_names_the_path_when_the_file_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::create_dir_all(&rc).unwrap();

        let err = remove_blocks(&rc).unwrap_err();

        let message = crate::error::user_message(&err);
        assert!(message.contains(&rc.display().to_string()), "{message}");
    }

    #[test]
    #[cfg(unix)]
    fn remove_blocks_falls_back_to_an_in_place_write_when_rename_is_refused() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        if nix::unistd::Uid::effective().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, format!("keep\n\n{BLOCK}")).unwrap();
        let inode = fs::metadata(&rc).unwrap().ino();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();

        let result = remove_blocks(&rc);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.unwrap().removed);
        assert_eq!(fs::read_to_string(&rc).unwrap(), "keep\n");
        assert_eq!(fs::metadata(&rc).unwrap().ino(), inode);
    }
}
