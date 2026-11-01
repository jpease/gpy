//! `gpy-agent`: the GPY background daemon plus the commands the shell
//! integrations call (`oneshot`, `theme export`, `config reload`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![expect(clippy::print_stdout, reason = "CLI output")]
#![expect(clippy::print_stderr, reason = "CLI diagnostics")]

use clap::{Parser, Subcommand};
use gpy_agent::{
    Result, VERSION,
    agent::oneshot::{OneshotKind, OneshotRequest, handle_oneshot_request},
    commands::{agent_report, exit_codes},
    shell::Shell,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Help for every `--socket`: one meaning, one description.
const SOCKET_HELP: &str = "Socket of the agent to act on (default: GPY_AGENT_SOCKET_PATH, else the runtime directory; see `gpy debug paths`)";

/// GPY Agent - A high-performance background agent for the GPY prompt (Bash, Fish, and Zsh).
#[derive(Parser, Debug)]
#[command(
    author,
    version = VERSION,
    about,
    long_about = None,
    after_help = exit_codes::AGENT_SUMMARY_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the agent as a background process.
    #[command(after_help = exit_codes::START_HELP)]
    Start {
        #[arg(long, value_name = "PATH", help = SOCKET_HELP)]
        socket: Option<PathBuf>,
        /// Print nothing on success.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Stop the background agent process.
    #[command(after_help = exit_codes::STOP_HELP)]
    Stop {
        #[arg(long, value_name = "PATH", help = SOCKET_HELP)]
        socket: Option<PathBuf>,
        /// Print nothing on success.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Stop the agent, then start it.
    #[command(after_help = exit_codes::RESTART_HELP)]
    Restart {
        #[arg(long, value_name = "PATH", help = SOCKET_HELP)]
        socket: Option<PathBuf>,
        /// Print nothing on success.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Show whether the agent is running, with its version and cache details.
    #[command(after_help = exit_codes::STATUS_HELP)]
    Status {
        #[arg(long, value_name = "PATH", help = SOCKET_HELP)]
        socket: Option<PathBuf>,
    },
    /// Export the active theme for the shell integrations.
    #[command(after_help = exit_codes::GENERIC_HELP)]
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Render one segment without the daemon (the shells' fallback).
    #[command(after_help = exit_codes::GENERIC_HELP)]
    Oneshot {
        #[command(subcommand)]
        command: OneshotCommands,
    },
    /// Ask the running agent to reload its configuration.
    #[command(after_help = exit_codes::GENERIC_HELP)]
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ThemeCommands {
    /// Print the active theme as shell variable assignments to source.
    #[command(after_help = exit_codes::GENERIC_HELP)]
    Export {
        /// Shell to export for ('fish', 'zsh', or 'bash'; case-insensitive).
        #[arg(long, value_name = "SHELL", ignore_case = true, default_value_t = Shell::Fish)]
        shell: Shell,
        /// Deprecated spelling of `--shell`.
        #[arg(long, hide = true, ignore_case = true, conflicts_with = "shell")]
        format: Option<Shell>,
    },
}

/// Prompt-position and output flags shared by every `oneshot` subcommand.
#[derive(clap::Args, Debug, Clone, Copy)]
struct SegmentArgs {
    /// Output format.
    #[arg(long, default_value_t = gpy_agent::formatter::Format::Json)]
    format: gpy_agent::formatter::Format,
    /// Another segment follows this one: no closing cap. Default: last.
    #[arg(long)]
    not_last: bool,
    /// This segment opens the prompt: no opening powerline cap. Default: not first.
    #[arg(long)]
    first: bool,
}

#[derive(Subcommand, Debug)]
enum OneshotCommands {
    /// Get Git repository status.
    Git {
        /// Directory to render for.
        #[arg(long, short, value_name = "DIR", value_hint = clap::ValueHint::DirPath, default_value = ".")]
        cwd: String,
        #[command(flatten)]
        segment: SegmentArgs,
        /// Skip config/theme loading so benchmarks time only the git-status
        /// path. Not for diagnostics.
        #[arg(long, hide = true)]
        benchmark_mode: bool,
    },
    /// Detect programming languages in a directory.
    Lang {
        /// Directory to render for.
        #[arg(long, short, value_name = "DIR", value_hint = clap::ValueHint::DirPath, default_value = ".")]
        cwd: String,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Render the directory segment for a path (when a format template is configured).
    Directory {
        /// Directory to render for.
        #[arg(long, short, value_name = "DIR", value_hint = clap::ValueHint::DirPath, default_value = ".")]
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
    /// Render the exit-status segment (exit-status pill, when a format template is configured).
    #[command(name = "exit-status")]
    ExitStatus {
        /// Last command exit code (0 = success, non-zero = failure).
        #[arg(long, default_value_t = 0_i32, allow_negative_numbers = true)]
        exit_code: i32,
        #[command(flatten)]
        segment: SegmentArgs,
    },
    /// Deprecated spelling of `exit-status`.
    #[command(name = "status", hide = true)]
    StatusDeprecated {
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
    #[command(after_help = exit_codes::RELOAD_HELP)]
    Reload {
        #[arg(long, value_name = "PATH", help = SOCKET_HELP)]
        socket: Option<PathBuf>,
    },
}

/// Log panics through `warn_log!`: the daemon's stderr is `/dev/null`.
fn install_panic_handler() {
    gpy_agent::error::install_broken_pipe_hook(|panic_info| {
        gpy_agent::warn_log!("agent", "PANIC: {:?}", panic_info);
    });
}

/// Warn on stderr that `old` is deprecated in favour of `new`.
fn warn_deprecated(old: &str, new: &str) {
    gpy_agent::commands::output::warn_stderr(format!("'{old}' is deprecated; use '{new}' instead"));
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
            OneshotCommands::ExitStatus { exit_code, segment }
            | OneshotCommands::StatusDeprecated { exit_code, segment } => Self {
                success: Some(exit_code == 0_i32),
                ..base_oneshot_request(OneshotKind::ExitStatus, String::new(), segment)
            },
        }
    }
}

/// Render the requested segment and print it.
///
/// # Errors
/// Returns the render error, which `main` prints as one `Error:` line.
fn handle_oneshot_command(command: OneshotCommands) -> Result<()> {
    gpy_agent::git::native::NativeGitBackend::disable_untracked_cache_setup();
    if matches!(command, OneshotCommands::StatusDeprecated { .. }) {
        warn_deprecated("gpy-agent oneshot status", "gpy-agent oneshot exit-status");
    }
    let request = OneshotRequest::from(command);

    println!("{}", handle_oneshot_request(&request)?);
    Ok(())
}

/// Ask the running agent to reload its config.
///
/// # Errors
/// Returns an error if the agent rejected the config (it keeps its previous
/// one) or did not answer, or the socket path or async runtime is unavailable.
fn reload_config(socket: Option<&Path>) -> Result<()> {
    use gpy_agent::agent::lifecycle::{AgentClient, ConfigReload};
    use gpy_agent::paths::SocketPath;

    let resolved = SocketPath::resolve(socket)?;
    match AgentClient::new(&resolved)?.reload_config() {
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
                resolved.path().display()
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

/// Dispatch the parsed command.
///
/// Exit codes: see `commands::exit_codes`.
///
/// # Errors
/// Returns the command's error.
fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Commands::Start { socket, quiet } => return agent_report::start(socket.as_deref(), quiet),
        Commands::Stop { socket, quiet } => return agent_report::stop(socket.as_deref(), quiet),
        Commands::Restart { socket, quiet } => {
            return agent_report::restart(socket.as_deref(), quiet);
        }
        Commands::Status { socket } => return Ok(agent_report::status(socket.as_deref())),
        Commands::Theme {
            command: ThemeCommands::Export { shell, format },
        } => export_theme(format.map_or(shell, |deprecated| {
            warn_deprecated("gpy-agent theme export --format", "--shell");
            deprecated
        }))?,
        Commands::Oneshot { command } => handle_oneshot_command(command)?,
        Commands::Config {
            command: ConfigCommands::Reload { socket },
        } => reload_config(socket.as_deref())?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Print the active theme as shell variable assignments.
///
/// # Errors
/// Returns an error if the theme cannot be loaded.
fn export_theme(shell: Shell) -> Result<()> {
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
