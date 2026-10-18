//! Real-daemon checks that every Git entry point — direct requests and
//! background work (reconcile ticks, registration scans) — honors
//! `git.enabled` and `git.skip_paths`, including skip roots that appear or
//! change while the agent runs.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

mod common;

use common::{AgentStopGuard, CliTestEnv, KillOnDrop, git, wait_until};
use gpy_agent::ipc::Response;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Reconcile tick period for these daemons, in seconds.
const RECONCILE_INTERVAL_SECS: u64 = 1;
/// Long enough for several reconcile ticks to run.
const SEVERAL_TICKS: Duration = Duration::from_secs(3 * RECONCILE_INTERVAL_SECS);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// A daemonized agent on a private socket, stopped on drop.
struct Daemon {
    socket: PathBuf,
    _stop: AgentStopGuard,
}

impl Daemon {
    fn start(env: &CliTestEnv) -> Self {
        let socket = env.root().join("s");
        // Armed before `start` so a half-started daemon is still stopped.
        let stop = AgentStopGuard::new(env.gpy_agent_command(), socket.clone());
        let started = env
            .gpy_agent_command()
            .env(
                "GPY_RECONCILE_INTERVAL_SECS",
                RECONCILE_INTERVAL_SECS.to_string(),
            )
            .args(["start", "--socket"])
            .arg(&socket)
            .output()
            .expect("run gpy-agent start");
        assert!(
            started.status.success(),
            "agent failed to start: {}",
            String::from_utf8_lossy(&started.stderr)
        );
        assert!(
            wait_until(WAIT_TIMEOUT, POLL_INTERVAL, || UnixStream::connect(&socket)
                .is_ok()),
            "agent never accepted a connection on {}",
            socket.display()
        );
        Self {
            socket,
            _stop: stop,
        }
    }

    fn request(&self, request: &serde_json::Value) -> serde_json::Value {
        let mut stream = UnixStream::connect(&self.socket).expect("connect to agent");
        stream
            .set_read_timeout(Some(WAIT_TIMEOUT))
            .expect("set read timeout");
        writeln!(stream, "{request}").expect("send request");
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .expect("read response");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad response {line:?}: {e}"))
    }

    /// Register a live disposable process as a client in `cwd`.
    fn register_client(&self, cwd: &Path) -> KillOnDrop {
        let client = KillOnDrop::new(
            Command::new("sleep")
                .arg("600")
                .stdin(Stdio::null())
                .spawn()
                .expect("spawn disposable client"),
        );
        let response = self.request(&serde_json::json!({
            "op": "register",
            "pid": client.id(),
            "cwd": cwd,
        }));
        assert!(
            response.get("error").is_none(),
            "registration failed: {response}"
        );
        client
    }

    /// Whether the agent reports Git disabled for `repo`, as opposed to a status reply.
    fn git_skipped(&self, repo: &Path) -> bool {
        let reply = self.request(&serde_json::json!({
            "op": "git",
            "cwd": repo,
            "format": "json",
        }));
        match reply.get("error").and_then(serde_json::Value::as_str) {
            Some(message) if message.contains("disabled") => true,
            None if reply.get("branch").is_some() => false,
            _ => panic!("unexpected git reply: {reply}"),
        }
    }

    /// `(watched_repos, git cache entries)` from the agent's status report.
    fn watched_and_cached(&self) -> (usize, usize) {
        let status = self.request(&serde_json::json!({ "op": "status" }));
        match serde_json::from_value(status.clone()) {
            Ok(Response::AgentStatus {
                watched_repos,
                cache_entries,
                ..
            }) => (watched_repos, cache_entries),
            _ => panic!("unexpected status response: {status}"),
        }
    }
}

/// A committed repo named `name` under the env root; returns its canonical path.
fn create_repo(env: &CliTestEnv, name: &str) -> PathBuf {
    let repo = env.root().join(name);
    std::fs::create_dir_all(&repo).expect("create repo dir");
    git::git_init(&repo);
    std::fs::write(repo.join("README.md"), "# test\n").expect("write file");
    git::git(&repo, &["add", "."]);
    git::git(&repo, &["commit", "-q", "-m", "initial"]);
    std::fs::canonicalize(&repo).expect("canonical repo")
}

/// Git instant-prompt cache files for the repo whose final component is
/// `repo_name`. Long keys are chunked into directories, so the relative path
/// is rejoined before matching `<repo_name>.git<variant>`.
fn git_cache_files(env: &CliTestEnv, repo_name: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let root = env.xdg_cache_home();
    let mut files = Vec::new();
    walk(root, &mut files);
    let needle = format!("{repo_name}.git");
    files.retain(|file| {
        file.strip_prefix(root)
            .is_ok_and(|rel| rel.to_string_lossy().replace('/', "").contains(&needle))
    });
    files
}

fn write_config(env: &CliTestEnv, git_section: &str) {
    env.write_config(&format!(
        "[agent]\nlive_updates = true\n\n[git]\n{git_section}\n"
    ))
    .expect("write config");
}

