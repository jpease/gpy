//! A `CliTestEnv` plus a private agent socket, for tests that drive the real
//! `gpy` and `gpy-agent` binaries. Socket-dependent, so Unix-only.
#![allow(
    clippy::panic,
    reason = "test helper: failing to become ready fails the test"
)]

use super::cli_harness::{CliCommandResult, CliTestEnv};
use super::fixtures::gpy_test_root;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// How long a freshly started daemon gets to answer `status`.
const READY_DEADLINE: Duration = Duration::from_secs(10);
/// How long a stopped daemon gets to remove its socket.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// The `gpy status` line of a live, answering agent.
pub const RUNNING_LINE: &str = "state: running";
/// The `gpy status` line when no agent runs.
pub const NOT_RUNNING_LINE: &str = "state: not-running";

/// A `CliTestEnv` plus a socket under `gpy_test_root()`, with a drop guard
/// that stops the daemon through its own socket (there is no child handle to
/// kill). A leaked daemon is still reaped by `scripts/cleanup-test-agents.sh`.
pub struct AgentSandbox {
    pub env: CliTestEnv,
    _socket_dir: TempDir,
    pub socket_path: PathBuf,
}

impl AgentSandbox {
    pub fn new() -> Self {
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

    pub fn socket_env(&self) -> [(&'static str, String); 1] {
        [(
            "GPY_AGENT_SOCKET_PATH",
            self.socket_path.to_string_lossy().into_owned(),
        )]
    }

    pub fn gpy(&self, args: &[&str]) -> CliCommandResult {
        self.env
            .run_gpy_with_env(args, &self.socket_env())
            .expect("spawn gpy")
    }

    pub fn gpy_agent(&self, args: &[&str]) -> CliCommandResult {
        self.env
            .run_gpy_agent_with_env(args, &self.socket_env())
            .expect("spawn gpy-agent")
    }

    pub fn start_and_wait(&self) {
        let started = self.gpy(&["start"]);
        started.assert_success("gpy start");
        self.wait_until_responding();
    }

    pub fn wait_until_responding(&self) {
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

    pub fn wait_until_socket_gone(&self) -> bool {
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

/// PIDs of processes whose command line mentions `needle`.
pub fn pids_matching(needle: &Path) -> Vec<String> {
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
pub fn describe_pids(pids: &[String]) -> String {
    let output = std::process::Command::new("ps")
        .args(["-o", "pid,ppid,stat,etime,args", "-p", &pids.join(",")])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sends `SIGTERM` to every daemon started with `--socket <path>`, so a
/// failing run never leaks a duplicate agent the socket cannot reach.
pub struct DaemonReaper(pub PathBuf);

impl Drop for DaemonReaper {
    fn drop(&mut self) {
        for pid in pids_matching(&self.0) {
            let _ = std::process::Command::new("kill").arg(&pid).status();
        }
    }
}
