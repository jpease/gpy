//! # GPY - Modern prompt CLI for Bash, Fish, and Zsh
//!
//! User-facing command-line interface for managing the GPY prompt system.
//!
//! This is the primary interface users interact with. The `gpy-agent` binary
//! is an internal implementation detail used by the background daemon.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![expect(
    clippy::print_stderr,
    reason = "this binary reports CLI errors and diagnostics on stderr, the conventional channel for a command-line tool"
)]

use clap::{CommandFactory, Parser, Subcommand};
use gpy_agent::{Result, VERSION, commands};
use std::process::ExitCode;

/// GPY - Modern prompt CLI for Bash, Fish, and Zsh
#[derive(Parser, Debug)]
#[command(name = "gpy", version = VERSION, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Available commands
#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the background agent
    Start,

    /// Stop the background agent
    Stop,

    /// Restart the background agent
    Restart,

    /// Show agent status and diagnostics
    Status,

    /// Manage themes
    Theme {
        #[command(subcommand)]
        action: ThemeAction,
    },
    /// Manage palettes (color themes)
    Palette {
        #[command(subcommand)]
        action: PaletteAction,
    },

    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },

    /// Enable a prompt segment
    Enable {
        /// Segment name (built-in or discovered plugin segment, e.g. `git`, `lang`, `k8s-tools`)
        segment: String,
    },

    /// Disable a prompt segment
    Disable {
        /// Segment name (built-in or discovered plugin segment)
        segment: String,
    },

    /// List all available segments
    Segments,

    /// Language detection configuration
    Lang {
        #[command(subcommand)]
        action: LangAction,
    },

    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Run health checks and diagnostics
    Doctor,

    /// Debugging tools
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },

    /// Internal: emit completion candidate names for shell glue (hidden).
    #[command(name = "__complete", hide = true)]
    Complete {
        /// Which value list to emit.
        #[arg(value_enum)]
        kind: CliCompleteKind,
    },

    /// Internal: force a wizard terminal-setup failure at `stage`, for the
    /// PTY-backed regression test in `tests/wizard_pty_tests.rs` (hidden;
    /// compiled only into `test-support` builds — see #460).
    #[cfg(all(unix, feature = "test-support"))]
    #[command(name = "__wizard_force_fail", hide = true)]
    WizardForceFail {
        /// Which `TerminalGuard` setup stage to fail.
        #[arg(value_enum)]
        stage: CliForcedFailureStage,
    },
}

/// Value-list kind for the hidden `gpy __complete` command.
///
/// Mirrors [`gpy_agent::commands::complete::CompleteKind`] but derives
/// [`clap::ValueEnum`] so `clap` can parse it directly from argv; the lib
/// crate's enum stays free of CLI-parsing concerns.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum CliCompleteKind {
    /// Installed theme names.
    Theme,
    /// Installed palette names.
    Palette,
    /// Available prompt segment names.
    Segment,
}

impl From<CliCompleteKind> for commands::complete::CompleteKind {
    fn from(kind: CliCompleteKind) -> Self {
        match kind {
            CliCompleteKind::Theme => Self::Theme,
            CliCompleteKind::Palette => Self::Palette,
            CliCompleteKind::Segment => Self::Segment,
        }
    }
}

/// Value-list kind for the hidden `gpy __wizard_force_fail` command.
///
/// Mirrors [`gpy_agent::commands::wizard::ForcedFailureStage`] but derives
/// [`clap::ValueEnum`] so `clap` can parse it directly from argv; the lib
/// crate's enum stays free of CLI-parsing concerns.
#[cfg(all(unix, feature = "test-support"))]
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum CliForcedFailureStage {
    /// Fail after raw mode is enabled.
    AfterRawMode,
    /// Fail after the alternate screen is entered.
    AfterAltScreen,
}

#[cfg(all(unix, feature = "test-support"))]
impl From<CliForcedFailureStage> for commands::wizard::ForcedFailureStage {
    fn from(stage: CliForcedFailureStage) -> Self {
        match stage {
            CliForcedFailureStage::AfterRawMode => Self::AfterRawMode,
            CliForcedFailureStage::AfterAltScreen => Self::AfterAltScreen,
        }
    }
}

