//! `gpy`: the user-facing CLI for the GPY prompt.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

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

    /// Wire GPY into your shell (Fish, Zsh or Bash): install its scripts,
    /// add the rc-file block, and pick an icon style. Safe to re-run.
    Setup {
        /// Shell to set up. Defaults to `GPY_SHELL`, then your login shell (`$SHELL`).
        #[arg(long, value_enum)]
        shell: Option<gpy_agent::shell::Shell>,
        /// Directory holding the fish/, zsh/ and bash/ script trees. Defaults to
        /// `GPY_SHARE_DIR`, then the share/gpy beside this binary, then the source
        /// checkout it was built in.
        #[arg(long, value_name = "DIR")]
        from: Option<std::path::PathBuf>,
        /// Do not prompt about icons; use the detected default.
        #[arg(long)]
        non_interactive: bool,
        /// Only re-run Nerd Font detection and overwrite `ui.show_icons`.
        #[arg(long, conflicts_with_all = ["shell", "from"])]
        redetect_icons: bool,
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
}

/// `clap::ValueEnum` mirror of [`gpy_agent::commands::complete::CompleteKind`].
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
    /// Show how your copy of a builtin theme differs from the current builtin
    Diff {
        /// Builtin theme name (`default`, `text` or `starship`)
        name: String,
    },
    /// Move your copy of a builtin theme to `.bak` so the builtin takes over
    Reset {
        /// Builtin theme name (`default`, `text` or `starship`)
        name: String,
    },
    /// Rewrite a user theme file for keys removed by breaking changes (backs up first)
    Migrate {
        /// Name of a theme file in the themes directory
        name: String,
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
    /// List every config key with its access mode and description
    List {
        /// Only list keys starting with this prefix (e.g., "git.icons")
        filter: Option<String>,
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
    /// `key=value` lines, as each shell's `__gpy_debug_paths` emits them.
    Kv,
}

/// Debug actions
#[derive(Subcommand, Debug)]
enum DebugAction {
    /// Time the agent's ping, git and language round trips
    Prompt,

    /// Dump every path GPY resolves from the environment (the same map as
    /// each shell's `__gpy_debug_paths`)
    Paths {
        /// Output format
        #[arg(long, value_enum, default_value_t = PathsFormat::Json)]
        format: PathsFormat,
    },
}

/// Dispatch a theme subcommand.
///
/// # Errors
/// Returns the subcommand's error.
fn run_theme(action: ThemeAction) -> Result<()> {
    match action {
        ThemeAction::List => commands::theme::list(),
        ThemeAction::Use { name, force } => commands::theme::use_theme(&name, force),
        ThemeAction::Show => commands::theme::show(),
        ThemeAction::New { name, from } => commands::theme::new(&name, from.as_deref()),
        ThemeAction::Save { name } => commands::theme::save(name.as_deref()),
        ThemeAction::Validate { target } => commands::theme::validate(target.as_deref()),
        ThemeAction::Diff { name } => commands::theme_override::diff(&name),
        ThemeAction::Reset { name } => commands::theme_override::reset(&name),
        ThemeAction::Migrate { name } => commands::theme_migrate::migrate(&name),
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

/// Dispatch a palette subcommand.
///
/// # Errors
/// Returns the subcommand's error.
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

/// `gpy setup`, or only the icon re-detection with `--redetect-icons`.
///
/// # Errors
/// Propagates setup or icon-detection failures.
fn run_setup(
    shell: Option<gpy_agent::shell::Shell>,
    from: Option<std::path::PathBuf>,
    non_interactive: bool,
    redetect_icons: bool,
) -> Result<()> {
    if redetect_icons {
        return commands::init::run(commands::init::InitOptions {
            non_interactive,
            redetect: true,
        });
    }
    commands::setup::run(
        &commands::setup::SetupOptions {
            shell,
            from,
            non_interactive,
        },
        |target| {
            Ok(render_completions(match target {
                gpy_agent::shell::Shell::Fish => clap_complete::Shell::Fish,
                gpy_agent::shell::Shell::Zsh => clap_complete::Shell::Zsh,
                gpy_agent::shell::Shell::Bash => clap_complete::Shell::Bash,
            }))
        },
    )
}

/// The structural completion script for `shell`.
fn render_completions(shell: clap_complete::Shell) -> Vec<u8> {
    let mut cmd = strip_hidden_for_completions(&Cli::command());
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut cmd, "gpy", &mut script);
    script
}

/// Rebuild `cmd` without `hide`-marked subcommands and arguments: the
/// `clap_complete` generators ignore `hide`, unlike `--help`.
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

/// Report failures as one `Error: ...` line and exit 1; clap usage errors
/// exit 2 before this runs.
fn main() -> ExitCode {
    let default_hook = std::panic::take_hook();
    gpy_agent::error::install_broken_pipe_hook(move |info| default_hook(info));
    let cli = Cli::parse();

    gpy_agent::error::report(run(cli.command))
}

/// Dispatch the parsed command. Only `status` has a non-zero success-path
/// exit code (1 when no agent is running).
///
/// # Errors
/// Returns the subcommand's error for [`main`] to report.
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
            ConfigAction::List { filter } => commands::config::list(filter.as_deref()),
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
        Commands::Setup {
            shell,
            from,
            non_interactive,
            redetect_icons,
        } => run_setup(shell, from, non_interactive, redetect_icons)?,
        Commands::Completions { shell } => {
            // The generator unwraps its own writes; buffer so a closed pipe
            // surfaces here as a quiet exit 0.
            let script = render_completions(shell);
            std::io::Write::write_all(&mut std::io::stdout().lock(), &script)?;
        }
        Commands::Complete { kind } => commands::complete::run(kind.into()),
    }
    Ok(ExitCode::SUCCESS)
}
