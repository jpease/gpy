//! A shell's lifecycle, kept by `gpy prompt` so the shell keeps none
//! (#1247): the agent's `.reload` and `.reregister` flags, starting the agent
//! with the shell's first prompt, registering, telling the agent the working
//! directory after a `cd`, restarting an agent that died (the supervisor),
//! the Bash that runs script segments, and unregistering at exit. Its state
//! lives in `<runtime root>/refresh-throttle/<pid>.session`; `gpy prompt
//! <shell> --exit` removes it with every other `<pid>.*` file.

use super::link::{Link, Reply, now_secs};
use crate::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a shell's lifecycle remembers between prompts.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// The agent accepted this shell's registration.
    registered: bool,
    /// The directory the agent last accepted.
    workspace: String,
    /// A directory the agent refused; not sent again until `$PWD` changes.
    rejected: String,
    /// Epoch seconds of the last supervisor check.
    checked_at: u64,
    /// Restarts since the agent was last seen up.
    restarts: u32,
    /// The last command's duration, for scripts' `cmd_duration` input on a
    /// prompt that ran no command (a `cd` alone).
    last_duration_ms: Option<u64>,
    /// The Bash that runs script segments, once resolved; `Some("")` when
    /// none was found.
    bash: Option<PathBuf>,
    /// The `gpy` version that last checked this shell's installed tree; a new
    /// binary checks once more after an upgrade.
    #[serde(default)]
    trees_checked_for: String,
    /// The `gpy` version that last asked an agent of another version to make
    /// way, so a newer agent it cannot replace is asked once, not every check.
    #[serde(default)]
    agent_nudged_for: String,
}

/// The shell's identity in a registration.
pub(super) struct Shell<'a> {
    pub(super) pid: u32,
    /// `bash`, `zsh` or `fish`.
    pub(super) name: &'static str,
    /// `$BASH_VERSION`, `$ZSH_VERSION` or `$FISH_VERSION`.
    pub(super) version: &'a str,
}

/// One prompt's view of the shell's lifecycle: loaded before the render,
/// stored after it.
pub(super) struct Session {
    path: Option<PathBuf>,
    state: State,
    /// The shell's first prompt, or the agent asked for a reload: script
    /// segments are due and their circuit breakers reset.
    pub(super) reload: bool,
}

impl Session {
    /// Load the state and take the agent's flags for `pid`. On the shell's
    /// first prompt, start the agent (when enabled): that counts as the
    /// first supervisor check.
    pub(super) fn open(pid: u32, agent_enabled: bool) -> Self {
        let runtime = crate::paths::runtime_dir().ok();
        let path = runtime
            .as_deref()
            .map(|root| root.join("refresh-throttle").join(format!("{pid}.session")));
        let loaded = path.as_deref().and_then(load);
        let first = loaded.is_none();
        let mut session = Self {
            path,
            state: loaded.unwrap_or_else(|| State {
                checked_at: now_secs(),
                ..State::default()
            }),
            reload: first,
        };
        if let Some(root) = runtime {
            let flags = root.join("shells").join(pid.to_string());
            let reregister = take(&flags.with_extension("reregister"));
            if reregister {
                session.state.registered = false;
                session.state.workspace.clear();
            }
            let reload = take(&flags.with_extension("reload"));
            session.reload |= reload;
            // The agent wrote the flag, so it is answering again.
            if reregister || reload {
                super::link::clear_late_window(&root.join("refresh-throttle"), pid);
            }
        }
        if first && agent_enabled {
            start_agent();
        }
        session
    }

    /// The duration scripts see: this prompt's command's, else the last one's.
    pub(super) const fn script_duration(&mut self, duration_ms: Option<u64>) -> Option<u64> {
        if duration_ms.is_some() {
            self.state.last_duration_ms = duration_ms;
        }
        self.state.last_duration_ms
    }

    /// The first Bash 4 or later on `PATH`, else `/bin/bash`; resolved once
    /// per shell. `None` when there is no Bash at all.
    pub(super) fn bash(&mut self) -> Option<PathBuf> {
        let resolved = self.state.bash.get_or_insert_with(resolve_bash);
        (!resolved.as_os_str().is_empty()).then(|| resolved.clone())
    }

    /// Register, sync the workspace and supervise, then store the state.
    pub(super) fn finish(mut self, link: &Link, config: &Config, shell: &Shell<'_>, pwd: &str) {
        if link.enabled() {
            sync(link, shell, pwd, &mut self.state);
            supervise(link, config, &mut self.state);
        }
        refresh_trees(shell.name, &mut self.state);
        if let Some(path) = &self.path {
            store(path, &self.state);
        }
    }
}

