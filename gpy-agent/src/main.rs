//! # GPY Agent - Background process for Git status and language detection
//!
//! Provides fast, cached responses for Bash, Fish, and Zsh prompt segments
//! through a background agent process that communicates via Unix domain
//! sockets.
//!
//! ## Usage
//!
//! ```sh
//! # Start the agent in the background
//! gpy-agent start
//!
//! # Get the status of the agent
//! gpy-agent status
//!
//! # Stop the agent
//! gpy-agent stop
//!
//! # Run a one-off command without the agent
//! gpy-agent oneshot git --cwd /path/to/repo
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![expect(
    clippy::print_stdout,
    reason = "this binary's whole purpose is to print CLI command output to stdout for the shell prompt/user"
)]
#![expect(
    clippy::print_stderr,
    reason = "this binary reports CLI errors and diagnostics on stderr, the conventional channel for a command-line tool"
)]
#![expect(
    clippy::multiple_crate_versions,
    reason = "multiple versions are common in large dependency trees; managed by cargo-deny"
)]

use clap::{Parser, Subcommand};
use gpy_agent::commands::config::list;
use gpy_agent::{
    Result, VERSION,
    agent::{
        Agent,
        lifecycle::SOCKET_OVERRIDE,
        lifecycle::start::start_background_agent,
        oneshot::{OneshotKind, OneshotRequest},
    },
    config::{self, schema},
};
use std::path::PathBuf;
use std::process::ExitCode;

