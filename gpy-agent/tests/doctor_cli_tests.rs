//! `gpy doctor` verdicts: agent state, IPC client, versions; Unix-only.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests assert by panicking"
)]

mod common;

use common::agent_sandbox::AgentSandbox;
use common::cli_harness::{CliCommandResult, lone_gpy};
use common::fake_agent::{Behaviour, FakeAgent};
use common::fixtures::bind_silent_socket;
use std::path::{Path, PathBuf};
use std::time::Duration;

const ALL_PASSED: &str = "ok: All checks passed.";

/// `gpy doctor` from a `gpy` with no sibling `gpy-agent`, PATH holding just
/// `bin_dir`, and `config` as the user config.
fn doctor_with_agent_path(
    sandbox: &AgentSandbox,
    bin_dir: &Path,
    config: &str,
) -> CliCommandResult {
    sandbox.env.write_config(config).expect("write config");
    sandbox
        .env
        .run_program_with_env(&lone_gpy(&sandbox.env), &["doctor"], &[("PATH", bin_dir)])
        .expect("run gpy doctor")
}

fn make_executable(path: &Path) {
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

/// A bin dir holding a copy of `gpy-agent` and an `nc` that prints `nc_help`.
fn ipc_bin_dir(sandbox: &AgentSandbox, nc_help: &str) -> PathBuf {
    let bin_dir = sandbox.env.root().join("ipc-bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_gpy-agent"), bin_dir.join("gpy-agent")).unwrap();
    let nc = bin_dir.join("nc");
    std::fs::write(&nc, format!("#!/bin/sh\necho '{nc_help}' >&2\nexit 1\n")).unwrap();
    make_executable(&nc);
    bin_dir
}

/// Install the real `shell` integration into the sandbox's HOME.
fn install_integration(sandbox: &AgentSandbox, shell: &str) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate lives inside the repository");
    let from = repository.to_string_lossy().into_owned();
    sandbox
        .gpy(&[
            "setup",
            "--shell",
            shell,
            "--non-interactive",
            "--from",
            &from,
        ])
        .assert_success("gpy setup");
}

#[test]
fn doctor_reports_not_running_when_no_agent() {
    let sandbox = AgentSandbox::new();

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(
        doctor.exit_code, 0_i32,
        "a not-running agent only warns: {doctor:?}"
    );
    assert!(
        doctor
            .stdout
            .contains("Checking process... warn: Not running"),
        "{doctor:?}"
    );
    assert!(
        doctor
            .stdout
            .contains("Hint: Start the agent with `gpy start`"),
        "the hint must name the real command: {doctor:?}"
    );
    assert!(
        !doctor.stdout.contains("gpy agent start"),
        "no nonexistent `gpy agent start` command in the hint: {doctor:?}"
    );
    assert!(
        !doctor.stdout.contains("Checking process... ok: Running"),
        "{doctor:?}"
    );
    assert!(doctor.stdout.contains(ALL_PASSED), "{doctor:?}");
}

#[test]
fn doctor_accepts_agent_disabled_via_config() {
    let sandbox = AgentSandbox::new();
    sandbox
        .env
        .write_config("[agent]\nenabled = false\n")
        .expect("write config");

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(
        doctor.exit_code, 0_i32,
        "a disabled agent is not a health failure: {doctor:?}"
    );
    assert!(
        doctor
            .stdout
            .contains("Checking process... Disabled via config (agent.enabled = false)"),
        "{doctor:?}"
    );
    assert!(!doctor.stdout.contains("Not running"), "{doctor:?}");
    assert!(
        doctor
            .stdout
            .contains("Checking gpy-agent binary... ok: Found ("),
        "the working gpy-agent must pass the binary check: {doctor:?}"
    );
    let sibling = Path::new(env!("CARGO_BIN_EXE_gpy")).with_file_name("gpy-agent");
    assert!(
        doctor
            .stdout
            .contains(&format!("Found ({})", sibling.display())),
        "{doctor:?}"
    );
}

