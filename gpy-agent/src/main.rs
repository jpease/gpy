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
        lifecycle::{
            START_PROGRESS_LINE, StartNotice, StatusProbe, probe_status, protocol_mismatch,
            start_background_agent,
        },
        oneshot::{OneshotKind, OneshotRequest, handle_oneshot_request},
    },
    config::schema,
    paths::set_socket_override,
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
        #[arg(long, default_value_t = 0_i32, allow_negative_numbers = true)]
        exit_code: i32,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Render the status segment (exit-status pill, when a format template is configured).
    Status {
        /// Last command exit code (0 = success, non-zero = failure).
        #[arg(long, default_value_t = 0_i32, allow_negative_numbers = true)]
        exit_code: i32,
        #[command(flatten)]
        segment: SegmentArgs,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Request the running agent to reload configuration from disk.
    Reload,
}

/// Log panics through `warn_log!`: the daemon's stderr is `/dev/null`.
fn install_panic_handler() {
    gpy_agent::error::install_broken_pipe_hook(|panic_info| {
        gpy_agent::warn_log!("agent", "PANIC: {:?}", panic_info);
    });
}

/// Record `--socket`, made absolute against the current directory so a
/// forked daemon (which runs in `/`) binds the path the user meant.
fn apply_socket_override(socket: Option<PathBuf>) -> Result<()> {
    if let Some(given) = socket {
        set_socket_override(&given)?;
    }
    Ok(())
}

/// Request fields every `oneshot` subcommand shares.
const fn base_oneshot_request(
    request_type: OneshotKind,
    path: String,
    segment: SegmentArgs,
) -> OneshotRequest {
    OneshotRequest {
        request_type,
        path,
        format: segment.format,
        is_last: !segment.not_last,
        is_first: segment.first,
        benchmark_mode: false,
        duration_ms: 0_u64,
        success: None,
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
            OneshotCommands::Status { exit_code, segment } => Self {
                success: Some(exit_code == 0_i32),
                ..base_oneshot_request(OneshotKind::ExitStatus, String::new(), segment)
            },
        }
    }
}

