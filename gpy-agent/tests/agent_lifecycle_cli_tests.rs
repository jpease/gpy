//! End-to-end `gpy` lifecycle and liveness tests against a real, double-forked
//! `gpy-agent` daemon. Socket-dependent, so Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use common::agent_sandbox::{
    AgentSandbox, DaemonReaper, NOT_RUNNING_LINE, RUNNING_LINE, describe_pids, pids_matching,
};
use common::child_guard::KillOnDrop;
use common::cli_harness::{CliCommandResult, CliTestEnv, lone_gpy};
use common::fake_agent::{Behaviour, FakeAgent};
use common::fixtures::{bind_silent_socket, gpy_test_root};
use common::wait::wait_until;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[test]
fn gpy_status_passes_the_agent_exit_code_through() {
    let sandbox = AgentSandbox::new();

    let absent = sandbox.gpy(&["status"]);
    assert_eq!(absent.exit_code, 3_i32, "{absent:?}");
    assert!(absent.stdout.contains(NOT_RUNNING_LINE), "{absent:?}");
    assert!(
        !absent.stderr.contains("Error:"),
        "a not-running agent is a finding, not a gpy failure: {absent:?}"
    );

    sandbox.start_and_wait();
    let live = sandbox.gpy(&["status"]);
    assert_eq!(live.exit_code, 0_i32, "{live:?}");
    assert!(live.stdout.contains(RUNNING_LINE), "{live:?}");
}

#[test]
fn gpy_start_status_stop_round_trip() {
    let sandbox = AgentSandbox::new();

    sandbox.start_and_wait();
    assert!(sandbox.socket_path.exists(), "socket is bound after start");

    let stopped = sandbox.gpy(&["stop"]);
    stopped.assert_success("gpy stop");
    assert!(
        stopped.stdout.starts_with("Stopped agent (pid "),
        "the stop must be confirmed with the PID: {stopped:?}"
    );
    assert!(
        !stopped.stderr.contains("Failed to send shutdown command"),
        "a live agent's shutdown ack must be recognised: {stopped:?}"
    );
    assert!(
        sandbox.wait_until_socket_gone(),
        "socket must be removed after stop"
    );

    let after = sandbox.gpy(&["status"]);
    assert_eq!(after.exit_code, 3_i32, "{after:?}");
    assert!(after.stdout.contains(NOT_RUNNING_LINE), "{after:?}");
}

#[test]
fn concurrent_starts_during_eviction_leave_one_daemon() {
    let sandbox = AgentSandbox::new();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let _reaper = DaemonReaper(sandbox.socket_path.clone());
    let start = || {
        sandbox
            .env
            .gpy_agent_command()
            .args(["start", "--socket", &socket])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn gpy-agent start")
    };

    // Stand-in for an old agent: answers as 0.0.0-old and stops on shutdown.
    let _old_agent = FakeAgent::spawn_reporting(
        &sandbox.socket_path,
        Behaviour::AliveUntilShutdown,
        "0.0.0-old",
        gpy_agent::ipc::protocol::PROTOCOL_VERSION,
    );

    let mut first = KillOnDrop::new(start());
    // Stagger the second start into the first one's eviction window.
    std::thread::sleep(Duration::from_millis(100));
    let mut second = KillOnDrop::new(start());
    let _ = first.wait().expect("first racing start");
    let _ = second.wait().expect("second racing start");

    // The evicted daemon exits asynchronously; wait for the survivor count to
    // settle instead of sampling after a fixed pause.
    let _ = wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
        pids_matching(&sandbox.socket_path).len() == 1
    });

    let daemons = pids_matching(&sandbox.socket_path);
    assert_eq!(
        daemons.len(),
        1,
        "exactly one daemon must survive: {daemons:?}\n{}",
        describe_pids(&daemons)
    );
}

