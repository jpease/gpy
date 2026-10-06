//! Shared CLI test harness for `gpy` and `gpy-agent` integration tests.

#![allow(dead_code)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use tempfile::TempDir;

const DEFAULT_TEST_CONFIG_TOML: &str = r#"
[agent]
enabled = true
timeout_seconds = 5
live_updates = true

[agent.supervisor]
enabled = true
check_interval_seconds = 30
max_restart_attempts = 5

[git]
enabled = true
show_upstream = true
timeout_seconds = 10
skip_paths = []

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"

[ui]
show_icons = true
theme = "default"
directory.max_length = 80
enabled_segments = ["clock", "duration", "directory", "git"]
"#;

/// `PATH`, with the directory holding the cargo-built `gpy`/`gpy-agent` test
/// binaries prepended.
///
/// `gpy doctor` (and anything else that shells out to a sibling binary by
/// bare name, e.g. `Command::new("gpy-agent")`) only finds it this way in CI,
/// which has no dev-install step putting it on `PATH`.
fn path_with_cargo_bin_dir() -> std::ffi::OsString {
    let bin_dir = Path::new(env!("CARGO_BIN_EXE_gpy"))
        .parent()
        .expect("CARGO_BIN_EXE_gpy must have a parent directory")
        .to_owned();
    match std::env::var_os("PATH") {
        Some(existing) => {
            std::env::join_paths(std::iter::once(bin_dir).chain(std::env::split_paths(&existing)))
                .expect("failed to join PATH with cargo bin dir")
        }
        None => bin_dir.into_os_string(),
    }
}

#[derive(Debug)]
pub struct CliCommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CliCommandResult {
    fn from_output(output: &Output) -> Self {
        Self {
            exit_code: output.status.code().unwrap_or(-1_i32),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    pub fn assert_success(&self, context: &str) {
        assert_eq!(
            self.exit_code, 0_i32,
            "{context} failed\nexit: {}\nstdout:\n{}\nstderr:\n{}",
            self.exit_code, self.stdout, self.stderr
        );
    }
}

#[derive(Debug)]
pub struct CliTestEnv {
    _temp_dir: TempDir,
    root: PathBuf,
    config_home: PathBuf,
    runtime_dir: PathBuf,
    cache_home: PathBuf,
}

impl CliTestEnv {
    pub fn new() -> io::Result<Self> {
        Self::with_config(DEFAULT_TEST_CONFIG_TOML)
    }

    pub fn with_config(config_toml: &str) -> io::Result<Self> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path().to_path_buf();
        let config_home = root.join(".config");
        let runtime_dir = root.join(".runtime");
        let cache_home = root.join(".cache");
        let env = Self {
            _temp_dir: temp_dir,
            root,
            config_home,
            runtime_dir,
            cache_home,
        };
        env.prepare(config_toml)?;
        Ok(env)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The isolated `XDG_CONFIG_HOME` this environment set up — exposed for
    /// callers that need to pass it to a child process by some means other
    /// than `run_command`/`run_command_with_env` (e.g. a pty-attached spawn).
    pub fn xdg_config_home(&self) -> &Path {
        &self.config_home
    }

    /// The isolated `XDG_RUNTIME_DIR` this environment set up. See
    /// [`Self::xdg_config_home`].
    pub fn xdg_runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// The isolated `XDG_CACHE_HOME` this environment set up. See
    /// [`Self::xdg_config_home`].
    pub fn xdg_cache_home(&self) -> &Path {
        &self.cache_home
    }

    pub fn config_dir(&self) -> PathBuf {
        self.config_home.join("gpy")
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir().join("themes")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.config_dir().join("plugins")
    }

    pub fn write_config(&self, config_toml: &str) -> io::Result<()> {
        fs::write(self.config_path(), config_toml)
    }

    pub fn run_gpy(&self, args: &[&str]) -> io::Result<CliCommandResult> {
        self.run_command(Path::new(env!("CARGO_BIN_EXE_gpy")), args)
    }

    pub fn run_gpy_with_env<K, V>(
        &self,
        args: &[&str],
        envs: &[(K, V)],
    ) -> io::Result<CliCommandResult>
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.run_command_with_env(Path::new(env!("CARGO_BIN_EXE_gpy")), args, envs)
    }

    pub fn run_gpy_agent(&self, args: &[&str]) -> io::Result<CliCommandResult> {
        self.run_command(Path::new(env!("CARGO_BIN_EXE_gpy-agent")), args)
    }

    pub fn run_gpy_agent_with_env<K, V>(
        &self,
        args: &[&str],
        envs: &[(K, V)],
    ) -> io::Result<CliCommandResult>
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.run_command_with_env(Path::new(env!("CARGO_BIN_EXE_gpy-agent")), args, envs)
    }