/// GPY Agent - A high-performance background agent for the GPY prompt (Bash, Fish, and Zsh).
///
/// This struct defines the command-line interface for the `gpy-agent` binary.
/// It uses `clap` for parsing arguments and subcommands.
#[derive(Parser, Debug)]
#[command(author, version = VERSION, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Defines the main subcommands for the `gpy-agent` CLI.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the agent as a background process.
    ///
    /// This command forks the agent into the background and sets up a Unix domain socket
    /// for communication with shell clients.
    Start {
        /// Override the socket path used by the agent (primarily for testing).
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Stop the background agent process.
    ///
    /// Sends a shutdown signal to the running agent process, causing it to exit gracefully.
    Stop {
        /// Override the socket path used by the agent (must match the path passed to start).
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Show the current status of the agent.
    ///
    /// Pings the agent to check if it is running and responsive, and displays
    /// information such as version, uptime, and configuration.
    Status {
        /// Override the socket path used by the agent (must match the path passed to start).
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Theme management commands.
    ///
    /// Export theme configuration for use in shell environments.
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Palette (color theme) management commands.
    Palette {
        #[command(subcommand)]
        command: PaletteCommands,
    },
    /// Run a single command and print the result (used as a fallback).
    ///
    /// This is useful for environments where a background agent is not desired,
    /// or for diagnostic purposes.
    Oneshot {
        #[command(subcommand)]
        command: OneshotCommands,
    },
    /// Query configuration values from config.toml (used by shell integrations).
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Run system diagnostics to check for common issues.
    Doctor,
    /// Bootstrap configuration on first install (picks a Nerd/ASCII icon default
    /// that renders on this machine). No-op when a config file already exists.
    Init {
        /// Skip the interactive confirmation and use detected font capability
        /// directly (for piped/non-interactive installs and CI).
        #[arg(long)]
        non_interactive: bool,
        /// Overwrite an existing config file instead of leaving it untouched.
        #[arg(long)]
        force: bool,
    },
}

/// Defines the subcommands for the `theme` command.
#[derive(Subcommand, Debug)]
enum ThemeCommands {
    /// Export theme as Fish or Zsh shell variable assignments.
    ///
    /// This command loads the configured theme and outputs it as shell
    /// `set` or `typeset` commands that can be sourced to load theme variables.
    Export {
        /// Output format ('fish', 'zsh', or 'bash').
        ///
        /// `ignore_case` preserves the case-insensitive matching the previous
        /// hand-rolled `Shell::from_str` parse accepted (e.g. `--format FISH`).
        #[arg(long, ignore_case = true, default_value_t = gpy_agent::shell::Shell::Fish)]
        format: gpy_agent::shell::Shell,
    },
    /// Validate a theme file or the currently active theme.
    Validate {
        /// Optional path to a theme file to validate. If omitted, validates the active theme.
        path: Option<String>,
    },
    /// Import a `starship.toml` into a GPY palette + prompt theme.
    Import {
        /// Path to the source `starship.toml`.
        path: String,
        /// Base name for the emitted artifacts (default: file stem).
        #[arg(long)]
        name: Option<String>,
        /// Overwrite existing artifacts of the same name.
        #[arg(long)]
        force: bool,
        /// Print both artifacts instead of writing files.
        #[arg(long)]
        stdout: bool,
        /// Also write the derived `enabled_segments` into the active config.
        #[arg(long)]
        apply_layout: bool,
    },
}

/// Defines the subcommands for the `palette` command.
#[derive(Subcommand, Debug)]
enum PaletteCommands {
    /// List available palettes.
    List,
    /// Switch the active palette.
    Use {
        /// Palette name to activate.
        name: String,
    },
    /// Show the current palette name.
    Show,
    /// Validate a palette file or the currently active palette.
    Validate {
        /// Optional palette name or path. If omitted, validates the active palette.
        target: Option<String>,
    },
}

/// Defines the subcommands for the `oneshot` command.
#[derive(Subcommand, Debug)]
enum OneshotCommands {
    /// Get Git repository status.
    ///
    /// This command provides a single, immediate Git status report for a given directory.
    Git {
        /// The directory to get Git status for (defaults to current directory).
        #[arg(long, short, default_value_t = String::from("."))]
        cwd: String,
        /// Output format.
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last (omit to mark as last - default for standalone use).
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap). Omit to mark as not first - default for standalone use.
        #[arg(long)]
        first: bool,
        /// Use the benchmark fast path for JSON output.
        ///
        /// This intentionally bypasses config/theme file I/O so benchmark runs measure
        /// the git-status request path rather than user-specific configuration loading.
        /// Do not use this for user-facing diagnostics or behavior checks.
        #[arg(long)]
        benchmark_mode: bool,
    },
    /// Detect programming languages in a directory.
    ///
    /// This command performs language detection for a given directory and returns the results.
    Lang {
        /// The directory to detect languages in (defaults to current directory).
        #[arg(long, short, default_value_t = String::from("."))]
        cwd: String,
        /// Output format (see possible values).
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last (omit to mark as last - default for standalone use).
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap). Omit to mark as not first - default for standalone use.
        #[arg(long)]
        first: bool,
    },
    /// Render the directory segment for a path (when a format template is configured).
    Directory {
        /// The directory to render (defaults to current directory).
        #[arg(long, short, default_value_t = String::from("."))]
        cwd: String,
        /// Output format (see possible values).
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last.
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap).
        #[arg(long)]
        first: bool,
    },
    /// Render the command-duration segment (when a format template is configured).
    Duration {
        /// Command duration in milliseconds (from `$CMD_DURATION`).
        #[arg(long, default_value_t = 0_u64)]
        duration_ms: u64,
        /// Output format (see possible values).
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last.
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap).
        #[arg(long)]
        first: bool,
    },
    /// Render the character segment (❯ prompt symbol, colored by exit status).
    Character {
        /// Last command exit code (0 = success, non-zero = error).
        #[arg(long, default_value_t = 0_i32)]
        exit_code: i32,
        /// Output format (see possible values).
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last.
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap).
        #[arg(long)]
        first: bool,
    },
    /// Render the hostname segment (when a format template is configured).
    ///
    /// Per #259 the caller resolves and passes the hostname string and gates
    /// SSH-only visibility; the agent never derives either. `--is-ssh` reports
    /// the caller's SSH state so the theme icon renders only over SSH (#826).
    Hostname {
        /// The hostname string to render (defaults to the machine hostname,
        /// or empty when that cannot be determined).
        #[arg(long, default_value_t = default_hostname_arg())]
        hostname: String,
        /// Output format (see possible values).
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last.
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap).
        #[arg(long)]
        first: bool,
        /// Mark the session as SSH, so the theme icon (Starship's
        /// `ssh_symbol`) renders.
        #[arg(long)]
        is_ssh: bool,
    },
    /// Render the username segment (when a format template is configured).
    ///
    /// Per #252 the caller resolves and passes the effective username string;
    /// the agent never derives it or gates on root/sudo state itself.
    Username {
        /// The username string to render (defaults to `$USER`/`$USERNAME`,
        /// or empty when that cannot be determined).
        #[arg(long, default_value_t = default_username_arg())]
        username: String,
        /// Output format (see possible values).
        #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
        format: gpy_agent::formatter::Format,
        /// Mark segment as NOT the last.
        #[arg(long)]
        not_last: bool,
        /// Mark segment as the first in the prompt (suppresses the opening
        /// powerline cap).
        #[arg(long)]
        first: bool,
    },
}

