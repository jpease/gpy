//! Shared CLI test harness for `gpy` and `gpy-agent` integration tests.

#![allow(
    dead_code,
    reason = "included by several test binaries, each using a subset"
)]

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

/// `PATH` with the cargo-built binaries prepended, so sibling lookups by bare name work in CI.
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

/// The isolated `HOME`/`XDG_*`/`PATH` every spawned command runs under.
#[derive(Debug)]
struct Isolation {
    root: PathBuf,
    config_home: PathBuf,
    runtime_dir: PathBuf,
    cache_home: PathBuf,
}

impl Isolation {
    fn under(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            config_home: root.join(".config"),
            runtime_dir: root.join(".runtime"),
            cache_home: root.join(".cache"),
        }
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

/// Recursively copy `src` to the existing directory `dst` with private-safe
/// modes: dirs `0755`, files `0644` (`0755` when the source is owner-executable).
/// Symlinks are followed.
fn copy_tree_private(src: &Path, dst: &Path) -> io::Result<()> {
    set_mode(dst, 0o755)?;
    for dir_entry in fs::read_dir(src)? {
        let entry = dir_entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let metadata = fs::metadata(&from)?;
        if metadata.is_dir() {
            fs::create_dir_all(&to)?;
            copy_tree_private(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
            set_mode(
                &to,
                if is_owner_executable(&metadata) {
                    0o755
                } else {
                    0o644
                },
            )?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn is_owner_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn is_owner_executable(_metadata: &fs::Metadata) -> bool {
    false
}

/// An isolated environment in a temp directory removed on drop.
#[derive(Debug)]
pub struct CliTestEnv {
    _temp_dir: TempDir,
    isolation: Isolation,
    bundled_plugins: OnceLock<PathBuf>,
}

impl CliTestEnv {
    pub fn new() -> io::Result<Self> {
        Self::with_config(DEFAULT_TEST_CONFIG_TOML)
    }

    pub fn with_config(config_toml: &str) -> io::Result<Self> {
        let temp_dir = TempDir::new()?;
        let env = Self {
            isolation: Isolation::under(temp_dir.path()),
            _temp_dir: temp_dir,
            bundled_plugins: OnceLock::new(),
        };
        env.prepare(config_toml)?;
        Ok(env)
    }

    pub fn root(&self) -> &Path {
        &self.isolation.root
    }

    /// Isolated `XDG_CONFIG_HOME`, for children spawned outside `run_command` (e.g. pty).
    pub fn xdg_config_home(&self) -> &Path {
        &self.isolation.config_home
    }

    /// Isolated `XDG_RUNTIME_DIR`.
    pub fn xdg_runtime_dir(&self) -> &Path {
        &self.isolation.runtime_dir
    }

    /// Isolated `XDG_CACHE_HOME`.
    pub fn xdg_cache_home(&self) -> &Path {
        &self.isolation.cache_home
    }

    pub fn config_dir(&self) -> PathBuf {
        self.isolation.config_home.join("gpy")
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

    /// A private copy of the repo's `plugins` tree (`username`, `hostname`) inside
    /// this env's temp root, for `GPY_BUNDLED_PLUGIN_DIR` where `gpy setup` would
    /// have installed it. The checkout's ancestors may be group/world-writable,
    /// which script-plugin loading rejects; the temp root is `0700`. Created once
    /// per env; the repo is never modified.
    pub fn bundled_plugins_dir(&self) -> PathBuf {
        self.bundled_plugins
            .get_or_init(|| {
                self.private_copy_of(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins"))
            })
            .clone()
    }

    /// Copy the plugin tree at `src` into a fresh directory under this env's
    /// private root (dirs `0755`, files `0644`, or `0755` if the source file is
    /// owner-executable) and return the copy's path.
    #[allow(
        clippy::create_dir,
        reason = "create_dir fails with AlreadyExists, which picks a fresh unique directory"
    )]
    pub fn private_copy_of(&self, src: &Path) -> PathBuf {
        let dst = (0_u32..10_000)
            .map(|n| self.isolation.root.join(format!("private-plugins-{n}")))
            .find(|candidate| fs::create_dir(candidate).is_ok())
            .expect("create private plugin copy dir");
        copy_tree_private(src, &dst).expect("copy plugin tree");
        dst
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

    /// Run `program` (e.g. a relocated `gpy`) with the harness environment plus `envs`.
    pub fn run_program_with_env<K, V>(
        &self,
        program: &Path,
        args: &[&str],
        envs: &[(K, V)],
    ) -> io::Result<CliCommandResult>
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.run_command_with_env(program, args, envs)
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

    /// Runs `gpy` with `stdin_input`, for `[y/N]` prompts.
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

        let mut child = self
            .command(program)
            .args(args)
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
        fs::create_dir_all(&self.isolation.runtime_dir)?;
        fs::create_dir_all(&self.isolation.cache_home)?;
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

    /// A `gpy-agent` command with this env's isolated `HOME`/`XDG_*`/`PATH`.
    pub fn gpy_agent_command(&self) -> Command {
        self.command(Path::new(env!("CARGO_BIN_EXE_gpy-agent")))
    }

    fn command(&self, program: &Path) -> Command {
        self.isolation.command(program)
    }
}

/// `gpy` copied alone into a fresh directory: no sibling `gpy-agent`, so it
/// must find the agent on `PATH`. Copied once per test environment: copying
/// over a binary that already ran rewrites it in place, and macOS then kills
/// the next exec of it (its cached code signature no longer matches).
pub fn lone_gpy(env: &CliTestEnv) -> PathBuf {
    let dir = env.root().join("lone-gpy");
    let gpy = dir.join("gpy");
    if !gpy.exists() {
        fs::create_dir_all(&dir).expect("create lone gpy dir");
        fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).expect("copy gpy");
    }
    gpy
}