/// Theme management actions
#[derive(Subcommand, Debug)]
enum ThemeAction {
    /// List available themes
    List,
    /// Switch to a theme
    Use {
        /// Theme name
        name: String,
        /// Apply the theme's recommended layout/config (segment order, directory
        /// display, language icons, …), preserving any settings you've
        /// explicitly configured.
        #[arg(long)]
        force: bool,
        /// Deprecated alias for `--force`.
        #[arg(long, hide = true)]
        apply_layout: bool,
    },
    /// Show current theme
    Show,
    /// Create new theme from template
    New {
        /// New theme name
        name: String,
        /// Clone an existing theme's full contents instead of the blank
        /// default template (theme name, not a file path).
        #[arg(long)]
        from: Option<String>,
    },
    /// Save the currently active theme's contents as a theme file
    Save {
        /// Theme name to save as (defaults to the currently active theme's
        /// own name, always prompting before overwrite)
        name: Option<String>,
    },
    /// Validate a theme by name or file path (defaults to active theme)
    Validate {
        /// Optional theme name or direct path to a theme TOML file
        target: Option<String>,
    },
    /// Import a `starship.toml` into a GPY palette + prompt theme
    Import {
        /// Path to the source `starship.toml`
        path: String,
        /// Base name for the emitted artifacts (default: file stem)
        #[arg(long)]
        name: Option<String>,
        /// Overwrite existing artifacts of the same name, or shadow a builtin or plugin theme or palette
        #[arg(long)]
        force: bool,
        /// Print both artifacts instead of writing files
        #[arg(long)]
        stdout: bool,
        /// Also write the derived `enabled_segments` into the active config
        #[arg(long)]
        apply_layout: bool,
    },
}

/// Palette management actions
#[derive(Subcommand, Debug)]
enum PaletteAction {
    /// List available palettes
    List,
    /// Switch the active palette
    Use {
        /// Palette name to activate
        name: String,
    },
    /// Show current palette name
    Show,
    /// Validate a palette file or the currently active palette
    Validate {
        /// Optional palette name or path. If omitted, validates the active palette.
        target: Option<String>,
    },
    /// Import a base16/base24 scheme file as a palette
    Import {
        /// Path to a base16/base24 scheme YAML file
        file: std::path::PathBuf,
        /// Override the palette name (defaults to the scheme name)
        #[arg(long)]
        name: Option<String>,
        /// Overwrite an existing palette of the same name, or shadow a builtin or plugin palette
        #[arg(long)]
        force: bool,
    },
}

/// Plugin management actions
#[derive(Subcommand, Debug)]
enum PluginAction {
    /// List discovered plugins
    List,
    /// Create a new plugin scaffold
    New {
        /// Plugin ID
        id: String,
        /// Optional segment ID (defaults to the plugin ID)
        #[arg(long)]
        segment: Option<String>,
    },
    /// Validate a plugin by path or ID
    Validate {
        /// Plugin directory path (or plugin.toml path), or discovered plugin ID
        target: String,
    },
}

/// Language configuration actions
#[derive(Subcommand, Debug)]
enum LangAction {
    /// Toggle version display (on/off)
    Versions {
        /// Enable or disable version display
        #[arg(value_parser = ["on", "off"])]
        toggle: String,
    },
}

/// Configuration actions
#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Show configuration
    Show {
        /// Section to show: agent, git, language, ui (shows all if omitted)
        section: Option<String>,
    },
    /// Get a config value
    Get {
        /// Config key using dot-notation (e.g., "git.enabled")
        key: String,
    },
    /// Set a config value
    Set {
        /// Config key using dot-notation
        key: String,
        /// New value
        value: String,
    },
    /// Open the config file in your editor
    Open,
    /// Launch the interactive configuration wizard (theme, palette, segments, live preview)
    Wizard,
}

/// Output format for `gpy debug paths`.
#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
enum PathsFormat {
    /// A JSON object, keys in resolution order.
    #[default]
    Json,
    /// `key=value` lines — the format each shell's `__gpy_debug_paths` emits,
    /// and what the cross-shell parity harness diffs.
    Kv,
}

/// Debug actions
#[derive(Subcommand, Debug)]
enum DebugAction {
    /// Show timing breakdown for prompt rendering
    Prompt,

