//! Parallel `gpy config set` writers on distinct keys must all persist: the
//! read-modify-rename in `save_config` is serialized by a lock beside the file.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure"
)]
#![allow(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure"
)]
#![allow(clippy::panic, reason = "test code: a panic is the failure")]
#![allow(clippy::missing_panics_doc, reason = "tests have no callers")]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Distinct keys with an even-round and an odd-round value that differ, so
/// every round writes all of them whatever the previous round left behind.
const KEYS: [(&str, &str, &str); 4] = [
    ("git.stash_enabled", "false", "true"),
    ("ui.show_icons", "false", "true"),
    ("ui.directory.max_length", "40", "41"),
    ("git.max_branch_length", "12", "13"),
];

/// Hold a `gpy config set` back until `go` exists, so the writers start within
/// microseconds of each other. Without this, process start-up staggers them
/// past the roughly 15 ms save window and the race rarely shows.
fn spawn_gated(go: &Path, home: &Path, xdg: &Path, key: &str, value: &str) -> Child {
    Command::new("sh")
        .arg("-c")
        .arg("while [ ! -e \"$1\" ]; do :; done; shift; exec \"$@\"")
        .arg("sh")
        .arg(go)
        .arg(env!("CARGO_BIN_EXE_gpy"))
        .args(["config", "set", key, value])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg)
        .env("XDG_RUNTIME_DIR", home)
        .env("XDG_CACHE_HOME", home)
        .env("GPY_AGENT_SOCKET_PATH", home.join("gpy.sock"))
        .env_remove("GPY_CONFIG_PATH")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gpy")
}

#[test]
fn concurrent_config_sets_on_distinct_keys_all_persist() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let xdg = temp.path().join("xdg");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&xdg).unwrap();
    let config = xdg.join("gpy/config.toml");

    let mut lost = Vec::new();
    for round in 0_u32..12_u32 {
        let go = temp.path().join(format!("go-{round}"));
        let children: Vec<Child> = KEYS
            .iter()
            .map(|(key, even, odd)| {
                let value = if round.is_multiple_of(2) { even } else { odd };
                spawn_gated(&go, &home, &xdg, key, value)
            })
            .collect();
        // Let every child reach its spin loop before releasing them.
        let ready = Instant::now();
        while ready.elapsed() < Duration::from_millis(150) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&go, "").unwrap();
        for mut child in children {
            child.wait().unwrap();
        }

        let text = std::fs::read_to_string(&config).unwrap();
        let root = toml::Value::Table(text.parse().unwrap());
        for (key, even, odd) in KEYS {
            let want = if round.is_multiple_of(2) { even } else { odd };
            let found = key
                .split('.')
                .try_fold(&root, |value, part| value.get(part))
                .map(ToString::to_string);
            if found.as_deref() != Some(want) {
                lost.push(format!("round {round}: {key} is {found:?}, want {want}"));
            }
        }
    }
    assert!(lost.is_empty(), "lost updates:\n{}", lost.join("\n"));
}
