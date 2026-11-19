//! The per-prompt bookkeeping the shells did after each render (#1247):
//! registering with the agent, telling it the working directory after a
//! `cd`, and restarting an agent that died (the supervisor). Its state lives
//! in `<runtime root>/refresh-throttle/<pid>.session`, which the shell's exit
//! handler sweeps with every `<pid>.*` file; a shell clears it on the
//! agent's `.reregister` flag.

use super::link::{Link, Reply, now_secs};
use crate::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a shell's bookkeeping remembers between prompts.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Session {
    /// The agent accepted this shell's registration.
    registered: bool,
    /// The directory the agent last accepted (`__gpy_last_workspace`).
    workspace: String,
    /// A directory the agent refused; not sent again until `$PWD` changes.
    rejected: String,
    /// Epoch seconds of the last supervisor check.
    checked_at: u64,
    /// Restarts since the agent was last seen up.
    restarts: u32,
}

/// The shell's identity in a registration.
pub(super) struct Shell<'a> {
    pub(super) pid: u32,
    /// `bash`, `zsh` or `fish`.
    pub(super) name: &'static str,
    /// `$BASH_VERSION`, `$ZSH_VERSION` or `$FISH_VERSION`.
    pub(super) version: &'a str,
}

/// Register, sync the workspace and supervise, as the shell's precmd did.
pub(super) fn run(link: &Link, config: &Config, shell: &Shell<'_>, pwd: &str) {
    if !link.enabled() {
        return;
    }
    let Some(path) = session_path(shell.pid) else {
        return;
    };
    let mut session = load(&path).unwrap_or_else(|| Session {
        // The shell started the agent when it loaded: that counts as the
        // first check, so the first probe runs one interval later.
        checked_at: now_secs(),
        ..Session::default()
    });
    sync(link, shell, pwd, &mut session);
    supervise(link, config, &mut session);
    store(&path, &session);
}

/// `__gpy_register_with_agent`, then `__gpy_sync_workspace`.
fn sync(link: &Link, shell: &Shell<'_>, pwd: &str, session: &mut Session) {
    if !link.socket_present() {
        return;
    }
    if session.registered {
        if session.rejected == pwd || session.workspace == pwd {
            return;
        }
        session.rejected.clear();
        match link.send(
            &json!({"op": "workspace", "pid": shell.pid, "cwd": pwd}),
            None,
        ) {
            Reply::Text(_) => {
                pwd.clone_into(&mut session.workspace);
                return;
            }
            // A refused path (e.g. /etc) keeps the registration and is not
            // sent again from here; a lost registration re-registers.
            Reply::Refused(error) if !error.contains("not registered") => {
                pwd.clone_into(&mut session.rejected);
                return;
            }
            _ => session.registered = false,
        }
    }
    let request = json!({
        "op": "register",
        "pid": shell.pid,
        "cwd": pwd,
        "shell": shell.name,
        "shell_version": shell.version,
    });
    if let Reply::Text(reply) = link.send(&request, None)
        && reply.contains(r#""status":"ok""#)
    {
        session.registered = true;
        pwd.clone_into(&mut session.workspace);
        session.rejected.clear();
        track_shell(shell.pid);
    }
}

/// `<runtime root>/shells/<pid>`: lets a restarted agent ring this shell
/// (`__gpy_track_shell_for_agent_recovery`).
fn track_shell(pid: u32) {
    let Ok(root) = crate::paths::runtime_dir() else {
        return;
    };
    let dir = root.join("shells");
    let file = dir.join(pid.to_string());
    if std::fs::write(&file, format!("{pid}\n")).is_err() {
        let _ =
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&file, format!("{pid}\n")));
    }
}

/// `__gpy_supervisor_check`: at most once per check interval, ping the
/// agent; when it is down, start it detached, up to the attempt cap.
fn supervise(link: &Link, config: &Config, session: &mut Session) {
    let enabled =
        super::flag("GPY_AGENT_SUPERVISOR_ENABLED").unwrap_or(config.agent.supervisor.enabled);
    if !enabled {
        return;
    }
    let interval = env_count("GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS")
        .unwrap_or(config.agent.supervisor.check_interval_seconds.get());
    let now = now_secs();
    if now.saturating_sub(session.checked_at) < interval {
        return;
    }
    session.checked_at = now;
    if let Reply::Text(reply) = link.send(&json!({"op": "ping"}), None)
        && reply.contains(r#""status":"ok""#)
    {
        session.restarts = 0;
        return;
    }
    let cap = env_count("GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS")
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(config.agent.supervisor.max_restart_attempts.get());
    if session.restarts >= cap {
        return;
    }
    session.restarts = session.restarts.saturating_add(1);
    session.registered = false;
    start_agent();
}

/// `gpy-agent start` in its own process group, every stream closed, not
/// waited for: the agent daemonises itself and the prompt never waits.
fn start_agent() {
    let mut command = Command::new(crate::commands::utils::agent_binary());
    command
        .arg("start")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::spawn_in_own_process_group(&mut command);
    let _ = command.spawn();
}

/// A non-negative integer from the environment (`0` = every prompt).
fn env_count(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse().ok())
}

fn session_path(pid: u32) -> Option<PathBuf> {
    crate::paths::runtime_dir()
        .ok()
        .map(|root| root.join("refresh-throttle").join(format!("{pid}.session")))
}

fn load(path: &Path) -> Option<Session> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn store(path: &Path, session: &Session) {
    let Ok(bytes) = serde_json::to_vec(session) else {
        return;
    };
    if std::fs::write(path, &bytes).is_err() {
        let _ = path
            .parent()
            .map(std::fs::create_dir_all)
            .and_then(Result::ok)
            .map(|()| std::fs::write(path, &bytes));
    }
}
