//! The `gpy-init` block `gpy setup` writes into shell rc files.
//!
//! The marker lines are the contract `scripts/uninstall.*` remove the block by:
//! [`BLOCK_START`], a `# <header>` line, the body, then [`BLOCK_END`].

use crate::Result;
use std::fs;
use std::path::Path;

/// First line of the rc-file block; `scripts/uninstall.*` match on it.
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

/// `line` without its trailing `\n` and `\r`.
fn trim_eol(line: &[u8]) -> &[u8] {
    let without_lf = line.strip_suffix(b"\n").unwrap_or(line);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

/// Whether any line of `text` is the [`BLOCK_START`] marker.
fn has_start(text: &[u8]) -> bool {
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
}
