//! Exit codes of `gpy-agent`/`gpy` start, stop and status (the LSB codes)
//! against a real daemon on a private socket; Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::panic, reason = "test code may panic on failure")]

mod common;

use common::agent_sandbox::{AgentSandbox, DaemonReaper, RUNNING_LINE, pids_matching};
use common::fake_agent::{Behaviour, FakeAgent};
use common::fixtures::bind_silent_socket;
use std::time::Duration;

/// A config with the agent switched off.
const DISABLED_CONFIG: &str = "[agent]\nenabled = false\n";

/// Remove a socket file a fixture left behind.
fn remove_socket(sandbox: &AgentSandbox) {
    let _ = std::fs::remove_file(&sandbox.socket_path);
}

#[test]
fn agent_status_exit_code_follows_lsb() {
    let sandbox = AgentSandbox::new();

    // No socket at all.
    let absent = sandbox.gpy_agent(&["status"]);
    assert_eq!(absent.exit_code, 3_i32, "no socket must exit 3: {absent:?}");
    assert!(
        absent.stderr.is_empty(),
        "no Error: line for a finding: {absent:?}"
    );

    // A socket file nothing answers on: the connection is accepted into the
    // backlog and never answered.
    {
        let _listener = bind_silent_socket(&sandbox.socket_path);
        let wedged = sandbox.gpy_agent(&["status"]);
        assert_eq!(
            wedged.exit_code, 4_i32,
            "an unanswered socket must exit 4: {wedged:?}"
        );
    }
    remove_socket(&sandbox);

    // A socket file nothing listens on: the connection is refused.
    drop(bind_silent_socket(&sandbox.socket_path));
    let refused = sandbox.gpy_agent(&["status"]);
    assert_eq!(
        refused.exit_code, 3_i32,
        "a refusing socket must exit 3: {refused:?}"
    );
    remove_socket(&sandbox);

    // A busy agent accepts and drops every connection unanswered.
    {
        let _busy =
            FakeAgent::spawn_with(&sandbox.socket_path, Behaviour::DropsUnread, Duration::ZERO);
        let busy = sandbox.gpy_agent(&["status"]);
        assert_eq!(busy.exit_code, 0_i32, "a busy agent must exit 0: {busy:?}");
    }
    remove_socket(&sandbox);

    // A live agent.
    sandbox.start_and_wait();
    let live = sandbox.gpy_agent(&["status"]);
    assert_eq!(live.exit_code, 0_i32, "live agent must exit 0: {live:?}");
    assert!(live.stdout.contains(RUNNING_LINE), "{live:?}");
    assert!(
        live.stdout.contains("version: "),
        "the live report carries the agent version: {live:?}"
    );
}

#[test]
fn start_with_agent_disabled_exits_6_and_binds_nothing() {
    let sandbox = AgentSandbox::new();
    sandbox
        .env
        .write_config(DISABLED_CONFIG)
        .expect("write config");

    for started in [sandbox.gpy_agent(&["start"]), sandbox.gpy(&["start"])] {
        assert_eq!(started.exit_code, 6_i32, "{started:?}");
        assert!(
            !started.stderr.contains("Error:"),
            "a disabled agent is not an error: {started:?}"
        );
        assert_eq!(
            started.stderr.matches("Agent disabled in config").count(),
            1,
            "the disabled line is printed once: {started:?}"
        );
        assert!(
            !sandbox.socket_path.exists(),
            "a disabled agent binds no socket"
        );
    }
}

#[test]
fn start_with_an_agent_already_running_exits_0() {
    let sandbox = AgentSandbox::new();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let _reaper = DaemonReaper(sandbox.socket_path.clone());

    // `--socket` puts the path in the daemon's argv, which `pids_matching` greps.
    let first = sandbox.gpy_agent(&["start", "--socket", &socket]);
    assert_eq!(first.exit_code, 0_i32, "{first:?}");
    sandbox.wait_until_responding();
    let second = sandbox.gpy_agent(&["start", "--socket", &socket]);
    assert_eq!(second.exit_code, 0_i32, "{second:?}");

    assert_eq!(
        pids_matching(&sandbox.socket_path).len(),
        1,
        "the second start forks nothing"
    );
}

#[test]
fn stop_exits_0_unless_the_agent_cannot_be_stopped() {
    let sandbox = AgentSandbox::new();

    // No agent.
    let none = sandbox.gpy_agent(&["stop"]);
    assert_eq!(none.exit_code, 0_i32, "{none:?}");
    assert_eq!(none.stdout.trim_end(), "Agent not running.", "{none:?}");

    // A stale socket file.
    drop(bind_silent_socket(&sandbox.socket_path));
    let stale = sandbox.gpy_agent(&["stop"]);
    assert_eq!(stale.exit_code, 0_i32, "{stale:?}");
    assert!(!sandbox.socket_path.exists(), "the stale socket is removed");

    // A live agent, even when the config now disables it.
    sandbox.start_and_wait();
    sandbox
        .env
        .write_config(DISABLED_CONFIG)
        .expect("write config");
    let live = sandbox.gpy_agent(&["stop"]);
    assert_eq!(live.exit_code, 0_i32, "stop never returns 6: {live:?}");
    assert!(sandbox.wait_until_socket_gone(), "{live:?}");
}

#[test]
fn oneshot_failure_prints_one_error_line_and_exits_1() {
    let sandbox = AgentSandbox::new();
    // A user `default.toml` overrides the builtin one, so both the configured
    // theme and the fallback fail to load.
    std::fs::create_dir_all(sandbox.env.themes_dir()).expect("create themes dir");
    std::fs::write(
        sandbox.env.themes_dir().join("default.toml"),
        "not = [valid",
    )
    .expect("write malformed theme");

    let result = sandbox.gpy_agent(&[
        "oneshot",
        "character",
        "--exit-code",
        "0",
        "--format",
        "ansi",
    ]);

    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(result.stdout.is_empty(), "{result:?}");
    assert_eq!(
        result
            .stderr
            .lines()
            .filter(|line| line.starts_with("Error: "))
            .count(),
        1,
        "exactly one Error: line: {result:?}"
    );
    assert!(
        !result.stderr.lines().any(|line| line.starts_with('{')),
        "no JSON error object on stderr: {result:?}"
    );
}
