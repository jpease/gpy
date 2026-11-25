//! A sandboxed shell that runs `gpy prompt bash`, and a scripted agent
//! that records every request and answers each as its script says.

use crate::common::agent_sandbox::AgentSandbox;
use crate::common::env_guard::EnvGuard;
use crate::common::wait::wait_until;
use gpy_agent::cache::InstantPromptCache;
use gpy_agent::formatter::PromptDialect;
use serde_json::Value;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// How long a test waits for a request or a cache write to land.
pub const LANDING: Duration = Duration::from_secs(10);

/// What `gpy prompt` may take even when the agent never answers: a few
/// bounded budgets, never a hang.
pub const BOUNDED: Duration = Duration::from_secs(5);

/// One `gpy prompt` run.
pub struct Run {
    pub stdout: String,
    pub elapsed: Duration,
}

/// A `gpy prompt bash` shell in an isolated `HOME`/`XDG_*` tree with a
/// private agent socket path (nothing listens until a test binds it).
pub struct Shell {
    pub sandbox: AgentSandbox,
    pid: u32,
}

impl Shell {
    /// A shell drawing `segments` (TOML array body) with the default theme,
    /// agent mode on and the supervisor off. Its session already exists, so
    /// no `gpy prompt` run starts a real agent behind the test's back.
    pub fn new(segments: &str) -> Self {
        Self::with_theme(segments, "default", None)
    }

    /// [`Self::new`] with theme `theme`, written to the user's themes
    /// directory from `theme_toml` when given.
    pub fn with_theme(segments: &str, theme: &str, theme_toml: Option<&str>) -> Self {
        let sandbox = AgentSandbox::new();
        if let Some(text) = theme_toml {
            std::fs::write(sandbox.env.themes_dir().join(format!("{theme}.toml")), text)
                .expect("write theme");
        }
        sandbox
            .env
            .write_config(&format!(
                "[agent]\nenabled = true\n\n[agent.supervisor]\nenabled = false\n\n\
                 [language]\nshow_versions = false\n\n\
                 [ui]\ntheme = \"{theme}\"\nenabled_segments = [{segments}]\n"
            ))
            .expect("write config");
        let shell = Self {
            sandbox,
            pid: std::process::id(),
        };
        // The first prompt of a shell starts the agent when agent mode is on.
        let root = shell.root().to_path_buf();
        let mut seed = shell.command(&root);
        seed.env("GPY_AGENT_ENABLED", "0");
        let output = seed.output().expect("run the seeding gpy prompt");
        assert!(
            output.status.success(),
            "seeding gpy prompt failed: {output:?}"
        );
        shell
    }

    pub fn root(&self) -> &Path {
        self.sandbox.env.root()
    }

    pub fn socket(&self) -> &Path {
        &self.sandbox.socket_path
    }

    /// `gpy prompt bash` in `dir`, with only the sandbox's environment: no
    /// `GPY_*` knob leaks in from the developer's shell.
    pub fn command(&self, dir: &Path) -> Command {
        let env = &self.sandbox.env;
        let mut command = Command::new(env!("CARGO_BIN_EXE_gpy"));
        command
            .args(["prompt", "bash", "--status", "0", "--pid"])
            .arg(self.pid.to_string())
            .current_dir(dir)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", env.root())
            .env("XDG_CONFIG_HOME", env.xdg_config_home())
            .env("XDG_DATA_HOME", env.root().join(".local/share"))
            .env("XDG_RUNTIME_DIR", env.xdg_runtime_dir())
            .env("XDG_CACHE_HOME", env.xdg_cache_home())
            .env("GPY_AGENT_SOCKET_PATH", self.socket())
            .env("PWD", dir)
            .env("TERM", "xterm-256color")
            .env("GPY_COLOR_DEPTH", "truecolor")
            .env("GPY_IPC_TIMEOUT_MS", "100")
            .env("GPY_IPC_COLD_TIMEOUT_MS", "5000")
            .env("GPY_GIT_INSTANT_CACHE_TTL_SECONDS", "3600")
            .env("GPY_LANGUAGE_CACHE_TTL_SECONDS", "3600");
        if let Some(tmp) = std::env::var_os("TMPDIR") {
            command.env("TMPDIR", tmp);
        }
        command
    }

    /// Run `gpy prompt bash` in `dir` with `envs` on top; it must succeed.
    pub fn prompt(&self, dir: &Path, envs: &[(&str, &str)]) -> Run {
        let mut command = self.command(dir);
        command.envs(envs.iter().copied());
        let started = Instant::now();
        let output = command.output().expect("run gpy prompt");
        let elapsed = started.elapsed();
        assert!(output.status.success(), "gpy prompt failed: {output:?}");
        Run {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            elapsed,
        }
    }

    /// The Bash cache file the prompt reads for `variant` (`git_first_last`,
    /// `lang_last`, ...) of the project keyed by `key_dir`.
    pub fn cache_file(&self, key_dir: &Path, variant: &str, prev_bg: Option<&str>) -> PathBuf {
        let _env = EnvGuard::set_one("XDG_CACHE_HOME", self.sandbox.env.xdg_cache_home());
        InstantPromptCache::cache_file_for_dir(key_dir, variant, prev_bg, PromptDialect::BashPrompt)
            .expect("instant cache path")
    }

