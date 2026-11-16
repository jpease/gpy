//! `paths::runtime_dir()` precedence: `$XDG_RUNTIME_DIR/gpy`, `$XDG_CACHE_HOME/gpy`,
//! then home `.cache/gpy`; with none usable it fails closed (no `/tmp/gpy`).
//! The home comes from `paths::home_path`, which falls back to passwd when `HOME`
//! is unset (as Fish and Zsh do); a relative `HOME` counts as no home at all.

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::paths::{NO_RUNTIME_DIR, SocketPath, runtime_dir};
use serial_test::serial;

const ROOT_VARS: [&str; 4] = [
    "XDG_RUNTIME_DIR",
    "XDG_CACHE_HOME",
    "HOME",
    "GPY_AGENT_SOCKET_PATH",
];

#[test]
#[serial]
fn runtime_dir_uses_xdg_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("XDG_RUNTIME_DIR", tmp.path());
    assert_eq!(runtime_dir().unwrap(), tmp.path().join("gpy"));
}

#[test]
#[serial]
fn runtime_dir_uses_xdg_cache_home_when_no_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("XDG_CACHE_HOME", tmp.path());
    assert_eq!(runtime_dir().unwrap(), tmp.path().join("gpy"));
}

#[test]
#[serial]
fn runtime_dir_uses_home_cache_when_no_xdg() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("HOME", tmp.path());
    assert_eq!(
        runtime_dir().unwrap(),
        tmp.path().join(".cache").join("gpy")
    );
}

/// With `HOME` unset, the runtime root is the passwd home, matching the shells.
#[test]
#[serial]
fn runtime_dir_without_home_uses_the_passwd_home() {
    let _env = EnvGuard::cleared(&ROOT_VARS);
    let Some(home) = gpy_agent::paths::home_path() else {
        // No passwd home either (a stripped container): nothing is usable.
        assert!(runtime_dir().is_err());
        return;
    };
    let expected = home.join(".cache").join("gpy");
    assert_eq!(runtime_dir().unwrap(), expected);
    assert_eq!(
        SocketPath::resolve(None).unwrap().path(),
        expected.join("gpy.sock")
    );
}

/// No usable variable: no runtime root, and the default socket fails with
/// the one-line message naming what to set, never a `/tmp/gpy` fallback.
#[test]
#[serial]
fn nothing_usable_fails_closed() {
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("HOME", "relative/home");
    env.set("XDG_RUNTIME_DIR", "run");
    env.set("XDG_CACHE_HOME", "");

    let runtime_error = runtime_dir().expect_err("no runtime root").to_string();
    assert!(runtime_error.contains(NO_RUNTIME_DIR), "{runtime_error}");
    let socket_error = SocketPath::resolve(None)
        .expect_err("no default socket")
        .to_string();
    assert!(socket_error.contains(NO_RUNTIME_DIR), "{socket_error}");
}

/// An explicit socket needs no runtime root.
#[test]
#[serial]
fn a_socket_override_resolves_without_a_runtime_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::cleared(&ROOT_VARS);
    env.set("HOME", "relative/home");
    let custom = tmp.path().join("custom.sock");
    env.set("GPY_AGENT_SOCKET_PATH", &custom);
    assert_eq!(SocketPath::resolve(None).unwrap().path(), custom);
}
