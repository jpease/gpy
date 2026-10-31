//! What config-changing `gpy` commands say about reloading the agent.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests assert by panicking"
)]

mod common;

use common::agent_sandbox::AgentSandbox;
use common::cli_harness::CliCommandResult;
use common::fixtures::bind_silent_socket;

const NOT_RELOADED_NOTICE: &str = "Agent not reloaded:";

/// Everything the command printed, on either stream.
fn all_output(result: &CliCommandResult) -> String {
    format!("{}\n{}", result.stdout, result.stderr)
}

#[test]
fn mutation_reloads_live_agent() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();

    let switched = sandbox.gpy(&["theme", "use", "starship"]);
    switched.assert_success("gpy theme use starship");
    assert!(
        switched.stdout.contains("Switched to 'starship' theme"),
        "{switched:?}"
    );
    assert!(
        !all_output(&switched).contains("not reloaded"),
        "a live agent must acknowledge the reload: {switched:?}"
    );

    let theme = sandbox.gpy(&["config", "get", "ui.theme"]);
    theme.assert_success("gpy config get ui.theme");
    assert_eq!(theme.stdout.trim(), "starship", "{theme:?}");
}

#[test]
fn mutation_without_agent_prints_no_reload_notice() {
    let sandbox = AgentSandbox::new();

    for args in [
        &["config", "set", "agent.enabled", "false"][..],
        &["enable", "status"][..],
        &["theme", "use", "default", "--force"][..],
    ] {
        let result = sandbox.gpy(args);
        result.assert_success("config-changing command with no agent");
        assert!(
            !all_output(&result).to_ascii_lowercase().contains("reload"),
            "no agent means nothing to reload: {args:?} {result:?}"
        );
    }
}

#[test]
fn mutation_with_an_unanswering_agent_warns_once() {
    let sandbox = AgentSandbox::new();
    let _listener = bind_silent_socket(&sandbox.socket_path);

    for args in [
        &["config", "set", "agent.enabled", "false"][..],
        &["enable", "status"][..],
        &["theme", "use", "default", "--force"][..],
    ] {
        let result = sandbox.gpy(args);
        result.assert_success("config-changing command with an unanswering agent");
        assert_eq!(
            all_output(&result).matches(NOT_RELOADED_NOTICE).count(),
            1_usize,
            "{args:?} {result:?}"
        );
    }
}