#[test]
fn gpy_start_returns_only_when_agent_answers() {
    let sandbox = AgentSandbox::new();
    for round in 0_u32..5 {
        sandbox.gpy(&["start"]).assert_success("gpy start");
        // Connect at once: spawning `gpy status` first would give the daemon
        // time to bind and hide the race.
        let msg =
            format!("round {round}: the socket must accept connections when `gpy start` exits 0");
        let _stream = std::os::unix::net::UnixStream::connect(&sandbox.socket_path).expect(&msg);
        sandbox.gpy(&["stop"]).assert_success("gpy stop");
        assert!(sandbox.wait_until_socket_gone(), "round {round}: stop");
    }
}

#[test]
fn gpy_start_fails_when_agent_cannot_bind() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = AgentSandbox::new();
    let dir = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let socket = dir.path().join("gpy.sock");

    for result in [
        sandbox
            .env
            .run_gpy_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", &socket)])
            .expect("spawn gpy"),
        sandbox
            .env
            .run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", &socket)])
            .expect("spawn gpy-agent"),
    ] {
        assert_eq!(result.exit_code, 1_i32, "{result:?}");
        assert!(result.stderr.contains("Error:"), "{result:?}");
    }

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

// The lock is taken but the daemon itself cannot bind (the path is too
// long for `sun_path`), so the failure must come from the readiness wait.
#[test]
fn gpy_agent_start_fails_when_the_daemon_dies_before_binding() {
    let sandbox = AgentSandbox::new();
    let dir = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    let socket = dir.path().join(format!("{}.sock", "x".repeat(120)));

    let result = sandbox
        .env
        .run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", &socket)])
        .expect("spawn gpy-agent");
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(result.stderr.contains("Error:"), "{result:?}");
}

/// The detached daemon's stderr is `/dev/null`; its initialization error must
/// still reach `start`, and the failed daemon must not linger.
#[test]
fn gpy_agent_start_reports_the_daemon_initialization_error() {
    let sandbox = AgentSandbox::new();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let _reaper = DaemonReaper(sandbox.socket_path.clone());
    let cache_file = sandbox.env.root().join("cache-is-a-file");
    std::fs::write(&cache_file, "").expect("write cache file");
    let cause = std::fs::create_dir_all(cache_file.join("gpy"))
        .expect_err("a regular file cannot hold a directory")
        .to_string();

    let result = sandbox
        .env
        .run_gpy_agent_with_env(
            &["start", "--socket", &socket],
            &[("XDG_CACHE_HOME", cache_file.as_os_str())],
        )
        .expect("spawn gpy-agent");
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(
        result.stderr.contains(&cache_file.display().to_string()),
        "the error names the offending path: {result:?}"
    );
    assert!(
        result.stderr.contains(&cause),
        "the error carries the filesystem cause `{cause}`: {result:?}"
    );
    assert!(
        result
            .stderr
            .contains("Error: Agent process error: the daemon failed before it was ready:"),
        "the daemon failure is one Agent error line: {result:?}"
    );
    assert!(
        !result.stderr.contains("command '"),
        "an internal step is not a failed command: {result:?}"
    );
    assert!(!sandbox.socket_path.exists(), "nothing was bound");
    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
            pids_matching(&sandbox.socket_path).is_empty()
        }),
        "the failed daemon exits"
    );
}

struct SocketStopper<'a>(&'a CliTestEnv, PathBuf);

impl Drop for SocketStopper<'_> {
    fn drop(&mut self) {
        let socket = self.1.to_string_lossy().into_owned();
        let _ = self.0.run_gpy_agent(&["stop", "--socket", &socket]);
    }
}

fn process_cwd(pid: &str) -> PathBuf {
    if cfg!(target_os = "linux") {
        return std::fs::read_link(format!("/proc/{pid}/cwd")).expect("read /proc cwd");
    }
    let output = std::process::Command::new("lsof")
        .args(["-a", "-p", pid, "-d", "cwd", "-Fn"])
        .output()
        .expect("run lsof");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix('n'))
        .map(PathBuf::from)
        .expect("lsof reports a cwd")
}

