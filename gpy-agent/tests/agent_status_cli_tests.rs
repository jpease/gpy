//! `gpy-agent status` report and `start`/`stop` outcome lines against a real
//! daemon on a private socket; Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::panic, reason = "test code may panic on failure")]

mod common;

use common::agent_sandbox::AgentSandbox;
use common::fake_agent::{Behaviour, FakeAgent};
use common::fixtures::bind_silent_socket;
use std::time::Duration;

/// The value of `key: value` in `report`, from the first line with that key.
fn value_of<'a>(report: &'a str, key: &str) -> Option<&'a str> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
}

/// Remove a socket file a fixture left behind.
fn remove_socket(sandbox: &AgentSandbox) {
    let _ = std::fs::remove_file(&sandbox.socket_path);
}

#[test]
fn status_reports_key_value_lines_in_order() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();

    let status = sandbox.gpy_agent(&["status"]);

    assert_eq!(status.exit_code, 0_i32, "{status:?}");
    let keys: Vec<&str> = status
        .stdout
        .lines()
        .filter_map(|line| line.split(": ").next())
        .collect();
    assert_eq!(
        keys,
        [
            "state",
            "pid",
            "version",
            "protocol",
            "socket",
            "config",
            "watched_repos",
            "registered_clients",
            "cache_entries"
        ],
        "{status:?}"
    );
    assert_eq!(value_of(&status.stdout, "state"), Some("running"));
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    assert_eq!(
        value_of(&status.stdout, "socket"),
        Some(socket.as_str()),
        "{status:?}"
    );
}

#[test]
fn status_state_agrees_with_the_exit_code() {
    let sandbox = AgentSandbox::new();

    let absent = sandbox.gpy_agent(&["status"]);
    assert_eq!(value_of(&absent.stdout, "state"), Some("not-running"));
    assert_eq!(absent.exit_code, 3_i32, "{absent:?}");
    assert_eq!(
        absent.stdout.lines().count(),
        3,
        "a missing socket reports only state, socket and config: {absent:?}"
    );

    drop(bind_silent_socket(&sandbox.socket_path));
    let refused = sandbox.gpy_agent(&["status"]);
    assert_eq!(value_of(&refused.stdout, "state"), Some("not-running"));
    assert_eq!(refused.exit_code, 3_i32, "{refused:?}");
    assert!(
        value_of(&refused.stdout, "error").is_some(),
        "a refusing socket file names the refusal: {refused:?}"
    );
    remove_socket(&sandbox);

    {
        let _busy =
            FakeAgent::spawn_with(&sandbox.socket_path, Behaviour::DropsUnread, Duration::ZERO);
        let busy = sandbox.gpy_agent(&["status"]);
        assert_eq!(value_of(&busy.stdout, "state"), Some("busy"), "{busy:?}");
        assert_eq!(busy.exit_code, 0_i32, "{busy:?}");
        assert_eq!(
            value_of(&busy.stdout, "error"),
            Some("agent closed the connection without a reply"),
            "{busy:?}"
        );
        assert!(
            value_of(&busy.stdout, "pid").is_none(),
            "a busy agent sent no counters: {busy:?}"
        );
    }
    remove_socket(&sandbox);

    {
        let _listener = bind_silent_socket(&sandbox.socket_path);
        let wedged = sandbox.gpy_agent(&["status"]);
        assert_eq!(
            value_of(&wedged.stdout, "state"),
            Some("not-answering"),
            "{wedged:?}"
        );
        assert_eq!(wedged.exit_code, 4_i32, "{wedged:?}");
        assert!(
            value_of(&wedged.stdout, "error").is_some(),
            "a wedged agent names the failure: {wedged:?}"
        );
    }
}

#[test]
fn start_and_status_report_the_same_pid() {
    let sandbox = AgentSandbox::new();

    let started = sandbox.gpy_agent(&["start"]);
    assert_eq!(started.exit_code, 0_i32, "{started:?}");
    let started_pid = started
        .stdout
        .trim_end()
        .strip_prefix("Started agent (pid ")
        .and_then(|rest| rest.strip_suffix(")."))
        .unwrap_or_else(|| panic!("start prints its outcome line: {started:?}"))
        .to_owned();
    sandbox.wait_until_responding();

    let status = sandbox.gpy_agent(&["status"]);
    assert_eq!(
        value_of(&status.stdout, "pid"),
        Some(started_pid.as_str()),
        "{status:?}"
    );
    let alive = std::process::Command::new("kill")
        .args(["-0", &started_pid])
        .status()
        .expect("spawn kill");
    assert!(alive.success(), "pid {started_pid} is a live process");
}

#[test]
fn stop_prints_one_line_naming_the_pid() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();
    let status = sandbox.gpy_agent(&["status"]);
    let pid = value_of(&status.stdout, "pid").expect("a live agent reports its pid");

    let stopped = sandbox.gpy_agent(&["stop"]);

    assert_eq!(stopped.exit_code, 0_i32, "{stopped:?}");
    assert_eq!(
        stopped.stdout,
        format!("Stopped agent (pid {pid}).\n"),
        "{stopped:?}"
    );
    assert!(stopped.stderr.is_empty(), "{stopped:?}");
}

#[test]
fn stop_names_the_removed_stale_socket() {
    let sandbox = AgentSandbox::new();
    drop(bind_silent_socket(&sandbox.socket_path));

    let stopped = sandbox.gpy_agent(&["stop"]);

    assert_eq!(stopped.exit_code, 0_i32, "{stopped:?}");
    assert_eq!(
        stopped.stdout,
        format!(
            "Agent not running; removed stale socket {}.\n",
            sandbox.socket_path.display()
        ),
        "{stopped:?}"
    );
}

#[test]
fn quiet_start_and_stop_print_nothing() {
    let sandbox = AgentSandbox::new();

    let started = sandbox.gpy_agent(&["start", "--quiet"]);
    assert_eq!(started.exit_code, 0_i32, "{started:?}");
    assert!(
        started.stdout.is_empty() && started.stderr.is_empty(),
        "{started:?}"
    );
    sandbox.wait_until_responding();

    let stopped = sandbox.gpy_agent(&["stop", "--quiet"]);
    assert_eq!(stopped.exit_code, 0_i32, "{stopped:?}");
    assert!(
        stopped.stdout.is_empty() && stopped.stderr.is_empty(),
        "{stopped:?}"
    );
}
