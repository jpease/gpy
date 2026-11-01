//! IPC handler for segment requests that carry their own response data
//! (directory, clock, duration, character, exit status); the agent echoes it
//! back, plus a writability `stat` for directories.

use crate::ipc::Response;

/// Directory segment: the client's logical `display_path`, plus whether the
/// canonical `path` is read-only.
#[must_use]
pub fn directory(path: &crate::security::SafePath, display_path: &str) -> Response {
    let canonical = path.to_string();
    let read_only = crate::fs_util::directory_is_read_only(std::path::Path::new(&canonical));
    Response::Directory {
        cwd: display_path.to_owned(),
        read_only,
    }
}

/// Clock segment: echoes the shell whose live-time token the render embeds.
#[must_use]
pub const fn clock(shell: crate::shell::Shell) -> Response {
    Response::Clock { shell }
}

/// Duration segment: echoes the elapsed milliseconds.
#[must_use]
pub const fn duration(duration_ms: u64) -> Response {
    Response::Duration { duration_ms }
}

/// Character segment: echoes whether the last command succeeded.
#[must_use]
pub const fn character(success: bool) -> Response {
    Response::Character { success }
}

/// Exit-status segment: echoes whether the last command succeeded.
#[must_use]
pub const fn exit_status(success: bool) -> Response {
    Response::ExitStatus { success }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_duration_request() {
        let response = duration(65_000);
        match response {
            Response::Duration { duration_ms } => assert_eq!(duration_ms, 65_000),
            other => panic!("expected Response::Duration, got {other:?}"),
        }
    }

    #[test]
    fn handles_character_request() {
        let response = character(false);
        match response {
            Response::Character { success } => assert!(!success),
            other => panic!("expected Response::Character, got {other:?}"),
        }
    }

    #[test]
    fn handles_directory_request() {
        // Not "/tmp": it canonicalizes to `\\?\D:\tmp` on Windows.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let requested = temp_dir
            .path()
            .to_str()
            .expect("temp dir path must be valid UTF-8")
            .to_owned();
        let path = crate::security::SafePath::new(&requested).expect("valid path");
        let response = directory(&path, &requested);
        match response {
            Response::Directory { cwd, .. } => {
                assert_eq!(cwd, requested, "expected cwd to echo the requested path");
            }
            other => panic!("expected Response::Directory, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn directory_request_preserves_logical_symlink_path() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let target = temp_dir.path().join("real").join("target");
        std::fs::create_dir_all(&target).expect("create target");
        let link = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let link_str = link.to_str().expect("link path must be valid UTF-8");
        let msg = crate::ipc::protocol::deserialize_message(
            format!(r#"{{"op":"directory","cwd":"{link_str}","format":"json"}}"#).as_bytes(),
        )
        .expect("directory request must parse");
        let response = match msg {
            crate::ipc::Message::DirectoryRequest {
                path, display_path, ..
            } => directory(&path, &display_path),
            other => panic!("expected DirectoryRequest, got {other:?}"),
        };
        match response {
            Response::Directory { cwd, .. } => assert_eq!(
                cwd, link_str,
                "cwd must be the logical (symlink) path, not the resolved target"
            ),
            other => panic!("expected Response::Directory, got {other:?}"),
        }
    }
}