#[test]
fn daemon_does_not_keep_the_launch_directory_as_cwd() {
    let sandbox = AgentSandbox::new();
    let launch = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let _reaper = DaemonReaper(sandbox.socket_path.clone());

    let status = sandbox
        .env
        .gpy_agent_command()
        .args(["start", "--socket", &socket])
        .current_dir(launch.path())
        .status()
        .expect("spawn gpy-agent start");
    assert!(status.success());
    sandbox.wait_until_responding();

    let daemons = pids_matching(&sandbox.socket_path);
    let [pid] = daemons.as_slice() else {
        panic!("expected one daemon, found {daemons:?}");
    };
    assert_eq!(process_cwd(pid), PathBuf::from("/"));
}

/// Start a daemon on the sandbox socket and return its single pid.
fn start_signal_target(sandbox: &AgentSandbox, socket: &str) -> String {
    let status = sandbox
        .env
        .gpy_agent_command()
        .args(["start", "--socket", socket])
        .status()
        .expect("spawn gpy-agent start");
    assert!(status.success());
    sandbox.wait_until_responding();
    let daemons = pids_matching(&sandbox.socket_path);
    let [pid] = daemons.as_slice() else {
        panic!("expected one daemon, found {daemons:?}");
    };
    pid.clone()
}

fn send_signal(signal: &str, pid: &str) {
    let status = std::process::Command::new("kill")
        .args([signal, pid])
        .status()
        .expect("spawn kill");
    assert!(status.success(), "kill {signal} {pid} failed");
}

#[test]
fn sigterm_stops_the_daemon_and_unlinks_its_socket() {
    let sandbox = AgentSandbox::new();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let _reaper = DaemonReaper(sandbox.socket_path.clone());

    let pid = start_signal_target(&sandbox, &socket);
    send_signal("-TERM", &pid);

    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
            pids_matching(&sandbox.socket_path).is_empty() && !sandbox.socket_path.exists()
        }),
        "SIGTERM must stop the daemon and unlink its socket"
    );
}

#[test]
fn sigkill_leaves_a_stale_socket_that_start_replaces() {
    let sandbox = AgentSandbox::new();
    let socket = sandbox.socket_path.to_string_lossy().into_owned();
    let _reaper = DaemonReaper(sandbox.socket_path.clone());

    let pid = start_signal_target(&sandbox, &socket);
    send_signal("-KILL", &pid);

    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
            pids_matching(&sandbox.socket_path).is_empty()
        }),
        "SIGKILL must end the daemon"
    );
    assert!(
        sandbox.socket_path.exists(),
        "a SIGKILLed daemon cannot unlink its socket"
    );

    start_signal_target(&sandbox, &socket);
    let status = sandbox
        .env
        .run_gpy_agent(&["status", "--socket", &socket])
        .expect("spawn gpy-agent status");
    assert_eq!(status.exit_code, 0_i32, "status after restart: {status:?}");

    sandbox
        .env
        .run_gpy_agent(&["stop", "--socket", &socket])
        .expect("spawn gpy-agent stop");
    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
            pids_matching(&sandbox.socket_path).is_empty() && !sandbox.socket_path.exists()
        }),
        "the restarted daemon must stop and unlink its socket"
    );
}

#[test]
fn relative_socket_is_resolved_against_the_launch_directory() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let launch = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    let socket = launch.path().join("rel.sock");
    let _stopper = SocketStopper(&env, socket.clone());

    let status = env
        .gpy_agent_command()
        .args(["start", "--socket", "rel.sock"])
        .current_dir(launch.path())
        .status()
        .expect("spawn gpy-agent start");
    assert!(status.success());
    assert!(
        socket.exists(),
        "rel.sock must be bound in the launch directory"
    );

    let socket_arg = socket.to_string_lossy().into_owned();
    let reported = env
        .run_gpy_agent(&["status", "--socket", &socket_arg])
        .expect("spawn gpy-agent status");
    assert_eq!(reported.exit_code, 0_i32, "{reported:?}");
    assert!(reported.stdout.contains(RUNNING_LINE), "{reported:?}");
}

/// The default endpoint `$XDG_RUNTIME_DIR/gpy/gpy.sock`, with any inherited
/// `GPY_AGENT_SOCKET_PATH` blanked (empty means unset).
fn start_on_default_socket(env: &CliTestEnv) -> CliCommandResult {
    env.run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", "")])
        .expect("spawn gpy-agent start")
}