/// Best-effort effective username for the `oneshot username` default.
///
/// Ad-hoc/diagnostic invocations without `--username` still render something
/// sensible; the parity harness and shell segments always pass `--username`
/// explicitly, so this default is not depended on for correctness. Falls back
/// to an empty string when no environment-provided username is available.
fn default_username_arg() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// Best-effort machine hostname for the `oneshot hostname` default.
///
/// Ad-hoc/diagnostic invocations without `--hostname` still render something
/// sensible; the parity harness and shell segments always pass `--hostname`
/// explicitly, so this default is not depended on for correctness. Falls back
/// to an empty string when no environment-provided hostname is available.
fn default_hostname_arg() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default()
}

/// Defines subcommands for `config` command.
#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Print a configuration value.
    Get {
        /// Configuration key using dot-notation (e.g. `agent.live_updates`).
        key: String,
    },
    /// Request the running agent to reload configuration from disk.
    Reload,
    /// List all available config keys
    List {
        /// Show only keys matching this prefix
        #[arg(short, long)]
        filter: Option<String>,
    },
}

/// Install panic handler to capture and log panics for debugging
fn install_panic_handler() {
    std::panic::set_hook(Box::new(|panic_info| {
        // A CLI subcommand whose stdout reader went away (`gpy-agent theme
        // export | head -1`) hits `println!`'s "failed printing to stdout:
        // Broken pipe" panic. That is a normal end for a pipeline, not a
        // failure, and this crate forbids `unsafe` so SIGPIPE cannot be reset
        // to its default; exit 0 quietly instead of reporting a panic (#641).
        // The daemon never prints to stdout after daemonising (it is
        // /dev/null), so this can only trigger in the CLI paths.
        let message = panic_info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic_info.payload().downcast_ref::<&str>().copied())
            .unwrap_or_default();
        if message.contains("failed printing to stdout") && message.contains("Broken pipe") {
            #[expect(
                clippy::exit,
                reason = "a closed stdout reader is a normal end, not a failure"
            )]
            std::process::exit(0);
        }
        // Once daemonized, stdin/stdout/stderr are redirected to /dev/null
        // (see lifecycle/start.rs), so a bare eprintln! here would vanish for
        // a background-agent panic. `warn_log!` always writes to stderr AND
        // mirrors into the file-backed debug log when GPY_DEBUG_LOG is
        // configured, so the panic stays recoverable either way (#323).
        gpy_agent::warn_log!("agent", "PANIC: {:?}", panic_info);
    }));
}

/// Record `--socket`, made absolute against the current directory so a
/// forked daemon (which runs in `/`, #724) binds the path the user meant.
fn apply_socket_override(socket: Option<PathBuf>) {
    if let Some(given) = socket {
        let path = gpy_agent::paths::absolutize(&given);
        match SOCKET_OVERRIDE.set(path.clone()) {
            Ok(()) => {}
            Err(existing) => {
                debug_assert!(
                    existing.as_path() == path.as_path(),
                    "socket override mismatch: existing={}, new={}",
                    existing.display(),
                    path.display()
                );
            }
        }
    }
}

/// The prompt-position flags every `oneshot` subcommand accepts, kept together
/// so the shared request builder does not take a pair of bare `bool`s.
#[derive(Debug, Clone, Copy)]
struct PromptPosition {
    /// Set when the segment is *not* the last one in the prompt.
    not_last: bool,
    /// Set when the segment is the first one in the prompt.
    first: bool,
}

/// Build the oneshot request fields every subcommand shares; each `From` arm
/// below fills in only the extras its own subcommand carries.
fn base_oneshot_request(
    request_type: OneshotKind,
    path: String,
    format: gpy_agent::formatter::Format,
    position: PromptPosition,
) -> OneshotRequest {
    OneshotRequest {
        request_type,
        path,
        format: format.as_str().to_owned(),
        is_last: !position.not_last, // is_last is the inverse of not_last
        is_first: position.first,
        benchmark_mode: false,
        duration_ms: 0_u64,
        success: None,
        hostname: None,
        is_ssh: false,
        username: None,
    }
}

