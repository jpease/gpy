//! `gpy restart` and `gpy-agent restart` against a real daemon on a private
//! socket; Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::panic, reason = "test code may panic on failure")]

mod common;

use common::agent_sandbox::AgentSandbox;
use common::fixtures::bind_silent_socket;

/// The PID in an outcome line such as `Started agent (pid 42).`.
fn pid_in(line: &str, prefix: &str) -> String {
    line.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(")."))
        .unwrap_or_else(|| panic!("`{line}` does not start with `{prefix}`"))
        .to_owned()
}

/// The `pid:` value of `gpy-agent status`.
fn status_pid(sandbox: &AgentSandbox) -> String {
    let status = sandbox.gpy_agent(&["status"]);
    status
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("pid: "))
        .unwrap_or_else(|| panic!("status reports a pid: {status:?}"))
        .to_owned()
}

#[test]
fn restart_replaces_the_running_agent() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();
    let before = status_pid(&sandbox);

    let restarted = sandbox.gpy(&["restart"]);

    assert_eq!(restarted.exit_code, 0_i32, "{restarted:?}");
    let lines: Vec<&str> = restarted.stdout.lines().collect();
    let [stopped_line, started_line] = lines.as_slice() else {
        panic!("restart prints a stop line and a start line: {restarted:?}");
    };
    let stopped_pid = pid_in(stopped_line, "Stopped agent (pid ");
    let started_pid = pid_in(started_line, "Started agent (pid ");
    assert_eq!(stopped_pid, before, "{restarted:?}");
    assert_ne!(started_pid, stopped_pid, "{restarted:?}");
    sandbox.wait_until_responding();
    assert_eq!(status_pid(&sandbox), started_pid, "{restarted:?}");
}

#[test]
fn restart_without_an_agent_starts_one() {
    let sandbox = AgentSandbox::new();

    let restarted = sandbox.gpy_agent(&["restart"]);

    assert_eq!(restarted.exit_code, 0_i32, "{restarted:?}");
    let lines: Vec<&str> = restarted.stdout.lines().collect();
    let [stop_line, start_line] = lines.as_slice() else {
        panic!("restart prints a stop line and a start line: {restarted:?}");
    };
    assert_eq!(*stop_line, "Agent not running.");
    let started_pid = pid_in(start_line, "Started agent (pid ");
    sandbox.wait_until_responding();
    assert_eq!(status_pid(&sandbox), started_pid, "{restarted:?}");
}

#[test]
fn restart_with_agent_disabled_exits_6() {
    let sandbox = AgentSandbox::new();
    sandbox
        .env
        .write_config("[agent]\nenabled = false\n")
        .expect("write config");

    for restarted in [sandbox.gpy_agent(&["restart"]), sandbox.gpy(&["restart"])] {
        assert_eq!(restarted.exit_code, 6_i32, "{restarted:?}");
        assert!(
            !restarted.stderr.contains("Error:"),
            "a disabled agent is not an error: {restarted:?}"
        );
    }
    assert!(!sandbox.socket_path.exists(), "nothing was bound");
}

#[test]
fn restart_recovers_from_an_unanswering_socket() {
    let sandbox = AgentSandbox::new();
    let _listener = bind_silent_socket(&sandbox.socket_path);

    let restarted = sandbox.gpy_agent(&["restart"]);

    assert_eq!(restarted.exit_code, 0_i32, "{restarted:?}");
    assert_eq!(
        restarted
            .stderr
            .lines()
            .filter(|line| line.starts_with("warn:"))
            .count(),
        1,
        "the failed stop is one warn: line: {restarted:?}"
    );
    sandbox.wait_until_responding();
}