#[test]
fn doctor_fails_a_gpy_agent_whose_version_probe_fails() {
    let sandbox = AgentSandbox::new();
    let disabled = "[agent]\nenabled = false\n";
    let bin_dir = sandbox.env.root().join("broken-bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let agent = bin_dir.join("gpy-agent");
    std::fs::write(&agent, "#!/bin/sh\necho 'loader failed' >&2\nexit 42\n").unwrap();
    make_executable(&agent);

    let broken = doctor_with_agent_path(&sandbox, &bin_dir, disabled);
    assert_eq!(broken.exit_code, 1_i32, "{broken:?}");
    let binary_report = broken
        .stdout
        .split_once("Checking gpy-agent binary...")
        .map(|(_, report)| report)
        .expect("doctor ran the binary check");
    assert!(binary_report.contains("42"), "exit status: {broken:?}");
    assert!(
        binary_report.contains("loader failed"),
        "stderr: {broken:?}"
    );

    let empty_dir = sandbox.env.root().join("empty-bin");
    std::fs::create_dir_all(&empty_dir).unwrap();
    let missing = doctor_with_agent_path(&sandbox, &empty_dir, disabled);
    assert_eq!(missing.exit_code, 1_i32, "{missing:?}");
    assert_ne!(
        missing
            .stdout
            .lines()
            .find(|line| line.contains("gpy-agent binary")),
        broken
            .stdout
            .lines()
            .find(|line| line.contains("gpy-agent binary")),
        "a failing binary must be reported differently from a missing one"
    );
}

#[test]
fn doctor_fails_when_the_socket_does_not_answer() {
    let sandbox = AgentSandbox::new();
    let _listener = bind_silent_socket(&sandbox.socket_path);

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(doctor.exit_code, 1_i32, "{doctor:?}");
    assert!(
        doctor.stdout.contains(&format!(
            "FAIL: Not answering on {}",
            sandbox.socket_path.display()
        )),
        "{doctor:?}"
    );
    assert!(doctor.stdout.contains("gpy restart"), "{doctor:?}");
    assert!(
        doctor.stdout.contains("FAIL: Issues were found."),
        "{doctor:?}"
    );
}

#[test]
fn doctor_warns_on_a_refusing_socket() {
    let sandbox = AgentSandbox::new();
    drop(bind_silent_socket(&sandbox.socket_path));
    assert!(sandbox.socket_path.exists(), "the stale socket file stays");

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    assert!(
        doctor
            .stdout
            .contains("Checking process... warn: Not running"),
        "{doctor:?}"
    );
    assert!(doctor.stdout.contains(ALL_PASSED), "{doctor:?}");
}

#[test]
fn doctor_warns_on_a_busy_agent() {
    let sandbox = AgentSandbox::new();
    let _agent =
        FakeAgent::spawn_with(&sandbox.socket_path, Behaviour::DropsUnread, Duration::ZERO);

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    assert!(
        doctor
            .stdout
            .lines()
            .any(|line| line.contains("warn:") && line.contains("too busy")),
        "{doctor:?}"
    );
    assert!(doctor.stdout.contains(ALL_PASSED), "{doctor:?}");
}

#[test]
fn doctor_reports_running_with_a_live_agent() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();

    let doctor = sandbox.gpy(&["doctor"]);
    doctor.assert_success("gpy doctor with a live agent");
    assert!(
        doctor.stdout.contains("Checking process... ok: Running"),
        "{doctor:?}"
    );
    assert!(doctor.stdout.contains(ALL_PASSED), "{doctor:?}");
}

#[test]
fn doctor_fails_on_an_overlong_socket_path() {
    let sandbox = AgentSandbox::new();
    let long = format!("/tmp/{}", "a".repeat(115));
    assert_eq!(long.len(), 120_usize);

    let doctor = sandbox
        .env
        .run_gpy_with_env(&["doctor"], &[("GPY_AGENT_SOCKET_PATH", long.as_str())])
        .expect("run gpy doctor");
    assert_eq!(doctor.exit_code, 1_i32, "{doctor:?}");
    assert!(
        doctor.stdout.contains("Cannot resolve the agent socket"),
        "{doctor:?}"
    );
}