    /// Dump every path GPY resolves from the environment
    ///
    /// Each shell integration exposes the same map via `__gpy_debug_paths`;
    /// `tests/fish/path_parity.test.fish` diffs the two across an environment
    /// matrix so path resolution cannot drift between the agent and a shell.
    Paths {
        /// Output format
        #[arg(long, value_enum, default_value_t = PathsFormat::Json)]
        format: PathsFormat,
    },
}

/// Dispatch a theme subcommand to the appropriate handler.
///
/// # Errors
///
/// Returns an error if the theme operation fails.
fn run_theme(action: ThemeAction) -> Result<()> {
    match action {
        ThemeAction::List => commands::theme::list(),
        ThemeAction::Use {
            name,
            force,
            apply_layout,
        } => {
            if apply_layout {
                eprintln!("⚠️  --apply-layout is deprecated; use --force instead.");
            }
            commands::theme::use_theme(&name, force || apply_layout)
        }
        ThemeAction::Show => commands::theme::show(),
        ThemeAction::New { name, from } => commands::theme::new(&name, from.as_deref()),
        ThemeAction::Save { name } => commands::theme::save(name.as_deref()),
        ThemeAction::Validate { target } => commands::theme::validate(target.as_deref()),
        ThemeAction::Import {
            path,
            name,
            force,
            stdout,
            apply_layout,
        } => commands::theme_import::run(&commands::theme_import::ImportOptions {
            path: &path,
            name: name.as_deref(),
            force,
            output: if stdout {
                commands::theme_import::ImportOutput::Stdout
            } else {
                commands::theme_import::ImportOutput::WriteFiles
            },
            apply_layout,
        }),
    }
}

/// Dispatch a palette subcommand to the appropriate handler.
///
/// # Errors
///
/// Returns an error if the palette operation fails.
fn run_palette(action: PaletteAction) -> Result<()> {
    match action {
        PaletteAction::List => commands::palette::list(),
        PaletteAction::Use { name } => commands::palette::use_palette(&name),
        PaletteAction::Show => commands::palette::show(),
        PaletteAction::Validate { target } => commands::palette::validate(target.as_deref()),
        PaletteAction::Import { file, name, force } => {
            commands::palette::import(&file, name.as_deref(), force)
        }
    }
}

/// Rebuild `cmd` with every `hide`-marked subcommand and argument removed so
/// generated shell completions match `--help` visibility.
///
/// `clap_complete`'s ahead-of-time generators (fish/bash/zsh) do not honor
/// `is_hide_set()` for subcommands or arguments (unlike `--help`), so hidden
/// items — the internal `__complete` command and the deprecated `--apply-layout`
/// alias — would otherwise leak into tab-completion. Keeping `hide` as the
/// single source of truth, we strip them here before handing the command to
/// `generate`. Arguments are cloned (full fidelity); command nodes are rebuilt
/// while copying the metadata the generators read.
fn strip_hidden_for_completions(cmd: &clap::Command) -> clap::Command {
    let mut stripped = clap::Command::new(cmd.get_name().to_owned());

    if let Some(about) = cmd.get_about() {
        stripped = stripped.about(about.clone());
    }
    if let Some(long_about) = cmd.get_long_about() {
        stripped = stripped.long_about(long_about.clone());
    }
    if let Some(version) = cmd.get_version() {
        stripped = stripped.version(version.to_owned());
    }
    let aliases: Vec<String> = cmd.get_visible_aliases().map(str::to_owned).collect();
    if !aliases.is_empty() {
        stripped = stripped.visible_aliases(aliases);
    }
    stripped = stripped.display_order(cmd.get_display_order());
    if cmd.is_allow_external_subcommands_set() {
        stripped = stripped.allow_external_subcommands(true);
    }

    for arg in cmd.get_arguments() {
        if !arg.is_hide_set() {
            stripped = stripped.arg(arg.clone());
        }
    }
    for sub in cmd.get_subcommands() {
        if !sub.is_hide_set() {
            stripped = stripped.subcommand(strip_hidden_for_completions(sub));
        }
    }

    stripped
}