impl From<OneshotCommands> for OneshotRequest {
    #[expect(
        clippy::too_many_lines,
        reason = "dispatch table for all oneshot subcommands; each arm names its own fields and adds only the extras that subcommand carries"
    )]
    fn from(command: OneshotCommands) -> Self {
        match command {
            OneshotCommands::Git {
                cwd,
                format,
                not_last,
                first,
                benchmark_mode,
            } => Self {
                benchmark_mode,
                ..base_oneshot_request(
                    OneshotKind::GitStatus,
                    cwd,
                    format,
                    PromptPosition { not_last, first },
                )
            },
            OneshotCommands::Lang {
                cwd,
                format,
                not_last,
                first,
            } => base_oneshot_request(
                OneshotKind::LanguageDetect,
                cwd,
                format,
                PromptPosition { not_last, first },
            ),
            OneshotCommands::Directory {
                cwd,
                format,
                not_last,
                first,
            } => base_oneshot_request(
                OneshotKind::Directory,
                cwd,
                format,
                PromptPosition { not_last, first },
            ),
            OneshotCommands::Duration {
                duration_ms,
                format,
                not_last,
                first,
            } => Self {
                duration_ms,
                ..base_oneshot_request(
                    OneshotKind::Duration,
                    String::new(),
                    format,
                    PromptPosition { not_last, first },
                )
            },
            OneshotCommands::Character {
                exit_code,
                format,
                not_last,
                first,
            } => Self {
                success: Some(exit_code == 0_i32),
                ..base_oneshot_request(
                    OneshotKind::Character,
                    String::new(),
                    format,
                    PromptPosition { not_last, first },
                )
            },
            OneshotCommands::Hostname {
                hostname,
                format,
                not_last,
                first,
                is_ssh,
            } => Self {
                hostname: Some(hostname),
                is_ssh,
                ..base_oneshot_request(
                    OneshotKind::Hostname,
                    String::new(),
                    format,
                    PromptPosition { not_last, first },
                )
            },
            OneshotCommands::Username {
                username,
                format,
                not_last,
                first,
            } => Self {
                username: Some(username),
                ..base_oneshot_request(
                    OneshotKind::Username,
                    String::new(),
                    format,
                    PromptPosition { not_last, first },
                )
            },
        }
    }
}

/// Handle oneshot command execution.
///
/// Renders the requested segment in-process and prints it. A failed request is
/// reported as JSON on stderr and exits non-zero, so this never returns an
/// error to the caller.
#[expect(clippy::exit, reason = "exit with error code on failure")]
fn handle_oneshot_command(command: OneshotCommands) {
    let request = OneshotRequest::from(command);

    match Agent::handle_oneshot_request(&request) {
        Ok(response) => println!("{response}"),
        Err(e) => {
            eprintln!("{}", serde_json::json!({"error": e.to_string()}));
            std::process::exit(1);
        }
    }
}

/// Handle config command execution.
///
/// # Errors
/// Returns an error when the requested key is unknown or the config cannot be loaded.
fn handle_config_command(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Get { key } => {
            // Propagate read/parse/validation errors instead of masking an
            // existing-but-invalid config with defaults (a missing config still
            // yields defaults via the loader's discovery).
            let config = config::loader::load_config()?;
            let value = config::metadata::get_config_value(&config, &key)?;
            println!("{value}");
        }
        ConfigCommands::Reload => {
            use gpy_agent::agent::lifecycle::get_socket_path;
            use gpy_agent::agent::lifecycle::send_config_reload_command;

            let socket_path = get_socket_path()?;
            if !socket_path.exists() {
                println!("Agent is not running; configuration changes will apply on next start.");
                return Ok(());
            }

            let success = tokio::runtime::Runtime::new()?.block_on(send_config_reload_command())?;

            if success {
                println!("Requested running agent to reload configuration.");
            } else {
                println!("Config reload request sent (agent may not have acknowledged).");
            }
        }
        ConfigCommands::List { filter } => {
            list(filter.as_deref());
        }
    }
    Ok(())
}

/// Handle theme subcommands (export, validate, import).
///
/// # Errors
/// Returns an error if the theme cannot be loaded, validated, or imported.
fn handle_theme_command(command: ThemeCommands) -> Result<()> {
    match command {
        ThemeCommands::Export { format } => export_theme(format),
        // Shares `gpy theme validate`'s implementation so both binaries print
        // the same success summary and the same remediation hints on failure.
        ThemeCommands::Validate { path } => gpy_agent::commands::theme::validate(path.as_deref()),
        ThemeCommands::Import {
            path,
            name,
            force,
            stdout,
            apply_layout,
        } => gpy_agent::commands::theme_import::run(
            &gpy_agent::commands::theme_import::ImportOptions {
                path: &path,
                name: name.as_deref(),
                force,
                output: if stdout {
                    gpy_agent::commands::theme_import::ImportOutput::Stdout
                } else {
                    gpy_agent::commands::theme_import::ImportOutput::WriteFiles
                },
                apply_layout,
            },
        ),
    }
}

