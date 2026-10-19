//! Trust checks for the directory the agent socket is published in.

use crate::warn_log;
use crate::{Error, Result};
use std::path::Path;

/// What [`secure_socket_dir`] does with a directory we own that is group- or
/// world-writable. Decided by where the socket path came from.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirRepair {
    /// GPY's own runtime directory: set it to `0700` and use it.
    Tighten,
    /// A directory the user chose (`GPY_AGENT_SOCKET_PATH`, `--socket`): refuse
    /// it rather than change the mode of a directory GPY does not own.
    Refuse,
}

/// Make the directory `socket_path` is published in safe for it. Socket mode
/// `0600` does not stop anyone who can write the directory from replacing the
/// socket, so refuse a symlink, a non-directory, or a directory owned by
/// another user; one we own that is group/world-writable is handled by `repair`.
///
/// # Errors
/// Fails if the directory cannot be opened or fails a trust check.
#[cfg(unix)]
pub(crate) fn secure_socket_dir(socket_path: &Path, repair: DirRepair) -> Result<()> {
    let dir = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    secure_dir_for_uid(dir, nix::unistd::getuid().as_raw(), repair)
}

/// [`secure_socket_dir`] against an explicit owner `uid`.
///
/// # Errors
/// Fails if `dir` cannot be opened or fails a trust check.
#[cfg(unix)]
fn secure_dir_for_uid(dir: &Path, uid: u32, repair: DirRepair) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let refuse = |problem: &str| {
        Error::ipc(format!(
            "refusing socket directory {}: {problem}. Use a directory you own that \
             only you can write (via XDG_RUNTIME_DIR or GPY_AGENT_SOCKET_PATH)",
            dir.display()
        ))
    };
    let unreadable = |e: std::io::Error| {
        Error::ipc(format!(
            "cannot inspect socket directory {}: {e}",
            dir.display()
        ))
    };

    // O_NOFOLLOW fails on a final-component symlink instead of following it,
    // and every check below acts on this one inode (fstat/fchmod), so the
    // path cannot be swapped between check and repair.
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) if dir.is_symlink() => refuse("it is a symlink"),
            Some(libc::ENOTDIR) => refuse("it is not a directory"),
            _ => unreadable(e),
        })?;
    let meta = handle.metadata().map_err(unreadable)?;
    if meta.uid() != uid {
        return Err(refuse(&format!(
            "it is owned by uid {}, not the current user (uid {uid})",
            meta.uid()
        )));
    }
    if meta.mode() & 0o022 == 0 {
        return Ok(());
    }
    if repair == DirRepair::Refuse {
        return Err(refuse(&format!(
            "it is group/world-writable (mode {:o}); run `chmod 700 {}`",
            meta.mode() & 0o7777,
            dir.display()
        )));
    }

    handle
        .set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(|e| {
            refuse(&format!(
                "it is group/world-writable and chmod 0700 failed: {e}"
            ))
        })?;
    let mode = handle.metadata().map_err(unreadable)?.mode();
    if mode & 0o022 != 0 {
        return Err(refuse(&format!(
            "it is still group/world-writable (mode {:o}) after chmod 0700",
            mode & 0o7777
        )));
    }
    warn_log!(
        "server",
        "Socket directory {} was group/world-writable; set it to 0700",
        dir.display()
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod socket_dir_trust_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{DirRepair, secure_dir_for_uid};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    const BOTH: [DirRepair; 2] = [DirRepair::Tighten, DirRepair::Refuse];

    #[test]
    fn accepts_owned_private_directory() {
        for repair in BOTH {
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod");

            let uid = fs::metadata(&dir).expect("meta").uid();
            secure_dir_for_uid(&dir, uid, repair)
                .expect("an owned, not-writable-by-others dir must pass");
            assert_eq!(mode_of(&dir), 0o755, "a trusted dir is left as it was");
        }
    }

    #[test]
    fn rejects_symlinked_directory() {
        for repair in BOTH {
            let tmp = tempfile::tempdir().expect("temp dir");
            let real = tmp.path().join("real");
            fs::create_dir_all(&real).expect("mkdir real");
            let link = tmp.path().join("gpy-rt");
            symlink(&real, &link).expect("symlink");

            let uid = fs::metadata(&real).expect("meta").uid();
            let err = secure_dir_for_uid(&link, uid, repair)
                .expect_err("a symlink must be rejected even when it points to a dir we own");
            assert!(
                err.to_string().contains(&link.display().to_string()),
                "{err}"
            );
        }
    }

    #[test]
    fn rejects_a_regular_file() {
        for repair in BOTH {
            let tmp = tempfile::tempdir().expect("temp dir");
            let file = tmp.path().join("gpy-rt");
            fs::write(&file, "").expect("write file");

            let uid = fs::metadata(&file).expect("meta").uid();
            secure_dir_for_uid(&file, uid, repair).expect_err("a non-directory must be rejected");
        }
    }

    #[test]
    fn tightens_owned_group_or_world_writable_directory_to_0700() {
        for mode in [0o777, 0o775, 0o1777] {
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).expect("chmod");

            let uid = fs::metadata(&dir).expect("meta").uid();
            secure_dir_for_uid(&dir, uid, DirRepair::Tighten)
                .expect("an owned writable dir is repaired, not refused");
            assert_eq!(mode_of(&dir), 0o700, "mode {mode:o} must be tightened");
        }
    }

    #[test]
    fn refuses_owned_group_or_world_writable_directory_without_touching_it() {
        for mode in [0o777, 0o775, 0o1777] {
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).expect("chmod");

            let uid = fs::metadata(&dir).expect("meta").uid();
            let err = secure_dir_for_uid(&dir, uid, DirRepair::Refuse)
                .expect_err("a user-chosen writable dir is refused, not repaired");
            assert!(
                err.to_string().contains(&dir.display().to_string()),
                "{err}"
            );
            assert_eq!(mode_of(&dir), mode, "mode {mode:o} must be left alone");
        }
    }

    #[test]
    fn rejects_directory_owned_by_another_uid_without_touching_it() {
        for repair in BOTH {
            // Can't chown without privilege; claim a non-owner uid instead.
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).expect("chmod");

            let foreign_uid = fs::metadata(&dir).expect("meta").uid().wrapping_add(1);
            secure_dir_for_uid(&dir, foreign_uid, repair)
                .expect_err("a dir not owned by the expected uid must be rejected");
            assert_eq!(mode_of(&dir), 0o777, "a foreign dir is never chmod'ed");
        }
    }

    fn mode_of(path: &std::path::Path) -> u32 {
        fs::metadata(path).expect("meta").mode() & 0o7777
    }
}
