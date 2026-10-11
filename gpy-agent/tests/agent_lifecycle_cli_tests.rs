//! End-to-end coverage of the `gpy` lifecycle glue and liveness reporting
//! against a real `gpy-agent` (#648).
//!
//! Every test here runs the compiled `gpy` binary, which shells out to the
//! compiled `gpy-agent` on `PATH`, exactly as an installed user's shell does.
//! Nothing is mocked: `gpy start` double-forks a real daemon on a socket
//! under the GPY-owned test root, and the assertions read what a user would
//! read.
//!
//! Pins:
//! - #636 — `gpy-agent status` / `gpy status` exit `1` when no agent answers,
//!   `gpy doctor` reports `Not running` (and exits `1`) instead of `Running`,
//!   and the doctor hint names `gpy start`.
//! - #656 — the CLI recognises a live agent's acknowledgement, so `gpy stop`,
//!   `gpy restart` and every mutating command's config reload work against a
//!   real daemon (the #573 regression).
//!
//! Socket-dependent, so Unix-only; the hermetic CLI targets stay portable
//! (#653).
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

/// A `CliTestEnv` plus a socket path under the GPY-owned test root, with a
/// drop guard that stops whatever daemon was started on that socket.
///
/// The guard is the cleanup contract from `tests/CLEANUP_GUARD_PATTERN.md`
/// applied to a double-forked daemon: there is no child handle to kill, so
/// the guard asks the daemon to shut down through its own socket and waits,
/// bounded, for the socket to disappear. The socket lives under
/// `gpy_test_root()` (#619) so a daemon that outlives a killed test run is
/// still found by `scripts/cleanup-test-agents.sh`.
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

    /// `gpy start`, then poll `gpy status` until the daemon answers.
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
            // A daemon that ignores its own stop is left for
            // scripts/cleanup-test-agents.sh, which reaps everything under
            // the test root; nothing more can be done from a drop guard.
            let _ = self.wait_until_socket_gone();
        }
    }
}

/// Bind a listener on `path` and keep it open so the socket file exists but
/// nothing answers: the shape of a daemon that died without unlinking, or
/// one that is wedged.
fn bind_silent_socket(path: &Path) -> std::os::unix::net::UnixListener {
    std::os::unix::net::UnixListener::bind(path).expect("bind silent socket")
}

// ---- #636: exit codes and doctor liveness -------------------------------

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
            .contains("Checking process... ⚠️  Not running"),
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
        !doctor.stdout.contains("Checking process... ✅ Running"),
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
            .contains("Checking process... ℹ️  Disabled via config (agent.enabled = false)"),
        "{doctor:?}"
    );
    assert!(!doctor.stdout.contains("Not running"), "{doctor:?}");
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
            .contains("Checking process... ⚠️  Not running"),
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
        doctor.stdout.contains("Checking process... ✅ Running"),
        "{doctor:?}"
    );
    assert!(
        doctor
            .stdout
            .contains("✅ Your GPY environment is healthy and ready to go!"),
        "{doctor:?}"
    );
}

// ---- lifecycle round trips -----------------------------------------------

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
        "#656: a live agent's shutdown ack must be recognised: {stopped:?}"
    );
    assert!(
        sandbox.wait_until_socket_gone(),
        "socket must be removed after stop"
    );

    let after = sandbox.gpy(&["status"]);
    assert_eq!(after.exit_code, 1_i32, "{after:?}");
    assert!(after.stdout.contains(NOT_RUNNING_LINE), "{after:?}");
}

/// PIDs of processes whose argv contains `needle` (`pgrep -f`).
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
/// duplicate-daemon assertion says what the survivors are (#862).
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

// #723: two starts racing an eviction must leave exactly one daemon.
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

    let mut first = start();
    std::thread::sleep(Duration::from_millis(100));
    let mut second = start();
    let _ = first.wait().expect("first racing start");
    let _ = second.wait().expect("second racing start");

    std::thread::sleep(Duration::from_secs(1));

    let daemons = pids_matching(&sandbox.socket_path);
    assert_eq!(
        daemons.len(),
        1,
        "exactly one daemon must survive: {daemons:?}\n{}",
        describe_pids(&daemons)
    );
}

// #741: `gpy start` returns only once the daemon is listening.
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

// #741: a start whose agent can never bind exits 1 with an `Error:` line.
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

// #741: the lock is taken but the daemon itself cannot bind (the path is too
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

/// Stops the agent on an explicit `--socket` path when the test ends.
struct SocketStopper<'a>(&'a CliTestEnv, PathBuf);

impl Drop for SocketStopper<'_> {
    fn drop(&mut self) {
        let socket = self.1.to_string_lossy().into_owned();
        let _ = self.0.run_gpy_agent(&["stop", "--socket", &socket]);
    }
}

/// The working directory of process `pid`.
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

// #724: the daemon never keeps the launch directory busy.
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

// #724: a relative `--socket` still binds next to the launch directory.
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

/// #804: `gpy debug prompt` with no agent is an unreachable agent: exit 1 with
/// an `Error:` line on stderr, like every other failed connection.
#[test]
fn debug_prompt_fails_without_agent() {
    let sandbox = AgentSandbox::new();

    let out = sandbox.gpy(&["debug", "prompt"]);
    assert_eq!(out.exit_code, 1_i32, "{out:?}");
    assert!(out.stderr.contains("Error:"), "{out:?}");
    assert!(out.stderr.contains("Agent is not running"), "{out:?}");
}

/// #742: a socket that accepts but never answers is an agent that could not
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

/// #742: a socket file with nothing listening is "not running", not a
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
            "✅ Shutdown command sent",
            "✅ GPY stopped",
            "✅ GPY started",
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

    for command in ["start", "stop", "restart", "status"] {
        let result = env
            .run_gpy_with_env(&[command], &[("PATH", empty.as_str())])
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
            "no Debug-formatted enum (#641): {result:?}"
        );
    }
}

// ---- #656: mutating commands reload the live agent -----------------------

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
        "#656/#573: a live agent must acknowledge the reload: {switched:?}"
    );

    let theme = sandbox.gpy_agent(&["config", "get", "ui.theme"]);
    theme.assert_success("gpy-agent config get ui.theme");
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