/// GPY CLI entry point.
///
/// Every failure is reported as one `Error: ...` line on stderr rendered
/// through [`gpy_agent::error::user_message`] and exits `1`; the `Debug`
/// form `std::process::Termination` would print (`Config { message: ... }`)
/// is not a message for a person (#641). Clap usage errors exit `2` before
/// this runs. A reader that goes away mid-output (`gpy ... | head -1`) ends
/// the command quietly with `0` instead of a broken-pipe panic.
fn main() -> ExitCode {
    install_broken_pipe_hook();
    let cli = Cli::parse();

    match run(cli.command) {
        Ok(code) => code,
        Err(err) if gpy_agent::error::is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {}", gpy_agent::error::user_message(&err));
            ExitCode::from(gpy_agent::error::CLI_FAILURE_EXIT_CODE)
        }
    }
}

/// End the process quietly when stdout's reader has gone away.
///
/// `println!` panics with "failed printing to stdout: Broken pipe" once the
/// pipe closes, which used to surface as exit `101` for something as
/// ordinary as `gpy segments | head -1` (#641). This crate forbids `unsafe`,
/// so the usual fix of restoring `SIGPIPE`'s default disposition is not
/// available; instead the panic hook recognises that one payload and exits
/// `0`, the outcome a shell pipeline expects. Every other panic keeps the
/// default hook's report and exit `101`.
fn install_broken_pipe_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or_default();
        if message.contains("failed printing to stdout") && message.contains("Broken pipe") {
            #[expect(
                clippy::exit,
                reason = "a closed stdout reader is a normal end, not a failure"
            )]
            std::process::exit(0);
        }
        default_hook(info);
    }));
}

/// Dispatch the parsed command.
///
/// Only `status` carries a meaningful non-zero exit code on its success
/// path (`1` when no agent is running, #636); every other command exits
/// `0` on `Ok`.
///
/// # Errors
///
/// Returns the subcommand's error unchanged for [`main`] to report.
fn run(command: Commands) -> Result<ExitCode> {
    match command {
        Commands::Start => commands::lifecycle::start()?,
        Commands::Stop => commands::lifecycle::stop()?,
        Commands::Restart => commands::lifecycle::restart()?,
        Commands::Status => return commands::lifecycle::status(),
        Commands::Theme { action } => run_theme(action)?,
        Commands::Palette { action } => run_palette(action)?,
        Commands::Plugin { action } => match action {
            PluginAction::List => commands::plugin::list(),
            PluginAction::New { id, segment } => commands::plugin::new(&id, segment.as_deref())?,
            PluginAction::Validate { target } => commands::plugin::validate(&target)?,
        },
        Commands::Enable { segment } => commands::segments::enable(&segment)?,
        Commands::Disable { segment } => commands::segments::disable(&segment)?,
        Commands::Segments => commands::segments::list()?,
        Commands::Lang { action } => match action {
            LangAction::Versions { toggle } => commands::lang::set_versions(toggle == "on")?,
        },
        Commands::Config { action } => match action {
            ConfigAction::Show { section } => commands::config::show(section.as_deref())?,
            ConfigAction::Get { key } => commands::config::get(&key)?,
            ConfigAction::Set { key, value } => commands::config::set(&key, &value)?,
            ConfigAction::Open => commands::config::open()?,
            ConfigAction::Wizard => commands::wizard::run()?,
        },
        Commands::Doctor => commands::doctor::run()?,
        Commands::Debug { action } => match action {
            DebugAction::Prompt => commands::debug::prompt()?,
            DebugAction::Paths { format } => {
                commands::debug_paths::run(matches!(format, PathsFormat::Json));
            }
        },
        Commands::Completions { shell } => {
            // clap_complete's aot generators don't filter `hide`-marked items,
            // so strip them first (keeps `hide` as the single source of truth).
            let mut cmd = strip_hidden_for_completions(&Cli::command());
            // Render to memory first: the generator unwraps its own writes,
            // so a reader that closes early (`| head -1`) must be met here,
            // where a broken pipe becomes a quiet exit 0 (#641).
            let mut script = Vec::new();
            clap_complete::generate(shell, &mut cmd, "gpy", &mut script);
            std::io::Write::write_all(&mut std::io::stdout().lock(), &script)?;
        }
        Commands::Complete { kind } => commands::complete::run(kind.into()),
        #[cfg(all(unix, feature = "test-support"))]
        Commands::WizardForceFail { stage } => {
            commands::wizard::run_with_forced_failure(stage.into())?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
