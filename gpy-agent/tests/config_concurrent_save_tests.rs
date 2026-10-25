//! Parallel `gpy config set` writers on distinct keys must all persist: the
//! read-modify-rename in `save_config` is serialized by a lock beside the file
//! (`flock` on Unix, `LockFileEx` on Windows).

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
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Barrier;
use tempfile::TempDir;

/// Distinct keys with an even-round and an odd-round value that differ, so
/// every round writes all of them whatever the previous round left behind.
const KEYS: [(&str, &str, &str); 4] = [
    ("git.stash_enabled", "false", "true"),
    ("ui.show_icons", "false", "true"),
    ("ui.directory.max_length", "40", "41"),
    ("git.max_branch_length", "12", "13"),
];

/// Run one `gpy config set` in the isolated tree and return how it exited.
fn config_set(home: &Path, xdg: &Path, key: &str, value: &str) -> ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["config", "set", key, value])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg)
        .env("XDG_RUNTIME_DIR", home)
        .env("XDG_CACHE_HOME", home)
        .env("GPY_AGENT_SOCKET_PATH", home.join("gpy.sock"))
        .env_remove("GPY_CONFIG_PATH")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
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
        // One thread per writer, released together so the processes start
        // within microseconds of each other. Spawned one after another,
        // process start-up staggers them past the roughly 15 ms save window
        // and the race rarely shows.
        let barrier = Barrier::new(KEYS.len());
        let (gate, home_dir, xdg_dir) = (&barrier, &home, &xdg);
        let statuses: Vec<ExitStatus> = std::thread::scope(|scope| {
            let writers: Vec<_> = KEYS
                .iter()
                .map(|(key, even, odd)| {
                    let value = if round.is_multiple_of(2) { even } else { odd };
                    scope.spawn(move || {
                        gate.wait();
                        config_set(home_dir, xdg_dir, key, value)
                    })
                })
                .collect();
            writers
                .into_iter()
                .map(|writer| writer.join().expect("writer thread"))
                .collect()
        });
        for ((key, _, _), status) in KEYS.iter().zip(&statuses) {
            if !status.success() {
                lost.push(format!("round {round}: setting {key} exited {status}"));
            }
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
