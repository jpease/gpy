//! `gpy-agent start`, `status`, `theme export` and `config reload` against fake
//! and real agents. Socket-dependent, so Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::panic, reason = "test code may panic on failure")]

mod common;

use common::agent_sandbox::{AgentSandbox, RUNNING_LINE};
use common::fake_agent::{Behaviour, FakeAgent};
use std::time::Duration;

/// A config whose `agent.enabled = false` sits beside a key that fails validation.
const DISABLED_IN_INVALID_CONFIG: &str = "[agent]\nenabled = false\n[ui]\ntheme = 42\n";

#[test]
fn status_reports_a_busy_agent_as_busy() {
    let sandbox = AgentSandbox::new();
    let _agent =
        FakeAgent::spawn_with(&sandbox.socket_path, Behaviour::DropsUnread, Duration::ZERO);

    let status = sandbox.gpy_agent(&["status"]);

    assert!(status.stdout.contains("state: busy"), "{status:?}");
    assert_eq!(
        status.exit_code, 0_i32,
        "status exits 0 for a busy agent: {status:?}"
    );
}

#[test]
fn start_honors_agent_disabled_in_an_otherwise_invalid_config() {
    let sandbox = AgentSandbox::new();
    sandbox
        .env
        .write_config(DISABLED_IN_INVALID_CONFIG)
        .expect("write config");

    let started = sandbox.gpy_agent(&["start"]);

    assert_eq!(started.exit_code, 6_i32, "{started:?}");
    assert!(
        started.stderr.contains("Agent disabled in config"),
        "the disabled line must be printed: {started:?}"
    );
    assert!(
        !sandbox.socket_path.exists(),
        "no daemon may bind a socket when the agent is disabled"
    );
}

#[test]
fn theme_export_keeps_agent_disabled_from_an_invalid_config() {
    let sandbox = AgentSandbox::new();
    sandbox
        .env
        .write_config(DISABLED_IN_INVALID_CONFIG)
        .expect("write config");

    let exported = sandbox.gpy_agent(&["theme", "export", "--shell", "bash"]);

    assert_eq!(exported.exit_code, 0_i32, "{exported:?}");
    let line = exported
        .stdout
        .lines()
        .find(|line| line.contains("GPY_AGENT_ENABLED"))
        .unwrap_or_else(|| panic!("GPY_AGENT_ENABLED must be exported: {exported:?}"));
    assert!(
        line.contains('0') && !line.contains('1'),
        "GPY_AGENT_ENABLED must be 0: {line}"
    );
}

#[test]
fn start_prints_one_progress_line_and_leaves_an_answering_daemon() {
    let sandbox = AgentSandbox::new();

    let started = sandbox.gpy_agent(&["start"]);

    assert_eq!(started.exit_code, 0_i32, "{started:?}");
    assert_eq!(
        started
            .stdout
            .lines()
            .filter(|line| line.starts_with("Started agent (pid "))
            .count(),
        1,
        "start prints its outcome once on stdout, never from the daemon child: {started:?}"
    );
    assert!(
        started.stderr.is_empty(),
        "start prints no progress on stderr: {started:?}"
    );
    sandbox.wait_until_responding();
    let status = sandbox.gpy_agent(&["status"]);
    assert!(status.stdout.contains(RUNNING_LINE), "{status:?}");

    sandbox
        .gpy_agent(&["stop"])
        .assert_success("gpy-agent stop");
    assert!(
        sandbox.wait_until_socket_gone(),
        "the stopped daemon must remove its socket"
    );
}

/// `config reload --socket` reaches the agent on that socket even when
/// `GPY_AGENT_SOCKET_PATH` points elsewhere: the flag beats the environment.
#[test]
fn config_reload_honors_the_socket_flag() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let elsewhere = sandbox.socket_path.with_file_name("elsewhere.sock");

    let reloaded = sandbox
        .env
        .run_gpy_agent_with_env(
            &["config", "reload", "--socket", &socket],
            &[("GPY_AGENT_SOCKET_PATH", elsewhere.as_os_str())],
        )
        .expect("spawn gpy-agent config reload");

    reloaded.assert_success("gpy-agent config reload --socket");
    assert!(
        reloaded.stdout.contains("reloaded its configuration"),
        "the agent on the flag's socket must have reloaded: {reloaded:?}"
    );
}

/// Nothing listens on the `--socket` path, with no `GPY_AGENT_SOCKET_PATH` set:
/// the reload reports "not running" rather than failing.
#[test]
fn config_reload_reports_not_running_when_the_flag_socket_is_empty() {
    let sandbox = AgentSandbox::new();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();

    let reloaded = sandbox
        .env
        .run_gpy_agent_with_env(
            &["config", "reload", "--socket", &socket],
            &[("GPY_AGENT_SOCKET_PATH", "")],
        )
        .expect("spawn gpy-agent config reload");

    reloaded.assert_success("gpy-agent config reload --socket with no agent");
    assert!(
        reloaded.stdout.contains("Agent is not running"),
        "{reloaded:?}"
    );
}
