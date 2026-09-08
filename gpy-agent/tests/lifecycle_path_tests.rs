// Regression tests for runtime/socket path resolution in lifecycle helpers.
// Verifies that get_runtime_dir() follows the canonical 4-step precedence:
//   1. $XDG_RUNTIME_DIR/gpy
//   2. $XDG_CACHE_HOME/gpy
//   3. the home directory's .cache/gpy
//   4. /tmp/gpy
//
// Step 3 is the *home directory*, not the `HOME` variable (#626): it comes
// from `paths::home_dir`, which falls back to the passwd database when the
// environment does not carry `HOME`, exactly as Fish and Zsh do. Step 4 is
// therefore reached only when the user has no home directory at all -- a
// state a test process on a machine with a user account cannot produce, so
// the two tests below assert the passwd home and fall back to asserting
// /tmp/gpy only if this machine really has no home.
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::agent::lifecycle::{SOCKET_OVERRIDE, get_runtime_dir, get_socket_path};
use std::ffi::OsString;

struct EnvGuard {
    keys: Vec<&'static str>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let saved = keys
            .iter()
            .map(|&k| (k, std::env::var_os(k)))
            .collect::<Vec<_>>();
        for &k in keys {
            unsafe { std::env::remove_var(k) };
        }
        Self {
            keys: keys.to_vec(),
            saved,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for &k in &self.keys {
            unsafe { std::env::remove_var(k) };
        }
        for (k, v) in &self.saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }
}

#[test]
fn runtime_dir_uses_xdg_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"]);
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", tmp.path()) };
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, tmp.path().join("gpy"));
}

#[test]
fn runtime_dir_uses_xdg_cache_home_when_no_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"]);
    unsafe { std::env::set_var("XDG_CACHE_HOME", tmp.path()) };
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, tmp.path().join("gpy"));
}

#[test]
fn runtime_dir_uses_home_cache_when_no_xdg() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"]);
    unsafe { std::env::set_var("HOME", tmp.path()) };
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, tmp.path().join(".cache").join("gpy"));
}

/// `HOME` absent used to mean `/tmp/gpy`; since #626 it means the passwd
/// home, which is what the shells have always resolved. `/tmp/gpy` survives
/// only for "no home directory exists at all".
#[test]
fn runtime_dir_without_home_uses_the_passwd_home() {
    let _guard = EnvGuard::new(&["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"]);
    let dir = get_runtime_dir().unwrap();
    assert_eq!(dir, expected_homeless_runtime_root());
}

#[test]
fn socket_path_without_home_uses_the_passwd_home() {
    let _guard = EnvGuard::new(&[
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