    /// Run `gpy` feeding `stdin_input` on stdin, for commands with an
    /// interactive `[y/N]` confirmation prompt.
    pub fn run_gpy_with_stdin(
        &self,
        args: &[&str],
        stdin_input: &str,
    ) -> io::Result<CliCommandResult> {
        self.run_command_with_stdin(Path::new(env!("CARGO_BIN_EXE_gpy")), args, stdin_input)
    }

    fn run_command_with_stdin(
        &self,
        program: &Path,
        args: &[&str],
        stdin_input: &str,
    ) -> io::Result<CliCommandResult> {
        use std::io::Write;

        let mut child = Command::new(program)
            .args(args)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("PATH", path_with_cargo_bin_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("failed to open child stdin"))?;
        stdin.write_all(stdin_input.as_bytes())?;
        drop(stdin);

        let output = child.wait_with_output()?;
        Ok(CliCommandResult::from_output(&output))
    }

    fn prepare(&self, config_toml: &str) -> io::Result<()> {
        fs::create_dir_all(self.config_dir())?;
        fs::create_dir_all(self.themes_dir())?;
        fs::create_dir_all(self.plugins_dir())?;
        fs::create_dir_all(&self.runtime_dir)?;
        fs::create_dir_all(&self.cache_home)?;
        self.copy_default_theme()?;
        self.write_config(config_toml)?;
        Ok(())
    }

    fn copy_default_theme(&self) -> io::Result<()> {
        let destination = self.themes_dir().join("default.toml");
        let candidates = [
            Path::new("../config/themes/default.toml"),
            Path::new("config/themes/default.toml"),
            Path::new("../../config/themes/default.toml"),
        ];

        for candidate in candidates {
            if candidate.exists() {
                fs::copy(candidate, &destination)?;
                return Ok(());
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Unable to locate default theme for CLI tests",
        ))
    }

    fn run_command(&self, program: &Path, args: &[&str]) -> io::Result<CliCommandResult> {
        let envs: [(&str, &str); 0] = [];
        self.run_command_with_env(program, args, &envs)
    }

    fn run_command_with_env<K, V>(
        &self,
        program: &Path,
        args: &[&str],
        envs: &[(K, V)],
    ) -> io::Result<CliCommandResult>
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut command = self.command(program);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in envs {
            command.env(key, value);
        }

        let output = command.output()?;
        Ok(CliCommandResult::from_output(&output))
    }

    /// A `gpy-agent` command with this env's isolated `HOME`/`XDG_*`/`PATH`,
    /// for tests that need to spawn concurrently or set a working directory.
    pub fn gpy_agent_command(&self) -> Command {
        self.command(Path::new(env!("CARGO_BIN_EXE_gpy-agent")))
    }

    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("PATH", path_with_cargo_bin_dir());
        command
    }
}

#[derive(Debug)]
pub struct SharedCliTestEnv {
    root: PathBuf,
    config_home: PathBuf,
    runtime_dir: PathBuf,
    cache_home: PathBuf,
}

impl SharedCliTestEnv {
    fn initialize() -> io::Result<Self> {
        let root = std::env::temp_dir().join(format!("gpy-cli-shared-{}", std::process::id()));
        let config_home = root.join(".config");
        let runtime_dir = root.join(".runtime");
        let cache_home = root.join(".cache");
        fs::create_dir_all(config_home.join("gpy/themes"))?;
        fs::create_dir_all(config_home.join("gpy/plugins"))?;
        fs::create_dir_all(&runtime_dir)?;
        fs::create_dir_all(&cache_home)?;

        let temp_like = CliTestEnv {
            _temp_dir: TempDir::new()?,
            root: root.clone(),
            config_home: config_home.clone(),
            runtime_dir: runtime_dir.clone(),
            cache_home: cache_home.clone(),
        };
        temp_like.copy_default_theme()?;
        temp_like.write_config(DEFAULT_TEST_CONFIG_TOML)?;

        Ok(Self {
            root,
            config_home,
            runtime_dir,
            cache_home,
        })
    }

    pub fn instance() -> &'static Self {
        static SHARED: OnceLock<SharedCliTestEnv> = OnceLock::new();
        SHARED.get_or_init(|| {
            Self::initialize().expect("shared CLI test environment should initialize")
        })
    }

    pub fn run_gpy(&self, args: &[&str]) -> io::Result<CliCommandResult> {
        self.run_command(Path::new(env!("CARGO_BIN_EXE_gpy")), args)
    }

    pub fn run_gpy_agent(&self, args: &[&str]) -> io::Result<CliCommandResult> {
        self.run_command(Path::new(env!("CARGO_BIN_EXE_gpy-agent")), args)
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_home.join("gpy").join("config.toml")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.config_home.join("gpy").join("themes")
    }

    fn run_command(&self, program: &Path, args: &[&str]) -> io::Result<CliCommandResult> {
        let output = Command::new(program)
            .args(args)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("PATH", path_with_cargo_bin_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        Ok(CliCommandResult::from_output(&output))
    }
}
