//! End-to-end `gpy` lifecycle and liveness tests against a real, double-forked
//! `gpy-agent` daemon. Socket-dependent, so Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

mod common;

use common::{CliCommandResult, CliTestEnv, gpy_test_root};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// How long a freshly started daemon gets to answer `status`.
const READY_DEADLINE: Duration = Duration::from_secs(10);
/// How long a stopped daemon gets to remove its socket.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

const RUNNING_LINE: &str = "Status: Running and Responding";
const NOT_RUNNING_LINE: &str = "Status: Not Running (socket not found)";
const NOT_RELOADED_NOTICE: &str = "Agent not reloaded";

/// A `CliTestEnv` plus a socket under `gpy_test_root()`, with a drop guard
/// that stops the daemon through its own socket (there is no child handle to
/// kill). A leaked daemon is still reaped by `scripts/cleanup-test-agents.sh`.
struct AgentSandbox {
    env: CliTestEnv,
    _socket_dir: TempDir,
    socket_path: PathBuf,
}

impl AgentSandbox {
    fn new() -> Self {
        let env = CliTestEnv::new().expect("create isolated CLI test env");
        let root = gpy_test_root().expect("GPY test root");
        let socket_dir = tempfile::Builder::new()
            .prefix("lifecycle-")
            .tempdir_in(root)
            .expect("socket tempdir under the test root");
        let socket_path = socket_dir.path().join("gpy.sock");
        Self {
            env,
            _socket_dir: socket_dir,
            socket_path,
        }
    }

