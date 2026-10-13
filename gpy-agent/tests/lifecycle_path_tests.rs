//! Runtime/socket path resolution in the lifecycle helpers.
//!
//! `get_runtime_dir()` follows the canonical 4-step precedence:
//!   1. `$XDG_RUNTIME_DIR/gpy`
//!   2. `$XDG_CACHE_HOME/gpy`
//!   3. the home directory's `.cache/gpy`
//!   4. `/tmp/gpy`
//!
//! Step 3 is the *home directory*, not the `HOME` variable: it comes from
//! `paths::home_dir`, which falls back to the passwd database when the
//! environment does not carry `HOME`, exactly as Fish and Zsh do. Step 4 is
//! therefore reached only when the user has no home directory at all -- a
//! state a test process on a machine with a user account cannot produce, so
//! the homeless tests assert the passwd home and fall back to asserting
//! `/tmp/gpy` only if this machine really has no home.
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::agent::lifecycle::{SOCKET_OVERRIDE, get_runtime_dir, get_socket_path};
use serial_test::serial;

const ROOT_VARS: [&str; 3] = ["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"];

#[test]
#[serial]
fn runtime_dir_uses_xdg_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("XDG_RUNTIME_DIR", tmp.path());
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, tmp.path().join("gpy"));
}

#[test]
#[serial]
fn runtime_dir_uses_xdg_cache_home_when_no_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("XDG_CACHE_HOME", tmp.path());
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, tmp.path().join("gpy"));
}

#[test]
#[serial]
fn runtime_dir_uses_home_cache_when_no_xdg() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("HOME", tmp.path());
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, tmp.path().join(".cache").join("gpy"));
}

/// With `HOME` absent the runtime root is the passwd home, which is what the
/// shells resolve. `/tmp/gpy` applies only when no home directory exists at
/// all.
#[test]
#[serial]
fn runtime_dir_without_home_uses_the_passwd_home() {
    let _env = EnvGuard::cleared(&ROOT_VARS);
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, expected_homeless_runtime_root());
}

#[test]
#[serial]
fn socket_path_without_home_uses_the_passwd_home() {
    let _env = EnvGuard::cleared(&[
        "XDG_RUNTIME_DIR",
        "XDG_CACHE_HOME",
        "HOME",
        "GPY_AGENT_SOCKET_PATH",
    ]);
    if SOCKET_OVERRIDE.get().is_some() {
        return; // SOCKET_OVERRIDE takes precedence; skip
    }
    let path = get_socket_path().unwrap();
    assert_eq!(path, expected_homeless_runtime_root().join("gpy.sock"));
}

/// The runtime root for an environment carrying no `HOME`: the passwd home's
/// `.cache/gpy`, or `/tmp/gpy` on the (practically unreachable) machine where
/// even the passwd database has no home for this user.
fn expected_homeless_runtime_root() -> std::path::PathBuf {
    gpy_agent::paths::home_dir().map_or_else(
        || std::path::PathBuf::from("/tmp/gpy"),
        |home| std::path::PathBuf::from(home).join(".cache").join("gpy"),
    )
}