/// GPY Agent entry point.
///
/// Every failure is reported as one `Error: ...` line on stderr rendered
/// through [`gpy_agent::error::user_message`] and exits `1`; the `Debug`
/// form `std::process::Termination` would print is not a message for a
/// person (#641). Clap usage errors exit `2` before this runs.
fn main() -> ExitCode {
    install_panic_handler();
    let cli = Cli::parse();

    match run(cli) {
        Ok(code) => code,
        Err(err) if gpy_agent::error::is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {}", gpy_agent::error::user_message(&err));
            ExitCode::from(gpy_agent::error::CLI_FAILURE_EXIT_CODE)
        }
    }
}

/// Dispatch the parsed command.
///
/// Only `status` carries a meaningful non-zero success-path exit code: it
/// exits `1` when no agent is running (or it cannot be reached) so scripts
/// and `gpy doctor` can rely on it, mirroring `systemctl status` (#636).
///
/// # Errors
/// Returns error if command execution fails or runtime initialization fails
fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Commands::Start { socket } => {
            apply_socket_override(socket);
            start_background_agent()?;
        }
        Commands::Stop { socket } => {
            apply_socket_override(socket);
            tokio::runtime::Runtime::new()?.block_on(stop_agent())?;
        }
        Commands::Status { socket } => {
            apply_socket_override(socket);
            let liveness = tokio::runtime::Runtime::new()?.block_on(print_agent_status())?;
            return Ok(match liveness {
                AgentLiveness::Responding => ExitCode::SUCCESS,
                AgentLiveness::NotRunning | AgentLiveness::Unreachable => {
                    ExitCode::from(STATUS_NOT_RUNNING_EXIT_CODE)
                }
            });
        }
        Commands::Theme { command } => handle_theme_command(command)?,
        Commands::Palette { command } => match command {
            PaletteCommands::List => gpy_agent::commands::palette::list()?,
            PaletteCommands::Use { name } => gpy_agent::commands::palette::use_palette(&name)?,
            PaletteCommands::Show => gpy_agent::commands::palette::show()?,
            PaletteCommands::Validate { target } => {
                gpy_agent::commands::palette::validate(target.as_deref())?;
            }
        },
        Commands::Oneshot { command } => handle_oneshot_command(command),
        Commands::Config { command } => handle_config_command(command)?,
        Commands::Doctor => gpy_agent::commands::doctor::run()?,
        Commands::Init {
            non_interactive,
            force,
        } => gpy_agent::commands::init::run(gpy_agent::commands::init::InitOptions {
            non_interactive,
            force,
        })?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Exit code of `gpy-agent status` when the agent is not running or does
/// not answer (#636). Documented in `docs/user/cli-reference.md`.
const STATUS_NOT_RUNNING_EXIT_CODE: u8 = 1;

/// What `gpy-agent status` found out about the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentLiveness {
    /// The socket exists and the agent answered a status query.
    Responding,
    /// No socket file: nothing is listening.
    NotRunning,
    /// A socket file exists but the agent did not answer (stale socket or
    /// a wedged process).
    Unreachable,
}

/// Check and display agent status.
///
/// Prints the report and returns what it found so the caller can turn it
/// into an exit code; the report itself is the same for every outcome (#636).
///
/// # Errors
///
/// Returns an error if the socket path cannot be determined.
async fn print_agent_status() -> Result<AgentLiveness> {
    use gpy_agent::agent::lifecycle::{
        check_protocol_version_compatibility, get_socket_path, query_agent_status,
    };

    println!("GPY Agent Status");
    println!("================");

    let socket_path = get_socket_path()?;
    println!("Socket Path: {}", socket_path.display());

    // Check config file location (always show this)
    let config_paths = schema::get_config_paths();

    match schema::resolve_active_config(&config_paths) {
        Some((_, Some(existing))) => println!("Config: {}", existing.display()),
        _ => println!("Config: Using defaults (no config file found)"),
    }
    println!();

    if !socket_path.exists() {
        println!("Status: Not Running (socket not found)");
        return Ok(AgentLiveness::NotRunning);
    }

    // Query the agent for real status information
    match query_agent_status().await {
        Ok(status) => {
            println!("Status: Running and Responding");
            println!("Agent Version: {}", status.version);
            println!("Protocol Version: {}", status.protocol_version);
            println!("Watched Repos: {}", status.watched_repos);
            println!("Registered Clients: {}", status.registered_clients);
            println!("Cache Entries: {}", status.cache_entries);

            // Check protocol version compatibility
            check_protocol_version_compatibility(status.protocol_version);
            Ok(AgentLiveness::Responding)
        }
        Err(e) => {
            println!("Status: Connection failed - {e}");
            Ok(AgentLiveness::Unreachable)
        }
    }
}