/// Read-only commands must not create GPY's runtime directory.
#[test]
fn status_does_not_create_the_runtime_dir() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let runtime = env.xdg_runtime_dir().join("gpy");
    assert!(!runtime.exists(), "precondition: no runtime dir yet");

    let result = env
        .run_gpy_agent_with_env(&["status"], &[("GPY_AGENT_SOCKET_PATH", "")])
        .expect("spawn gpy-agent status");

    assert_eq!(result.exit_code, 3_i32, "no agent is running: {result:?}");
    assert!(
        !runtime.exists(),
        "status created {}: {result:?}",
        runtime.display()
    );
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777
}

#[test]
fn start_tightens_an_owned_world_writable_runtime_dir() {
    use std::os::unix::fs::PermissionsExt;

    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let runtime = env.xdg_runtime_dir().join("gpy");
    std::fs::create_dir_all(&runtime).expect("create runtime dir");
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o777)).expect("chmod");
    let _stopper = SocketStopper(&env, runtime.join("gpy.sock"));

    start_on_default_socket(&env).assert_success("gpy-agent start");
    assert_eq!(mode_of(&runtime), 0o700, "the runtime dir is repaired");
    let _stream = std::os::unix::net::UnixStream::connect(runtime.join("gpy.sock"))
        .expect("the agent serves from the repaired dir");
}

#[test]
fn start_refuses_a_symlinked_runtime_dir() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let target = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    let runtime = env.xdg_runtime_dir().join("gpy");
    std::os::unix::fs::symlink(target.path(), &runtime).expect("symlink runtime dir");
    let _stopper = SocketStopper(&env, runtime.join("gpy.sock"));

    let result = start_on_default_socket(&env);
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(
        result.stderr.contains(&runtime.display().to_string()),
        "the refusal names the directory: {result:?}"
    );
    assert_eq!(
        std::fs::read_dir(target.path())
            .expect("read target")
            .count(),
        0,
        "nothing is created through the symlink"
    );
}

/// `gpy-agent start` on the default socket fails naming `runtime` and the
/// `io::ErrorKind` that `create_dir_all(runtime)` reports.
fn assert_start_reports_runtime_dir_error(env: &CliTestEnv, xdg_runtime: &Path, runtime: &Path) {
    let cause = std::fs::create_dir_all(runtime).expect_err("the runtime dir cannot be created");
    let result = env
        .run_gpy_agent_with_env(
            &["start"],
            &[
                ("GPY_AGENT_SOCKET_PATH", std::ffi::OsStr::new("")),
                ("XDG_RUNTIME_DIR", xdg_runtime.as_os_str()),
            ],
        )
        .expect("spawn gpy-agent start");
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(
        result.stderr.contains(&runtime.display().to_string()),
        "the error names the runtime dir: {result:?}"
    );
    assert!(
        result.stderr.contains(&cause.to_string()),
        "the error carries the {:?} cause `{cause}`: {result:?}",
        cause.kind()
    );
}

#[test]
fn start_reports_an_xdg_runtime_dir_that_is_a_regular_file() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let file = env.root().join("runtime-is-a-file");
    std::fs::write(&file, "").expect("write runtime file");

    assert_start_reports_runtime_dir_error(&env, &file, &file.join("gpy"));
}

#[test]
fn start_reports_a_dangling_runtime_dir_symlink() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let runtime = env.xdg_runtime_dir().join("gpy");
    std::os::unix::fs::symlink(env.root().join("missing"), &runtime).expect("dangling symlink");

    assert_start_reports_runtime_dir_error(&env, env.xdg_runtime_dir(), &runtime);
}

#[test]
fn start_refuses_an_explicit_socket_whose_directory_is_a_symlink() {
    let sandbox = AgentSandbox::new();
    let target = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    let link = sandbox
        .socket_path
        .parent()
        .expect("socket dir")
        .join("link");
    std::os::unix::fs::symlink(target.path(), &link).expect("symlink socket dir");
    let socket = link.join("gpy.sock");
    let _stopper = SocketStopper(&sandbox.env, socket.clone());

    let result = sandbox
        .env
        .run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", &socket)])
        .expect("spawn gpy-agent");
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(
        result.stderr.contains(&link.display().to_string()),
        "the refusal names the directory: {result:?}"
    );
    assert_eq!(
        std::fs::read_dir(target.path())
            .expect("read target")
            .count(),
        0,
        "nothing is created through the symlink"
    );
}

