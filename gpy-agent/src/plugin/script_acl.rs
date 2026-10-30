//! macOS extended-ACL check for script segment trust.
//!
//! The ownership and mode checks read POSIX mode bits only. On macOS a file
//! or directory can also carry an ACL that lets another principal write it
//! while the mode stays `0644`/`0755`, so the mode check alone would accept a
//! script that someone else can rewrite after export. Linux and the other
//! Unixes have no such second permission channel in the checks GPY uses.

use crate::Result;
use std::path::Path;

/// Refuse `path` when an extended ACL grants a principal other than the
/// current user a permission that changes its contents, its directory entries,
/// its ACL or its owner. Deny entries are ignored (macOS home directories carry
/// `everyone deny delete` by default). Fails closed when the ACL cannot be read.
///
/// # Errors
/// Returns an error naming `path` if the ACL cannot be read or holds such an
/// allow entry.
#[cfg(target_os = "macos")]
pub(super) fn check_no_foreign_write_acl(path: &Path) -> Result<()> {
    use crate::Error;
    use exacl::{AclOption, Perm, getfacl};

    const MUTATING: Perm = Perm::WRITE
        .union(Perm::APPEND)
        .union(Perm::DELETE)
        .union(Perm::DELETE_CHILD)
        .union(Perm::WRITESECURITY)
        .union(Perm::CHOWN);

    let shown = path.to_string_lossy().escape_debug().to_string();
    let entries = getfacl(path, AclOption::SYMLINK_ACL)
        .map_err(|e| Error::invalid(format!("cannot read the ACL of '{shown}': {e}")))?;
    let uid = nix::unistd::getuid();
    let user = nix::unistd::User::from_uid(uid)
        .ok()
        .flatten()
        .map(|u| u.name);
    let uid_text = uid.as_raw().to_string();
    for entry in entries {
        if !entry.allow || !entry.perms.intersects(MUTATING) {
            continue;
        }
        let is_current_user = entry.kind == exacl::AclEntryKind::User
            && (entry.name == uid_text || user.as_deref() == Some(entry.name.as_str()));
        if !is_current_user {
            return Err(Error::invalid(format!(
                "'{shown}' has an ACL entry ({} {}) that lets another principal change it; \
                 run `chmod -N` on it to remove the ACL",
                entry.kind,
                if entry.name.is_empty() {
                    "(no name)"
                } else {
                    &entry.name
                }
            )));
        }
    }
    Ok(())
}

/// Only macOS adds ACL-based write access beside the mode bits checked.
///
/// # Errors
/// Never.
#[cfg(all(unix, not(target_os = "macos")))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the macOS check, which can fail"
)]
pub(super) const fn check_no_foreign_write_acl(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests: panics are the assertions"
    )]

    use super::check_no_foreign_write_acl;
    use std::process::Command;

    fn chmod_acl(rule: &str, path: &std::path::Path) {
        let status = Command::new("/bin/chmod")
            .args(["+a", rule])
            .arg(path)
            .status()
            .expect("run chmod");
        assert!(status.success(), "chmod +a {rule:?} failed");
    }

    #[test]
    fn everyone_write_acl_is_refused_but_deny_and_read_acls_pass() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let file = tmp.path().join("seg.sh");
        std::fs::write(&file, "#!/bin/sh\n").expect("write");
        check_no_foreign_write_acl(&file).expect("no ACL passes");

        chmod_acl("everyone deny delete", &file);
        chmod_acl("everyone allow read", &file);
        check_no_foreign_write_acl(&file).expect("deny and read entries pass");

        chmod_acl("everyone allow write", &file);
        let err = check_no_foreign_write_acl(&file).expect_err("allow write must be refused");
        assert!(err.to_string().contains("ACL"), "{err}");
    }
}