#[test]
fn reconcile_never_scans_or_publishes_a_skipped_repo() {
    let env = CliTestEnv::new().expect("cli env");
    let skipped = create_repo(&env, "skippedrepo");
    let control = create_repo(&env, "controlrepo");
    write_config(&env, &format!("skip_paths = [\"{}\"]", skipped.display()));

    let daemon = Daemon::start(&env);
    let _skipped_client = daemon.register_client(&skipped);
    let _control_client = daemon.register_client(&control);

    assert!(
        wait_until(WAIT_TIMEOUT, POLL_INTERVAL, || !git_cache_files(
            &env,
            "controlrepo"
        )
        .is_empty()),
        "control: the registration scan must publish the enabled repo's git cache"
    );
    // Only a background refresh (reconcile) can bring these back.
    for file in git_cache_files(&env, "controlrepo") {
        std::fs::remove_file(file).expect("remove control cache file");
    }
    assert!(
        wait_until(WAIT_TIMEOUT, POLL_INTERVAL, || !git_cache_files(
            &env,
            "controlrepo"
        )
        .is_empty()),
        "control: reconcile must keep refreshing the enabled repo"
    );
    std::thread::sleep(SEVERAL_TICKS);

    assert_eq!(
        git_cache_files(&env, "skippedrepo"),
        Vec::<PathBuf>::new(),
        "a skipped repo must never get git instant-cache files"
    );
    assert_eq!(
        daemon.watched_and_cached(),
        (2, 1),
        "both repos are watched, but only the enabled one may be scanned"
    );
}

#[test]
fn reconcile_never_scans_or_publishes_when_git_disabled() {
    let env = CliTestEnv::new().expect("cli env");
    let repo = create_repo(&env, "disabledrepo");
    write_config(&env, "enabled = false");

    let daemon = Daemon::start(&env);
    let _client = daemon.register_client(&repo);
    std::thread::sleep(SEVERAL_TICKS);

    assert_eq!(
        git_cache_files(&env, "disabledrepo"),
        Vec::<PathBuf>::new(),
        "git.enabled = false must keep reconcile from publishing git caches"
    );
    assert_eq!(
        daemon.watched_and_cached(),
        (1, 0),
        "the repo is watched (for language), but git must never scan it"
    );
}

/// A skip root created after the agent first resolved `skip_paths` takes
/// effect without a restart and agrees with a freshly started agent; moving
/// the symlink elsewhere stops it matching the old target.
#[test]
fn skip_root_created_or_retargeted_while_running_takes_effect() {
    let env = CliTestEnv::new().expect("cli env");
    let target = create_repo(&env, "targetrepo");
    let elsewhere = env.root().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("create retarget dir");
    let link = env.root().join("skiplink");
    write_config(&env, &format!("skip_paths = [\"{}\"]", link.display()));

    let daemon = Daemon::start(&env);
    assert!(
        !daemon.git_skipped(&target),
        "the skip root does not exist yet"
    );

    std::os::unix::fs::symlink(&target, &link).expect("create skip root");
    assert!(
        wait_until(WAIT_TIMEOUT, POLL_INTERVAL, || daemon.git_skipped(&target)),
        "a skip root created after first use must take effect without a restart"
    );

    drop(daemon);
    let daemon = Daemon::start(&env);
    assert!(
        daemon.git_skipped(&target),
        "control: a fresh agent with identical config skips the repo too"
    );

    std::fs::remove_file(&link).expect("remove skip root");
    std::os::unix::fs::symlink(&elsewhere, &link).expect("retarget skip root");
    assert!(
        wait_until(WAIT_TIMEOUT, POLL_INTERVAL, || !daemon.git_skipped(&target)),
        "a retargeted skip root must stop matching its old target"
    );
}

/// `skip_paths` is matched against the repository root: an entry at the root
/// skips the repo for direct requests from a subdirectory and for background
/// work alike, while an entry below the root skips neither.
#[test]
fn skip_paths_matches_the_repository_root() {
    let env = CliTestEnv::new().expect("cli env");
    let root_entry = create_repo(&env, "rootentry");
    let sub_entry = create_repo(&env, "subentry");
    for repo in [&root_entry, &sub_entry] {
        std::fs::create_dir(repo.join("sub")).expect("create subdirectory");
    }
    write_config(
        &env,
        &format!(
            "skip_paths = [\"{}\", \"{}\"]",
            root_entry.display(),
            sub_entry.join("sub").display()
        ),
    );

    let daemon = Daemon::start(&env);
    let _root_client = daemon.register_client(&root_entry.join("sub"));
    let _sub_client = daemon.register_client(&sub_entry.join("sub"));

    assert!(
        wait_until(WAIT_TIMEOUT, POLL_INTERVAL, || !git_cache_files(
            &env, "subentry"
        )
        .is_empty()),
        "background work must scan a repo whose skip entry lies below its root"
    );
    assert!(
        !daemon.git_skipped(&sub_entry.join("sub")),
        "a direct request from inside a skip entry below the root must not be skipped"
    );
    assert!(
        daemon.git_skipped(&root_entry.join("sub")),
        "a direct request from a subdirectory of a skipped root must be skipped"
    );
    std::thread::sleep(SEVERAL_TICKS);
    assert_eq!(
        git_cache_files(&env, "rootentry"),
        Vec::<PathBuf>::new(),
        "background work must never publish a repo whose root is skipped"
    );
}