/// A directory the user chose is never chmod'ed, through either override.
#[test]
fn start_refuses_an_explicit_socket_in_an_owned_world_writable_dir() {
    use std::os::unix::fs::PermissionsExt;

    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let dir = tempfile::tempdir_in(gpy_test_root().expect("GPY test root")).expect("tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).expect("chmod");
    let socket = dir.path().join("gpy.sock");
    let socket_arg = socket.to_string_lossy().into_owned();
    let _stopper = SocketStopper(&env, socket.clone());

    for spawned in [
        env.run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", &socket)]),
        env.run_gpy_agent_with_env(
            &["start", "--socket", &socket_arg],
            &[("GPY_AGENT_SOCKET_PATH", "")],
        ),
    ] {
        let result = spawned.expect("spawn gpy-agent");
        assert_eq!(result.exit_code, 1_i32, "{result:?}");
        assert!(
            result.stderr.contains(&dir.path().display().to_string()),
            "the refusal names the directory: {result:?}"
        );
        assert_eq!(mode_of(dir.path()), 0o777, "the mode is left unchanged");
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("read dir").count(),
            0,
            "nothing is created in the refused directory"
        );
    }
}

#[test]
fn gpy_stop_when_not_running_is_a_noop() {
    let sandbox = AgentSandbox::new();

    let stopped = sandbox.gpy(&["stop"]);
    stopped.assert_success("gpy stop with no agent");
    assert!(stopped.stdout.contains("Agent not running."), "{stopped:?}");
}

/// `gpy debug prompt` with no agent is an unreachable agent: exit 1 with
/// an `Error:` line on stderr, like every other failed connection.
#[test]
fn debug_prompt_fails_without_agent() {
    let sandbox = AgentSandbox::new();

    let out = sandbox.gpy(&["debug", "prompt"]);
    assert_eq!(out.exit_code, 1_i32, "{out:?}");
    assert!(out.stderr.contains("Error:"), "{out:?}");
    assert!(out.stderr.contains("Agent is not running"), "{out:?}");
}

/// A socket that accepts but never answers is an agent that could not
/// be stopped; `stop` must say so and exit 1, from both binaries.
#[test]
fn gpy_stop_fails_when_agent_does_not_answer() {
    let sandbox = AgentSandbox::new();
    let _listener = bind_silent_socket(&sandbox.socket_path);

    let stopped = sandbox.gpy(&["stop"]);
    assert_eq!(stopped.exit_code, 1_i32, "{stopped:?}");
    assert!(stopped.stderr.contains("Error:"), "{stopped:?}");
    assert!(
        stopped
            .stderr
            .contains(&sandbox.socket_path.display().to_string()),
        "{stopped:?}"
    );

    let direct = sandbox.gpy_agent(&["stop"]);
    assert_eq!(direct.exit_code, 1_i32, "{direct:?}");
    assert!(direct.stderr.contains("Error:"), "{direct:?}");
}

/// A socket file with nothing listening is "not running", not a
/// process to `kill`; the stale file is removed.
#[test]
fn gpy_stop_on_stale_socket_reports_not_running() {
    let sandbox = AgentSandbox::new();
    drop(bind_silent_socket(&sandbox.socket_path));
    assert!(sandbox.socket_path.exists(), "fixture must leave the file");

    let stopped = sandbox.gpy(&["stop"]);
    stopped.assert_success("gpy stop on a stale socket");
    assert!(
        stopped.stdout.contains("removed stale socket"),
        "{stopped:?}"
    );
    assert!(!stopped.stdout.contains("kill"), "{stopped:?}");
    assert!(!stopped.stderr.contains("kill"), "{stopped:?}");
    assert!(!sandbox.socket_path.exists(), "{stopped:?}");
}