/// The shell is exiting: unregister it and remove every file kept for it.
pub(super) fn exit(link: &Link, pid: u32) {
    if link.enabled() && link.socket_present() {
        let _ = link.send(&json!({"op": "unregister", "pid": pid}), None);
    }
    let Ok(root) = crate::paths::runtime_dir() else {
        return;
    };
    let tracked = root.join("shells").join(pid.to_string());
    for file in [
        tracked.with_extension("reload"),
        tracked.with_extension("reregister"),
        tracked,
    ] {
        let _ = std::fs::remove_file(file);
    }
    let prefix = format!("{pid}.");
    if let Ok(entries) = std::fs::read_dir(root.join("refresh-throttle")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Remove a flag file; whether it was there.
fn take(flag: &Path) -> bool {
    std::fs::remove_file(flag).is_ok()
}

fn resolve_bash() -> PathBuf {
    let on_path = std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("bash"))
        .find(|bash| {
            bash.is_file()
                && Command::new(bash)
                    .args(["-p", "-c", r#"[ "${BASH_VERSINFO[0]}" -ge 4 ]"#])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
        });
    on_path.unwrap_or_else(|| {
        let fallback = PathBuf::from("/bin/bash");
        if fallback.is_file() {
            fallback
        } else {
            if super::flag("GPY_DEBUG") == Some(true) {
                eprintln!("gpy[script]: warning: No bash found - script segments disabled");
            }
            PathBuf::new()
        }
    })
}

/// Register when not registered, else tell the agent about a new `$PWD`.
fn sync(link: &Link, shell: &Shell<'_>, pwd: &str, state: &mut State) {
    if !link.socket_present() {
        return;
    }
    if state.registered {
        if state.rejected == pwd || state.workspace == pwd {
            return;
        }
        state.rejected.clear();
        match link.send(
            &json!({"op": "workspace", "pid": shell.pid, "cwd": pwd}),
            None,
        ) {
            Reply::Text(_) => {
                pwd.clone_into(&mut state.workspace);
                return;
            }
            // A refused path (e.g. /etc) keeps the registration and is not
            // sent again from here; a lost registration re-registers.
            Reply::Refused(error) if !error.contains("not registered") => {
                pwd.clone_into(&mut state.rejected);
                return;
            }
            _ => state.registered = false,
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
        state.registered = true;
        pwd.clone_into(&mut state.workspace);
        state.rejected.clear();
        track_shell(shell.pid);
    }
}

/// `<runtime root>/shells/<pid>`: lets a restarted agent ring this shell.
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

/// At most once per check interval, ask the agent its status; when it is
/// down, start it detached, up to the attempt cap. One of another version
/// (left running across an upgrade) gets `gpy-agent start`, which replaces an
/// older one and keeps a newer one, once per `gpy` version.
fn supervise(link: &Link, config: &Config, state: &mut State) {
    let enabled =
        super::flag("GPY_AGENT_SUPERVISOR_ENABLED").unwrap_or(config.agent.supervisor.enabled);
    if !enabled {
        return;
    }
    let interval = env_count("GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS")
        .unwrap_or(config.agent.supervisor.check_interval_seconds.get());
    let now = now_secs();
    if now.saturating_sub(state.checked_at) < interval {
        return;
    }
    state.checked_at = now;
    if let Reply::Text(reply) = link.send(&json!({"op": "status"}), None)
        && (reply.contains(r#""AgentStatus""#) || reply.contains(r#""status":"ok""#))
    {
        state.restarts = 0;
        // Only a status reply names a version; an agent that answers is up.
        let current = format!(r#""version":"{}""#, crate::VERSION);
        if reply.contains(r#""AgentStatus""#)
            && !reply.contains(&current)
            && state.agent_nudged_for != crate::VERSION
        {
            crate::VERSION.clone_into(&mut state.agent_nudged_for);
            start_agent();
        }
        return;
    }
    let cap = env_count("GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS")
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(config.agent.supervisor.max_restart_attempts.get());
    if state.restarts >= cap {
        return;
    }
    state.restarts = state.restarts.saturating_add(1);
    state.registered = false;
    start_agent();
}

/// Once per `gpy` version: when this shell's tree under the data root was
/// installed by `gpy setup` for another version (an upgrade that ran no
/// setup, as Homebrew's), reinstall the stale trees with a detached
/// `gpy setup --refresh-stale`. No rc file is touched; the next shell started
/// sources the new files.
fn refresh_trees(name: &str, state: &mut State) {
    if state.trees_checked_for == crate::VERSION {
        return;
    }
    crate::VERSION.clone_into(&mut state.trees_checked_for);
    let shell = match name {
        "bash" => crate::shell::Shell::Bash,
        "zsh" => crate::shell::Shell::Zsh,
        "fish" => crate::shell::Shell::Fish,
        _ => return,
    };
    if !crate::commands::setup::tree_is_stale(shell) {
        return;
    }
    let Ok(gpy) = std::env::current_exe() else {
        return;
    };
    let mut command = Command::new(gpy);
    command
        .args(["setup", "--refresh-stale"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::spawn_in_own_process_group(&mut command);
    let _ = command.spawn();
}

/// `gpy-agent start` in its own process group, every stream closed, not
/// waited for: the agent daemonises itself and the prompt never waits. It
/// is idempotent: it keeps a healthy agent and replaces one left running
/// across an upgrade.
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

fn load(path: &Path) -> Option<State> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn store(path: &Path, state: &State) {
    let Ok(bytes) = serde_json::to_vec(state) else {
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