/// Render the requested segment and print it; a failure is printed as JSON on
/// stderr and exits 1.
#[allow(
    clippy::exit,
    reason = "oneshot failure must exit 1 after printing its JSON error; the caller returns no exit code"
)]
fn handle_oneshot_command(command: OneshotCommands) {
    gpy_agent::git::native::NativeGitBackend::disable_untracked_cache_setup();
    let request = OneshotRequest::from(command);

    match handle_oneshot_request(&request) {
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
    use gpy_agent::agent::lifecycle::{ConfigReload, send_config_reload_command};
    use gpy_agent::paths::get_socket_path;

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

    gpy_agent::error::report(run(cli))
}

/// Dispatch the parsed command. Only `status` has a non-zero success-path
/// exit code (1 when the agent is not running), like `systemctl status`.
///
/// # Errors
/// Returns the command's error.
fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Commands::Start { socket } => {
            apply_socket_override(socket)?;
            start_background_agent(|notice| eprintln!("{}", notice_text(&notice)))?;
        }
        Commands::Stop { socket } => {
            apply_socket_override(socket)?;
            tokio::runtime::Runtime::new()?.block_on(stop_agent())?;
        }
        Commands::Status { socket } => {
            apply_socket_override(socket)?;
            let outcome = print_agent_status()?;
            return Ok(match outcome {
                StatusOutcome::Responding => ExitCode::SUCCESS,
                StatusOutcome::Busy | StatusOutcome::NotRunning | StatusOutcome::Unreachable => {
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
pub(crate) enum StatusOutcome {
    /// The agent answered a status query.
    Responding,
    /// The agent accepts connections but is closing them without a reply.
    Busy,
    /// No socket file, or nothing listening on it.
    NotRunning,
    /// The agent did not answer, or the probe could not be made.
    Unreachable,
}

/// The stderr text for a start notice.
fn notice_text(notice: &StartNotice) -> String {
    match notice {
        StartNotice::Disabled => {
            "GPY Agent disabled via config (agent.enabled = false); skipping start".to_owned()
        }
        StartNotice::Unverifiable => {
            "Could not verify whether the GPY agent is running; leaving its socket alone".to_owned()
        }
        StartNotice::EvictingUnanswered => "GPY agent accepted a connection but did not answer across every retry; evicting the wedged agent...".to_owned(),
        StartNotice::VersionMismatch { running } => format!(
            "Detected version mismatch (running: {running}, binary: {VERSION})\nRestarting agent with this binary's version ({VERSION})..."
        ),
        StartNotice::ProtocolMismatch { running } => format!(
            "Detected protocol mismatch (running: protocol {running}, binary: protocol {})\nRestarting agent with this binary's version ({VERSION})...",
            gpy_agent::ipc::protocol::PROTOCOL_VERSION
        ),
        StartNotice::StaleSocket => "Cleaning up stale socket...".to_owned(),
        StartNotice::AlreadyRunning => "GPY Agent is already running and responsive".to_owned(),
        StartNotice::Busy => {
            "GPY Agent is running but too busy to answer; leaving it alone".to_owned()
        }
        StartNotice::NewerRunning { running } => format!(
            "A newer GPY agent ({running}) is already running; not replacing it with {VERSION}"
        ),
        StartNotice::Starting => START_PROGRESS_LINE.to_owned(),
    }
}

/// Print the status report and return what it found.
///
/// # Errors
/// Returns an error if the socket path cannot be determined.
fn print_agent_status() -> Result<StatusOutcome> {
    use gpy_agent::paths::get_socket_path;

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
        return Ok(StatusOutcome::NotRunning);
    }

    Ok(match probe_status(&socket_path) {
        StatusProbe::Answered(status) => {
            println!("Status: Running and Responding");
            println!("Agent Version: {}", status.version);
            println!("Protocol Version: {}", status.protocol_version);
            println!("Watched Repos: {}", status.watched_repos);
            println!("Registered Clients: {}", status.registered_clients);
            println!("Cache Entries: {}", status.cache_entries);

            if let Some(mismatch) = protocol_mismatch(status.protocol_version) {
                print_protocol_mismatch(mismatch.running, mismatch.expected);
            }
            StatusOutcome::Responding
        }
        StatusProbe::Busy => {
            println!("Status: Running but busy (not answering right now)");
            StatusOutcome::Busy
        }
        StatusProbe::NotListening(e) => {
            println!("Status: Connection failed - {e}");
            StatusOutcome::NotRunning
        }
        StatusProbe::NoReply(e) | StatusProbe::Failed(e) => {
            println!("Status: Connection failed - {e}");
            StatusOutcome::Unreachable
        }
    })
}

/// Print the protocol-mismatch advice block.
fn print_protocol_mismatch(running: u8, expected: u8) {
    println!();
    gpy_agent::commands::output::warn("Protocol version mismatch");
    println!("=================================================");
    println!("Running agent protocol version: {running}");
    println!("Expected protocol version: {expected}");
    println!();
    println!("This may cause compatibility issues. Recommended actions:");
    println!("  1. Restart the agent: gpy-agent stop && gpy-agent start");
    println!("  2. If issues persist, update gpy and re-run: gpy setup");
    println!();
    println!("Fallback: Use oneshot mode if IPC fails:");
    println!("  gpy-agent oneshot git --cwd . --format ansi");
    println!("=================================================");
}

/// Stop the background agent gracefully.
///
/// # Errors
/// Returns an error if the agent is still answering or cannot be reached.
async fn stop_agent() -> Result<()> {
    use gpy_agent::agent::lifecycle::{
        send_shutdown_command, socket_is_stale, wait_for_agent_shutdown,
    };
    use gpy_agent::paths::get_socket_path;

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
        // Only the agent kill switch survives a broken file.
        let mut config = gpy_agent::config::Config::default();
        config.agent.enabled = gpy_agent::config::loader::agent_enabled();
        config
    });
    let theme_name = &config.ui.theme;

    let manager = gpy_agent::theme::ThemeManager::new(theme_name.as_str()).map_err(|e| {
        eprintln!("Failed to load theme '{theme_name}': {e}");
        e
    })?;

    let palette = gpy_agent::palette::active_palette(&config);
    let output = manager.export(shell, &config, &palette);
    print!("{output}");

    Ok(())
}
