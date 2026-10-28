//! Ancestry check for script segments: who can replace the plugin root.
//!
//! Renaming or replacing a directory needs write permission on its *parent*,
//! not on the directory itself, so a plugin root that passes the ownership
//! check can still be swapped out after export when any ancestor is writable by
//! another user. Every ancestor of the canonical root is therefore checked once
//! at export, keeping the shells free of per-prompt `stat` calls.

#[cfg(unix)]
use crate::Error;
use crate::Result;
use std::path::Path;

/// Mode bits that let a group or world member create, rename or remove entries.
#[cfg(unix)]
const GROUP_WORLD_WRITE: u32 = 0o022;
/// The sticky bit: only an entry's owner or the directory's owner may rename it.
#[cfg(unix)]
const STICKY: u32 = 0o1000;

/// Refuse a directory above the plugin root through which another user could
/// replace the root (or a component beneath it).
///
/// A directory passes when it is owned by the current user or root and is not
/// group- or world-writable; when it is writable but sticky and owned by the
/// current user or root (the kernel then lets only the root's own owner, the
/// user, rename or remove it); or when it is group-writable only (not
/// world-writable) through the user's own private group (see
/// [`group_is_private`]), as `$HOME` is under user-private-group distros with
/// umask 002. Callers pass canonical paths.
///
/// # Errors
/// Returns an error naming `path` if it cannot be inspected, is owned by
/// another non-root user, or is group- or world-writable without the sticky
/// bit.
#[cfg(unix)]
pub(super) fn check_ancestor_not_replaceable(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let shown = path.to_string_lossy().escape_debug().to_string();
    let meta = path
        .symlink_metadata()
        .map_err(|e| Error::invalid(format!("cannot inspect '{shown}': {e}")))?;
    let user = nix::unistd::getuid().as_raw();
    if meta.uid() != user && meta.uid() != 0 {
        return Err(Error::invalid(format!(
            "ancestor directory '{shown}' is owned by uid {}, not by you (uid {user}) or \
             root, so its owner could replace the plugin",
            meta.uid()
        )));
    }
    let mode = meta.mode();
    let only_private_group_writes = mode & 0o002 == 0 && group_is_private(meta.gid());
    if mode & GROUP_WORLD_WRITE != 0 && mode & STICKY == 0 && !only_private_group_writes {
        return Err(Error::invalid(format!(
            "ancestor directory '{shown}' is group- or world-writable without the sticky bit \
             (mode {:o}), so another user could replace the plugin after export; run \
             `chmod go-w` on it or move the plugin elsewhere",
            mode & 0o7777
        )));
    }
    Ok(())
}

/// Whether `gid` is a group only the current user is in: the process's primary
/// group, named like the user (a user-private group), with no member other
/// than the user or root. A shared group such as macOS `staff` is never
/// private, because the directory service does not list every member in it.
/// Supplementary groups are not considered: a private group is the primary one.
#[cfg(unix)]
pub(super) fn group_is_private(gid: u32) -> bool {
    use nix::unistd::{Gid, Group, User, getgid, getuid};

    let wanted = Gid::from_raw(gid);
    if getgid() != wanted {
        return false;
    }
    let Ok(Some(user)) = User::from_uid(getuid()) else {
        return false;
    };
    let Ok(Some(group)) = Group::from_gid(wanted) else {
        return false;
    };
    group.name == user.name
        && group
            .mem
            .iter()
            .all(|member| *member == user.name || member == "root")
}

/// Windows builds the CLI only and runs no script segments: nothing to check.
///
/// # Errors
/// Never.
#[cfg(not(unix))]
pub(super) fn check_ancestor_not_replaceable(_path: &Path) -> Result<()> {
    Ok(())
}
