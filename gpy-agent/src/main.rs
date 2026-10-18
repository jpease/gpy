//! `gpy-agent`: the GPY background daemon plus the commands the shell
//! integrations call (`oneshot`, `theme export`, `config reload`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![expect(clippy::print_stdout, reason = "CLI output")]
#![expect(clippy::print_stderr, reason = "CLI diagnostics")]

use clap::{Parser, Subcommand};
use gpy_agent::{
    Result, VERSION,
    agent::{
        Agent,
        lifecycle::SOCKET_OVERRIDE,
        lifecycle::start::start_background_agent,
        oneshot::{OneshotKind, OneshotRequest},
    },
    config::schema,
};
use std::path::PathBuf;
use std::process::ExitCode;

/// GPY Agent - A high-performance background agent for the GPY prompt (Bash, Fish, and Zsh).
#[derive(Parser, Debug)]
#[command(author, version = VERSION, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the agent as a background process.
    Start {
        /// Override the socket path used by the agent (primarily for testing).
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Stop the background agent process.
    Stop {
        /// Override the socket path used by the agent (must match the path passed to start).
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Show whether the agent is running, with its version and cache details.
    Status {
        /// Override the socket path used by the agent (must match the path passed to start).
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Export the active theme for the shell integrations.
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Render one segment without the daemon (the shells' fallback).
    Oneshot {
        #[command(subcommand)]
        command: OneshotCommands,
    },
    /// Ask the running agent to reload its configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ThemeCommands {
    /// Print the active theme as shell variable assignments to source.
    Export {
        /// Output format ('fish', 'zsh', or 'bash'; case-insensitive).
        #[arg(long, ignore_case = true, default_value_t = gpy_agent::shell::Shell::Fish)]
        format: gpy_agent::shell::Shell,
    },
}

/// Prompt-position and output flags shared by every `oneshot` subcommand.
#[derive(clap::Args, Debug, Clone, Copy)]
struct SegmentArgs {
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
}

#[derive(Subcommand, Debug)]
enum OneshotCommands {
    /// Get Git repository status.
    Git {
        /// The directory to get Git status for (defaults to current directory).
        #[arg(long, short, default_value_t = String::from("."))]
        cwd: String,
        #[command(flatten)]
        segment: SegmentArgs,
        /// Skip config/theme loading so benchmarks time only the git-status
        /// path. Not for diagnostics.
        #[arg(long)]
        benchmark_mode: bool,
    },
    /// Detect programming languages in a directory.
    Lang {
        /// The directory to detect languages in (defaults to current directory).
        #[arg(long, short, default_value_t = String::from("."))]
        cwd: String,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Render the directory segment for a path (when a format template is configured).
    Directory {
        /// The directory to render (defaults to current directory).
        #[arg(long, short, default_value_t = String::from("."))]
        cwd: String,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Render the command-duration segment (when a format template is configured).
    Duration {
        /// Command duration in milliseconds (from `$CMD_DURATION`).
        #[arg(long, default_value_t = 0_u64)]
        duration_ms: u64,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Render the character segment (❯ prompt symbol, colored by exit status).
    Character {
        /// Last command exit code (0 = success, non-zero = error).
        #[arg(long, default_value_t = 0_i32)]
        exit_code: i32,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Render the hostname segment (when a format template is configured).
    ///
    /// The caller resolves the hostname and decides SSH-only visibility.
    Hostname {
        /// The hostname string to render (defaults to the machine hostname,
        /// or empty when that cannot be determined).
        #[arg(long, default_value_t = default_hostname_arg())]
        hostname: String,
        #[command(flatten)]
        segment: SegmentArgs,
        /// Mark the session as SSH, so the theme icon (Starship's
        /// `ssh_symbol`) renders.
        #[arg(long)]
        is_ssh: bool,
    },
    /// Render the username segment (when a format template is configured).
    Username {
        /// The username string to render (defaults to `$USER`/`$USERNAME`,
        /// or empty when that cannot be determined).
        #[arg(long, default_value_t = default_username_arg())]
        username: String,
        #[command(flatten)]
        segment: SegmentArgs,
    },
}

/// `$USER`/`$USERNAME`, or empty. The shells always pass `--username`.
fn default_username_arg() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// `$HOSTNAME`/`$COMPUTERNAME`, or empty. The shells always pass `--hostname`.
fn default_hostname_arg() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default()
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Request the running agent to reload configuration from disk.
    Reload,
}

/// Log panics through `warn_log!` (the daemon's stderr is `/dev/null`), and
/// exit 0 when a CLI pipeline closes stdout early: `unsafe` is forbidden, so
/// `SIGPIPE` cannot be reset to its default.
fn install_panic_handler() {
    std::panic::set_hook(Box::new(|panic_info| {
        let message = panic_info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic_info.payload().downcast_ref::<&str>().copied())
            .unwrap_or_default();
        if message.contains("failed printing to stdout") && message.contains("Broken pipe") {
            std::process::exit(0);
        }
        gpy_agent::warn_log!("agent", "PANIC: {:?}", panic_info);
    }));
}

/// Record `--socket`, made absolute against the current directory so a
/// forked daemon (which runs in `/`) binds the path the user meant.
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

/// Request fields every `oneshot` subcommand shares.
fn base_oneshot_request(
    request_type: OneshotKind,
    path: String,
    segment: SegmentArgs,
) -> OneshotRequest {
    OneshotRequest {
        request_type,
        path,
        format: segment.format.as_str().to_owned(),
        is_last: !segment.not_last,
        is_first: segment.first,
        benchmark_mode: false,
        duration_ms: 0_u64,
        success: None,
        hostname: None,
        is_ssh: false,
        username: None,
    }
}

impl From<OneshotCommands> for OneshotRequest {
    fn from(command: OneshotCommands) -> Self {
        match command {
            OneshotCommands::Git {
                cwd,
                segment,
                benchmark_mode,
            } => Self {
                benchmark_mode,
                ..base_oneshot_request(OneshotKind::GitStatus, cwd, segment)
            },
            OneshotCommands::Lang { cwd, segment } => {
                base_oneshot_request(OneshotKind::LanguageDetect, cwd, segment)
            }
            OneshotCommands::Directory { cwd, segment } => {
                base_oneshot_request(OneshotKind::Directory, cwd, segment)
            }
            OneshotCommands::Duration {
                duration_ms,
                segment,
            } => Self {
                duration_ms,
                ..base_oneshot_request(OneshotKind::Duration, String::new(), segment)
            },
            OneshotCommands::Character { exit_code, segment } => Self {
                success: Some(exit_code == 0_i32),
                ..base_oneshot_request(OneshotKind::Character, String::new(), segment)
            },
            OneshotCommands::Hostname {
                hostname,
                segment,
                is_ssh,
            } => Self {
                hostname: Some(hostname),
                is_ssh,
                ..base_oneshot_request(OneshotKind::Hostname, String::new(), segment)
            },
            OneshotCommands::Username { username, segment } => Self {
                username: Some(username),
                ..base_oneshot_request(OneshotKind::Username, String::new(), segment)
            },
        }
    }
}

/// Render the requested segment and print it; a failure is printed as JSON on
/// stderr and exits 1.
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

/// Ask the running agent to reload its config.
///
/// # Errors
/// Returns an error if the agent rejected the config (it keeps its previous
/// one) or did not answer, or the socket path or async runtime is unavailable.
fn reload_config() -> Result<()> {
    use gpy_agent::agent::lifecycle::{ConfigReload, get_socket_path, send_config_reload_command};

    let socket_path = get_socket_path()?;
    match tokio::runtime::Runtime::new()?.block_on(send_config_reload_command()) {
        Ok(ConfigReload::Applied) => println!("Running agent reloaded its configuration."),
        Ok(ConfigReload::NotRunning) => {
            println!("Agent is not running; configuration changes will apply on next start.");
        }
        Ok(ConfigReload::Rejected(reason)) => {
            return Err(gpy_agent::Error::config(format!(
                "the running agent rejected the new configuration and kept its previous one: {reason}"
            )));
        }
        Err(e) => {
            return Err(gpy_agent::Error::agent(format!(
                "agent at {} did not answer the config reload request ({e})",
                socket_path.display()
            )));
        }
    }
    Ok(())
}

/// Report failures as one `Error: ...` line and exit 1; clap usage errors
/// exit 2 before this runs.
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

/// Dispatch the parsed command. Only `status` has a non-zero success-path
/// exit code (1 when the agent is not running), like `systemctl status`.
///
/// # Errors
/// Returns the command's error.
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
        Commands::Theme {
            command: ThemeCommands::Export { format },
        } => export_theme(format)?,
        Commands::Oneshot { command } => handle_oneshot_command(command),
        Commands::Config {
            command: ConfigCommands::Reload,
        } => reload_config()?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Exit code of `gpy-agent status` when the agent is not running or does
/// not answer.
const STATUS_NOT_RUNNING_EXIT_CODE: u8 = 1;

/// What `gpy-agent status` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentLiveness {
    /// The socket exists and the agent answered a status query.
    Responding,
    /// No socket file: nothing is listening.
    NotRunning,
    /// A socket file exists but the agent did not answer.
    Unreachable,
}

/// Print the status report and return what it found.
///
/// # Errors
/// Returns an error if the socket path cannot be determined.
async fn print_agent_status() -> Result<AgentLiveness> {
    use gpy_agent::agent::lifecycle::{
        check_protocol_version_compatibility, get_socket_path, query_agent_status,
    };

    println!("GPY Agent Status");
    println!("================");

    let socket_path = get_socket_path()?;
    println!("Socket Path: {}", socket_path.display());

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

    match query_agent_status().await {
        Ok(status) => {
            println!("Status: Running and Responding");
            println!("Agent Version: {}", status.version);
            println!("Protocol Version: {}", status.protocol_version);
            println!("Watched Repos: {}", status.watched_repos);
            println!("Registered Clients: {}", status.registered_clients);
            println!("Cache Entries: {}", status.cache_entries);

            check_protocol_version_compatibility(status.protocol_version);
            Ok(AgentLiveness::Responding)
        }
        Err(e) => {
            println!("Status: Connection failed - {e}");
            Ok(AgentLiveness::Unreachable)
        }
    }
}

/// Stop the background agent gracefully.
///
/// # Errors
/// Returns an error if the agent is still answering or cannot be reached.
async fn stop_agent() -> Result<()> {
    use gpy_agent::agent::lifecycle::{
        get_socket_path, send_shutdown_command, socket_is_stale, wait_for_agent_shutdown,
    };

    let socket_path = get_socket_path()?;
    if !socket_path.exists() {
        println!("Agent is not running (socket not found)");
        return Ok(());
    }

    match send_shutdown_command().await {
        Ok(()) => {
            println!("Shutdown command sent successfully");
            // The socket can vanish while the agent is still draining a
            // blocking task, so poll pings instead. `spawn_blocking`: the
            // wait builds its own runtime per ping.
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
                // Only a refused connection proves the socket is stale; an
                // accepted-but-silent one is a live agent.
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

/// Print the active theme as shell variable assignments.
///
/// # Errors
/// Returns an error if the theme cannot be loaded.
fn export_theme(shell: gpy_agent::shell::Shell) -> Result<()> {
    // An invalid or half-written config must still export a usable prompt.
    let config = gpy_agent::config::loader::load_config().unwrap_or_else(|e| {
        gpy_agent::debug::warn_fallback("Config loading", "using defaults", &e);
        gpy_agent::config::Config::default()
    });
    let theme_name = &config.ui.theme;

    let manager = gpy_agent::theme::ThemeManager::new(theme_name.as_str()).map_err(|e| {
        eprintln!("Failed to load theme '{theme_name}': {e}");
        e
    })?;

    let output = manager.export(shell, &config);
    print!("{output}");

    Ok(())
}