#[test]
fn doctor_warns_without_an_ipc_client() {
    let sandbox = AgentSandbox::new();
    install_integration(&sandbox, "bash");
    let bin_dir = ipc_bin_dir(&sandbox, "usage: nc [-46] host port");

    let doctor = doctor_with_agent_path(&sandbox, &bin_dir, "");
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    let line = doctor
        .stdout
        .lines()
        .find(|line| line.contains("Checking IPC client"))
        .unwrap_or_else(|| panic!("no IPC client line: {doctor:?}"));
    assert!(line.contains("warn: No IPC client for Bash/Fish"), "{line}");
    assert!(doctor.stdout.contains("socat"), "{doctor:?}");
    assert!(doctor.stdout.contains(ALL_PASSED), "{doctor:?}");
}

#[test]
fn doctor_accepts_nc_with_unix_socket_support() {
    let sandbox = AgentSandbox::new();
    install_integration(&sandbox, "bash");
    let bin_dir = ipc_bin_dir(&sandbox, "usage: nc [-46] [-U] host port");

    let doctor = doctor_with_agent_path(&sandbox, &bin_dir, "");
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    let expected = format!("ok: nc -U ({})", bin_dir.join("nc").display());
    assert!(
        doctor
            .stdout
            .lines()
            .any(|line| line.contains("Checking IPC client") && line.contains(&expected)),
        "{doctor:?}"
    );
}

#[test]
fn doctor_skips_the_ipc_check_when_the_agent_is_disabled() {
    let sandbox = AgentSandbox::new();
    install_integration(&sandbox, "bash");
    let bin_dir = ipc_bin_dir(&sandbox, "usage: nc [-46] host port");

    let doctor = doctor_with_agent_path(&sandbox, &bin_dir, "[agent]\nenabled = false\n");
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    assert!(!doctor.stdout.contains("IPC client"), "{doctor:?}");
}

#[test]
fn doctor_skips_the_ipc_check_for_zsh_only() {
    let sandbox = AgentSandbox::new();
    install_integration(&sandbox, "zsh");
    let bin_dir = ipc_bin_dir(&sandbox, "usage: nc [-46] host port");

    let doctor = doctor_with_agent_path(&sandbox, &bin_dir, "");
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    assert!(!doctor.stdout.contains("IPC client"), "{doctor:?}");
}

#[test]
fn doctor_prints_versions_without_a_v_prefix() {
    let sandbox = AgentSandbox::new();

    let doctor = sandbox.gpy(&["doctor"]);
    let line = doctor
        .stdout
        .lines()
        .find(|line| line.contains("gpy-core"))
        .unwrap_or_else(|| panic!("no gpy-core line: {doctor:?}"));
    assert!(
        line.contains(&format!(
            "gpy-core {} [first-party]",
            env!("CARGO_PKG_VERSION")
        )),
        "{line}"
    );
}

#[test]
fn doctor_does_not_fail_on_an_inactive_broken_theme_override() {
    let sandbox = AgentSandbox::new();
    std::fs::write(
        sandbox.env.themes_dir().join("starship.toml"),
        "[ui]\nseparator = \" \"\n\n[segments.username]\nformat = \"{value}\"\nshow_always = true\n",
    )
    .unwrap();

    let doctor = sandbox.gpy(&["doctor"]);
    assert!(
        doctor
            .stdout
            .lines()
            .any(|line| line.contains("warn: Fails to load (inactive, not blocking)")),
        "{doctor:?}"
    );
    assert_eq!(doctor.exit_code, 0_i32, "{doctor:?}");
    assert!(doctor.stdout.contains(ALL_PASSED), "{doctor:?}");
}