#[test]
fn gpy_lifecycle_fails_clearly_when_gpy_agent_missing() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let empty_path = env.root().join("empty-path");
    std::fs::create_dir_all(&empty_path).expect("create empty PATH dir");
    let empty = empty_path.to_string_lossy().into_owned();
    let gpy = lone_gpy(&env);

    for command in ["start", "stop", "restart", "status"] {
        let result = env
            .run_program_with_env(&gpy, &[command], &[("PATH", empty.as_str())])
            .expect("spawn gpy");
        let want = if command == "status" { 4_i32 } else { 1_i32 };
        assert_eq!(result.exit_code, want, "gpy {command}: {result:?}");
        assert!(
            result.stderr.starts_with("Error: "),
            "gpy {command} must report one Error: line: {result:?}"
        );
        assert!(
            result.stderr.contains("gpy-agent"),
            "gpy {command} must name the missing binary: {result:?}"
        );
        assert!(
            !result.stderr.contains("{ message:"),
            "no Debug-formatted enum: {result:?}"
        );
    }
}

/// The same minimal config with only the theme varying.
fn theme_config(theme: &str) -> String {
    format!("[ui]\ntheme = \"{theme}\"\n")
}

#[test]
fn config_reload_applies_a_valid_config() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();
    sandbox
        .env
        .write_config(&theme_config("starship"))
        .expect("write config");

    let reloaded = sandbox.gpy_agent(&["config", "reload"]);
    reloaded.assert_success("gpy-agent config reload");
    assert!(reloaded.stderr.is_empty(), "{reloaded:?}");
}

#[test]
fn config_reload_surfaces_a_toml_parse_error_and_keeps_serving() {
    const BROKEN: &str = "not [valid";
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();
    sandbox.env.write_config(BROKEN).expect("write config");
    let cause = toml::from_str::<toml::Table>(BROKEN)
        .expect_err("the fixture is invalid TOML")
        .message()
        .to_owned();

    let reloaded = sandbox.gpy_agent(&["config", "reload"]);
    assert_eq!(reloaded.exit_code, 1_i32, "{reloaded:?}");
    assert!(reloaded.stderr.starts_with("Error:"), "{reloaded:?}");
    assert!(
        reloaded.stderr.contains(&cause),
        "the agent's parse error `{cause}` reaches the user: {reloaded:?}"
    );

    let status = sandbox.gpy_agent(&["status"]);
    assert!(
        status.stdout.contains(RUNNING_LINE),
        "a rejected reload leaves the agent serving: {status:?}"
    );
}

#[test]
fn config_reload_rejects_an_unknown_theme() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();
    sandbox
        .env
        .write_config(&theme_config("no-such-theme"))
        .expect("write config");

    let reloaded = sandbox.gpy_agent(&["config", "reload"]);
    assert_eq!(reloaded.exit_code, 1_i32, "{reloaded:?}");
    assert!(
        reloaded.stderr.contains("no-such-theme"),
        "the rejection names the theme: {reloaded:?}"
    );
}

/// No agent, a stale socket file, and an agent that never answers are three
/// different outcomes: only the last is a failure.
#[test]
fn config_reload_distinguishes_absent_and_unanswering_agents() {
    let sandbox = AgentSandbox::new();

    let absent = sandbox.gpy_agent(&["config", "reload"]);
    absent.assert_success("config reload with no agent");

    drop(bind_silent_socket(&sandbox.socket_path));
    let stale = sandbox.gpy_agent(&["config", "reload"]);
    stale.assert_success("config reload on a stale socket");
    assert_eq!(stale.stdout, absent.stdout, "a stale socket is no agent");
    std::fs::remove_file(&sandbox.socket_path).expect("remove stale socket");

    let _listener = bind_silent_socket(&sandbox.socket_path);
    let silent = sandbox.gpy_agent(&["config", "reload"]);
    assert_eq!(silent.exit_code, 1_i32, "{silent:?}");
    assert!(silent.stderr.starts_with("Error:"), "{silent:?}");
    assert!(
        silent
            .stderr
            .contains(&sandbox.socket_path.display().to_string()),
        "names the agent that did not answer: {silent:?}"
    );
}
