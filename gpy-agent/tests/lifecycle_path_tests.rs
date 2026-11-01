//! `paths::runtime_dir()` precedence: `$XDG_RUNTIME_DIR/gpy`, `$XDG_CACHE_HOME/gpy`,
//! home `.cache/gpy`, `/tmp/gpy`.
//! The home comes from `paths::home_path`, which falls back to passwd when `HOME`
//! is unset (as Fish and Zsh do), so `/tmp/gpy` is reachable only with no home.

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::paths::{SocketPath, runtime_dir};
use serial_test::serial;

const ROOT_VARS: [&str; 3] = ["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"];

#[test]
#[serial]
fn runtime_dir_uses_xdg_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("XDG_RUNTIME_DIR", tmp.path());
    let dir = runtime_dir();
    assert_eq!(dir, tmp.path().join("gpy"));
}

#[test]
#[serial]
fn runtime_dir_uses_xdg_cache_home_when_no_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("XDG_CACHE_HOME", tmp.path());
    let dir = runtime_dir();
    assert_eq!(dir, tmp.path().join("gpy"));
}

#[test]
#[serial]
fn runtime_dir_uses_home_cache_when_no_xdg() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("HOME", tmp.path());
    let dir = runtime_dir();
    assert_eq!(dir, tmp.path().join(".cache").join("gpy"));
}

/// With `HOME` unset, the runtime root is the passwd home, matching the shells.
#[test]
#[serial]
fn runtime_dir_without_home_uses_the_passwd_home() {
    let _env = EnvGuard::cleared(&ROOT_VARS);
    let dir = runtime_dir();
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
    let socket = SocketPath::resolve(None).unwrap();
    assert_eq!(
        socket.path(),
        expected_homeless_runtime_root().join("gpy.sock")
    );
}

/// Runtime root without `HOME`: passwd home's `.cache/gpy`, else `/tmp/gpy`.
fn expected_homeless_runtime_root() -> std::path::PathBuf {
    gpy_agent::paths::home_path().map_or_else(
        || std::path::PathBuf::from("/tmp/gpy"),
        |home| home.join(".cache").join("gpy"),
    )
}