/// Stop the background agent gracefully
///
/// # Errors
///
/// Returns an error if the socket path cannot be determined.
async fn stop_agent() -> Result<()> {
    use gpy_agent::agent::lifecycle::{
        get_socket_path, send_shutdown_command, wait_for_agent_shutdown,
    };

    let socket_path = get_socket_path()?;
    if !socket_path.exists() {
        println!("Agent is not running (socket not found)");
        return Ok(());
    }

    match send_shutdown_command().await {
        Ok(()) => {
            println!("Shutdown command sent successfully");
            // Poll until the agent actually stops answering pings, rather
            // than trusting a fixed sleep + socket-file-existence check: the
            // socket can be unlinked by the shutdown handler before the
            // process has actually finished (e.g. still draining an
            // in-flight blocking task like language detection, #390), which
            // previously made this print "stopped successfully" while the
            // process kept running at full CPU. Runs on the blocking pool
            // since it internally spins its own runtime per ping and would
            // panic if called directly on this already-running one.
            let stopped = tokio::task::spawn_blocking({
                let wait_path = socket_path.clone();
                move || wait_for_agent_shutdown(&wait_path)
            })
            .await
            .unwrap_or(false);
            if stopped {
                println!("Agent stopped successfully");
                Ok(())
            } else {
                Err(gpy_agent::Error::agent(format!(
                    "agent at {} was still answering after the shutdown request",
                    socket_path.display()
                )))
            }
        }
        Err(e) => {
            if socket_is_stale(&socket_path) {
                // Nothing is listening: the file is a leftover, not a process
                // to signal. Only this verdict lets `stop` unlink the socket;
                // an accepted-but-unanswered connection is a live agent (#742).
                std::fs::remove_file(&socket_path).or_else(|remove_err| {
                    if remove_err.kind() == std::io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(remove_err)
                    }
                })?;
                println!("Agent is not running (stale socket removed)");
                Ok(())
            } else {
                Err(gpy_agent::Error::agent(format!(
                    "agent at {} did not answer the shutdown request ({e}); you may need to use 'kill' or system signals to stop it",
                    socket_path.display()
                )))
            }
        }
    }
}

/// Whether nothing is listening on `socket_path`.
///
/// A fresh connection is refused (or the file vanished). Any other outcome,
/// including an accepted connection, is evidence of a live agent and must not
/// be treated as stale.
#[cfg(unix)]
fn socket_is_stale(socket_path: &std::path::Path) -> bool {
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => false,
        Err(e) => matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
        ),
    }
}

/// Native Windows has no daemon to probe (#284); never claim a stale socket.
#[cfg(not(unix))]
fn socket_is_stale(_socket_path: &std::path::Path) -> bool {
    false
}

/// Export theme as shell variable assignments
///
/// # Errors
///
/// Returns an error if theme cannot be loaded or config cannot be read
fn export_theme(shell: gpy_agent::shell::Shell) -> Result<()> {
    // Load config to get theme name and enabled segments. Fall back to
    // defaults on failure (mirrors the oneshot request path) so a mid-write
    // or otherwise invalid config still sources a usable prompt instead of
    // `gpy-agent theme export <shell> | source` sourcing nothing (#323).
    let config = gpy_agent::config::loader::load_config().unwrap_or_else(|e| {
        gpy_agent::debug::warn_fallback("Config loading", "using defaults", &e);
        gpy_agent::config::Config::default()
    });
    let theme_name = &config.ui.theme;

    // Create theme manager and export
    let manager = gpy_agent::theme::ThemeManager::new(theme_name.as_str()).map_err(|e| {
        eprintln!("Failed to load theme '{theme_name}': {e}");
        e
    })?;

    // Export to requested format and print
    let output = manager.export(shell, &config);
    print!("{output}");

    Ok(())
}