    fn socket_env(&self) -> [(&'static str, String); 1] {
        [(
            "GPY_AGENT_SOCKET_PATH",
            self.socket_path.to_string_lossy().into_owned(),
        )]
    }

    fn gpy(&self, args: &[&str]) -> CliCommandResult {
        self.env
            .run_gpy_with_env(args, &self.socket_env())
            .expect("spawn gpy")
    }

    fn gpy_agent(&self, args: &[&str]) -> CliCommandResult {
        self.env
            .run_gpy_agent_with_env(args, &self.socket_env())
            .expect("spawn gpy-agent")
    }

    fn start_and_wait(&self) {
        let started = self.gpy(&["start"]);
        started.assert_success("gpy start");
        self.wait_until_responding();
    }

    fn wait_until_responding(&self) {
        let started = Instant::now();
        let mut last = None;
        while started.elapsed() < READY_DEADLINE {
            let status = self.gpy(&["status"]);
            if status.exit_code == 0_i32 && status.stdout.contains(RUNNING_LINE) {
                return;
            }
            last = Some(status);
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("agent did not answer `gpy status` within {READY_DEADLINE:?}; last: {last:?}");
    }

    fn wait_until_socket_gone(&self) -> bool {
        let started = Instant::now();
        while started.elapsed() < STOP_DEADLINE {
            if !self.socket_path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

impl Drop for AgentSandbox {
    fn drop(&mut self) {
        if self.socket_path.exists() {
            let _ = self.gpy_agent(&["stop"]);
            // A daemon that ignores stop is left for cleanup-test-agents.sh.
            let _ = self.wait_until_socket_gone();
        }
    }
}

/// Keep a listener open so the socket file exists but nothing answers.
fn bind_silent_socket(path: &Path) -> std::os::unix::net::UnixListener {
    std::os::unix::net::UnixListener::bind(path).expect("bind silent socket")
}

#[test]
fn agent_status_exit_code_reflects_liveness() {
    let sandbox = AgentSandbox::new();

    // No socket at all.
    let absent = sandbox.gpy_agent(&["status"]);
    assert_eq!(absent.exit_code, 1_i32, "no socket must exit 1: {absent:?}");
    assert!(absent.stdout.contains(NOT_RUNNING_LINE), "{absent:?}");
    assert!(
        absent.stderr.is_empty(),
        "no Error: line for a finding: {absent:?}"
    );

    // A socket file nothing answers on.
    {
        let _listener = bind_silent_socket(&sandbox.socket_path);
        let stale = sandbox.gpy_agent(&["status"]);
        assert_eq!(
            stale.exit_code, 1_i32,
            "unanswered socket must exit 1: {stale:?}"
        );
        assert!(
            stale.stdout.contains("Status: Connection failed"),
            "{stale:?}"
        );
    }
    std::fs::remove_file(&sandbox.socket_path).expect("remove silent socket");

    // A live agent.
    sandbox.start_and_wait();
    let live = sandbox.gpy_agent(&["status"]);
    assert_eq!(live.exit_code, 0_i32, "live agent must exit 0: {live:?}");
    assert!(live.stdout.contains(RUNNING_LINE), "{live:?}");
    assert!(
        live.stdout.contains("Agent Version: "),
        "the live report carries the agent version: {live:?}"
    );
}

#[test]
fn gpy_status_passes_the_agent_exit_code_through() {
    let sandbox = AgentSandbox::new();

    let absent = sandbox.gpy(&["status"]);
    assert_eq!(absent.exit_code, 1_i32, "{absent:?}");
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
fn doctor_reports_not_running_when_no_agent() {
    let sandbox = AgentSandbox::new();

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(
        doctor.exit_code, 1_i32,
        "doctor with no agent must fail: {doctor:?}"
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

/// `gpy` copied alone into a fresh directory: no sibling `gpy-agent`, so it
/// must find the agent on `PATH`.
fn lone_gpy(env: &CliTestEnv) -> PathBuf {
    let dir = env.root().join("lone-gpy");
    std::fs::create_dir_all(&dir).expect("create lone gpy dir");
    let gpy = dir.join("gpy");
    std::fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).expect("copy gpy");
    gpy
}

/// `gpy doctor` with the agent disabled (so only the binary check can fail)
/// and PATH holding just `bin_dir`.
fn doctor_with_agent_path(sandbox: &AgentSandbox, bin_dir: &Path) -> CliCommandResult {
    sandbox
        .env
        .write_config("[agent]\nenabled = false\n")
        .expect("write config");
    sandbox
        .env
        .run_program_with_env(&lone_gpy(&sandbox.env), &["doctor"], &[("PATH", bin_dir)])
        .expect("run gpy doctor")
}

#[test]
fn doctor_fails_a_gpy_agent_whose_version_probe_fails() {
    let sandbox = AgentSandbox::new();
    let bin_dir = sandbox.env.root().join("broken-bin");
    std::fs::create_dir(&bin_dir).unwrap();
    let agent = bin_dir.join("gpy-agent");
    std::fs::write(&agent, "#!/bin/sh\necho 'loader failed' >&2\nexit 42\n").unwrap();
    std::fs::set_permissions(&agent, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    let broken = doctor_with_agent_path(&sandbox, &bin_dir);
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
    std::fs::create_dir(&empty_dir).unwrap();
    let missing = doctor_with_agent_path(&sandbox, &empty_dir);
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
fn doctor_reports_not_running_on_a_stale_socket() {
    let sandbox = AgentSandbox::new();
    let _listener = bind_silent_socket(&sandbox.socket_path);

    let doctor = sandbox.gpy(&["doctor"]);
    assert_eq!(doctor.exit_code, 1_i32, "{doctor:?}");
    assert!(
        doctor
            .stdout
            .contains("Checking process... warn: Not running"),
        "{doctor:?}"
    );
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
    assert!(
        doctor.stdout.contains("ok: All checks passed."),
        "{doctor:?}"
    );
}

#[test]
fn gpy_start_status_stop_round_trip() {
    let sandbox = AgentSandbox::new();

    sandbox.start_and_wait();
    assert!(sandbox.socket_path.exists(), "socket is bound after start");

    let stopped = sandbox.gpy(&["stop"]);
    stopped.assert_success("gpy stop");
    assert!(
        stopped
            .stdout
            .contains("Shutdown command sent successfully"),
        "{stopped:?}"
    );
    assert!(
        stopped.stdout.contains("Agent stopped successfully"),
        "the stop must be confirmed, not merely sent: {stopped:?}"
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
    assert_eq!(after.exit_code, 1_i32, "{after:?}");
    assert!(after.stdout.contains(NOT_RUNNING_LINE), "{after:?}");
}

fn pids_matching(needle: &Path) -> Vec<String> {
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg("--")
        .arg(needle)
        .output()
        .expect("run pgrep");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// `ps` rows (pid, parent, state, age, argv) for `pids`, so a failing
/// duplicate-daemon assertion says what the survivors are.
fn describe_pids(pids: &[String]) -> String {
    let output = std::process::Command::new("ps")
        .args(["-o", "pid,ppid,stat,etime,args", "-p", &pids.join(",")])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sends `SIGTERM` to every daemon started with `--socket <path>`, so a
/// failing run never leaks a duplicate agent the socket cannot reach.
struct DaemonReaper(PathBuf);

impl Drop for DaemonReaper {
    fn drop(&mut self) {
        for pid in pids_matching(&self.0) {
            let _ = std::process::Command::new("kill").arg(&pid).status();
        }
    }
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

    assert!(start().wait().expect("first start").success());
    sandbox.wait_until_responding();
    std::fs::write(format!("{socket}.version"), "0.0.0-old").expect("age the running agent");

    let mut first = common::KillOnDrop::new(start());
    // Stagger the second start into the first one's eviction window.
    std::thread::sleep(Duration::from_millis(100));
    let mut second = common::KillOnDrop::new(start());
    let _ = first.wait().expect("first racing start");
    let _ = second.wait().expect("second racing start");

    // The evicted daemon exits asynchronously; wait for the survivor count to
    // settle instead of sampling after a fixed pause.
    let _ = common::wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
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
        assert!(
            std::os::unix::net::UnixStream::connect(&sandbox.socket_path).is_ok(),
            "round {round}: the socket must accept connections when `gpy start` exits 0"
        );
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
    assert!(!sandbox.socket_path.exists(), "nothing was bound");
    assert!(
        common::wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
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
        common::wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
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
        common::wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
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
    assert_eq!(status.exit_code, 0, "status after restart: {status:?}");

    sandbox
        .env
        .run_gpy_agent(&["stop", "--socket", &socket])
        .expect("spawn gpy-agent stop");
    assert!(
        common::wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
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
    assert!(
        std::os::unix::net::UnixStream::connect(runtime.join("gpy.sock")).is_ok(),
        "the agent serves from the repaired dir"
    );
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

    for result in [
        env.run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", &socket)]),
        env.run_gpy_agent_with_env(
            &["start", "--socket", &socket_arg],
            &[("GPY_AGENT_SOCKET_PATH", "")],
        ),
    ] {
        let result = result.expect("spawn gpy-agent");
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
    assert!(
        stopped
            .stdout
            .contains("Agent is not running (socket not found)"),
        "{stopped:?}"
    );
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
    assert!(stopped.stderr.contains("did not answer"), "{stopped:?}");

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
    assert!(stopped.stdout.contains("not running"), "{stopped:?}");
    assert!(!stopped.stdout.contains("kill"), "{stopped:?}");
    assert!(!stopped.stderr.contains("kill"), "{stopped:?}");
    assert!(!sandbox.socket_path.exists(), "{stopped:?}");
}

#[test]
fn gpy_restart_prints_normalized_steps() {
    let sandbox = AgentSandbox::new();
    sandbox.start_and_wait();

    let restarted = sandbox.gpy(&["restart"]);
    restarted.assert_success("gpy restart");
    let lines: Vec<&str> = restarted.stdout.lines().collect();
    assert_eq!(
        lines,
        vec![
            "Restarting GPY...",
            "Shutdown command sent",
            "GPY stopped",
            "GPY started",
        ],
        "stdout must be exactly the four normalized steps: {restarted:?}"
    );
    assert!(
        !restarted
            .stderr
            .contains("Starting GPY Agent in background"),
        "the raw gpy-agent progress line is suppressed on restart: {restarted:?}"
    );

    sandbox.wait_until_responding();
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
        assert_eq!(result.exit_code, 1_i32, "gpy {command}: {result:?}");
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
        !switched.stdout.contains(NOT_RELOADED_NOTICE),
        "a live agent must acknowledge the reload: {switched:?}"
    );

    let theme = sandbox.gpy(&["config", "get", "ui.theme"]);
    theme.assert_success("gpy config get ui.theme");
    assert_eq!(theme.stdout.trim(), "starship", "{theme:?}");
}

#[test]
fn mutation_without_agent_prints_exactly_one_not_reloaded_notice() {
    let sandbox = AgentSandbox::new();

    let switched = sandbox.gpy(&["theme", "use", "starship"]);
    switched.assert_success("gpy theme use starship with no agent");
    assert_eq!(
        switched.stdout.matches(NOT_RELOADED_NOTICE).count(),
        1_usize,
        "{switched:?}"
    );
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