    /// The Bash language presence marker for `key_dir`.
    pub fn lang_marker(&self, key_dir: &Path) -> PathBuf {
        let _env = EnvGuard::set_one("XDG_CACHE_HOME", self.sandbox.env.xdg_cache_home());
        InstantPromptCache::lang_marker_for_dir(key_dir, PromptDialect::BashPrompt)
            .expect("presence marker path")
    }

    /// The instant cache root (`<cache>/gpy/instant-prompts`).
    pub fn cache_root(&self) -> PathBuf {
        self.sandbox
            .env
            .xdg_cache_home()
            .join("gpy/instant-prompts")
    }

    /// The agent's flag file `<runtime root>/shells/<pid>.<flag>`.
    pub fn shell_flag(&self, flag: &str) -> PathBuf {
        self.runtime_root()
            .join("shells")
            .join(format!("{}.{flag}", self.pid))
    }

    /// The late window stamp `<runtime root>/refresh-throttle/<pid>.agent-late`.
    pub fn late_window(&self) -> PathBuf {
        self.runtime_root()
            .join("refresh-throttle")
            .join(format!("{}.agent-late", self.pid))
    }

    fn runtime_root(&self) -> PathBuf {
        let _env = EnvGuard::set_one("XDG_RUNTIME_DIR", self.sandbox.env.xdg_runtime_dir());
        gpy_agent::paths::runtime_dir().expect("runtime dir")
    }
}

/// Write `text` to `path`, creating chunk directories as the agent does.
pub fn write_entry(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("cache file has a parent")).expect("mkdir");
    std::fs::write(path, text).expect("write cache entry");
}

/// Set `path`'s mtime a minute back: stale under any TTL below 60 s.
pub fn backdate(path: &Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| {
            let past = SystemTime::now()
                .checked_sub(Duration::from_secs(60))
                .expect("a minute ago is representable");
            file.set_modified(past)
        })
        .expect("backdate cache entry");
}

/// How the scripted agent answers one request.
pub enum Answer {
    /// This line, at once.
    Line(String),
    /// This line, after a delay.
    After(Duration, String),
    /// Nothing: the connection stays open until the agent stops.
    Hold,
}

/// What every connection of a [`ScriptedAgent`] shares.
struct Shared {
    script: Box<dyn Fn(&Value) -> Answer + Send + Sync>,
    requests: Mutex<Vec<Value>>,
    /// Connections answered with [`Answer::Hold`], open until the agent stops.
    held: Mutex<Vec<UnixStream>>,
}

/// A Unix-socket agent that records each request (one JSON line) and
/// answers per `script`. Stopping it removes its socket file.
pub struct ScriptedAgent {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl ScriptedAgent {
    pub fn spawn(socket: &Path, script: impl Fn(&Value) -> Answer + Send + Sync + 'static) -> Self {
        let listener = UnixListener::bind(socket).expect("bind the scripted agent");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            script: Box::new(script),
            requests: Mutex::new(Vec::new()),
            held: Mutex::new(Vec::new()),
        });
        let accept = {
            let stopping = Arc::clone(&stop);
            let state = Arc::clone(&shared);
            std::thread::spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let connection = Arc::clone(&state);
                            std::thread::spawn(move || serve(stream, &connection));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // Idle accept loop on a non-blocking listener.
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            socket: socket.to_path_buf(),
            stop,
            accept: Some(accept),
            shared,
        }
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<Value> {
        self.shared.requests.lock().expect("requests lock").clone()
    }

    /// The requests whose `op` is `op`.
    pub fn with_op(&self, op: &str) -> Vec<Value> {
        self.requests()
            .into_iter()
            .filter(|request| field(request, "op") == op)
            .collect()
    }

    /// `(op, cwd)` of every request so far.
    pub fn ops(&self) -> Vec<(String, String)> {
        self.requests()
            .iter()
            .map(|request| {
                (
                    field(request, "op").as_str().unwrap_or_default().to_owned(),
                    field(request, "cwd")
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect()
    }

    /// Wait until a request with `op` arrived; it may come from a
    /// fire-and-forget refresh the prompt did not wait for.
    pub fn wait_for_op(&self, op: &str) -> Vec<Value> {
        let arrived = wait_until(LANDING, Duration::from_millis(20), || {
            !self.with_op(op).is_empty()
        });
        assert!(
            arrived,
            "no {op} request within {LANDING:?}; got {:?}",
            self.requests()
        );
        self.with_op(op)
    }
}

/// `request[key]`, `null` when absent.
pub fn field<'a>(request: &'a Value, key: &str) -> &'a Value {
    request.get(key).unwrap_or(&Value::Null)
}

/// Read one request line, record it, answer it.
fn serve(mut stream: UnixStream, shared: &Shared) {
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }
    let request: Value = serde_json::from_str(line.trim_end()).unwrap_or(Value::String(line));
    shared
        .requests
        .lock()
        .expect("requests lock")
        .push(request.clone());
    let reply = match (shared.script)(&request) {
        Answer::Line(reply) => reply,
        Answer::After(delay, reply) => {
            // The reply delay is the behaviour under test.
            std::thread::sleep(delay);
            reply
        }
        Answer::Hold => {
            shared.held.lock().expect("held lock").push(stream);
            return;
        }
    };
    // A fire-and-forget client has hung up already.
    let _ = stream.write_all(format!("{reply}\n").as_bytes());
}

impl Drop for ScriptedAgent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        self.shared.held.lock().map(|mut held| held.clear()).ok();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The agent's ack, as `register` and `workspace` get it.
pub fn ack() -> Answer {
    Answer::Line(r#"{"status":"ok"}"#.to_owned())
}
