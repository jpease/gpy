//! Icon-style bootstrap run by `gpy setup`: pick `ui.show_icons` so the first
//! prompt renders on this machine.
//!
//! Font presence does not prove the terminal uses that font, so on a terminal
//! the user confirms a rendered sample; detection only pre-selects the answer.
//! A set `GPY_NERD_FONT` is the user's answer and skips the prompt.

use crate::Result;
use crate::commands::output;
use crate::commands::utils::{active_config_path, load_active_config};
use crate::config::loader::save_config;
use crate::font::{self, FontCapability};
use std::path::Path;

/// Options for [`run`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InitOptions {
    /// Skip the confirmation prompt and use the detected capability.
    pub non_interactive: bool,
    /// Overwrite `ui.show_icons` in an existing config instead of leaving it alone.
    pub redetect: bool,
}

/// Detect icon support and write `ui.show_icons`.
///
/// # Errors
///
/// Returns an error if the config path cannot be resolved, an existing config
/// is invalid, or the config cannot be written.
pub fn run(options: InitOptions) -> Result<()> {
    let path = active_config_path()?;

    if path.exists() && !options.redetect {
        output::plain(format_args!(
            "GPY config already exists at {}",
            path.display()
        ));
        output::detail("Leaving your existing settings unchanged.");
        return Ok(());
    }

    let capability = font::detect_nerd_font();
    // The installers run this from a terminal; an explicit override must not block on `/dev/tty`.
    let non_interactive = options.non_interactive || font::override_from_env().is_some();
    let show_icons = resolve_show_icons(capability, non_interactive);

    let original = load_active_config(&path)?;
    let mut config = original.clone();
    config.ui.show_icons = show_icons;
    save_config(&path, &original, &config, &["ui.show_icons"])?;

    report(&path, capability, show_icons);
    Ok(())
}

/// The recommended value when non-interactive, else the user's answer.
fn resolve_show_icons(capability: FontCapability, non_interactive: bool) -> bool {
    let recommended = font::recommend_show_icons(capability);
    if non_interactive {
        return recommended;
    }
    confirm_icons(capability, recommended).unwrap_or_else(|err| {
        let choice = if recommended { "icons" } else { "ASCII" };
        output::warn_stderr(format_args!(
            "could not ask about icons on /dev/tty ({err}); using {choice}"
        ));
        recommended
    })
}

/// A prompt line with a git branch (U+E0A0), the Rust icon (U+F1617) and `❯`.
#[cfg(unix)]
const SAMPLE_PROMPT_LINE: &str = "\u{e0a0} main   \u{f1617} rust 1.82   \u{276f}";

/// Ask on `/dev/tty` whether the sample renders; stdin is the installer's
/// piped script, not the user.
///
/// # Errors
/// Returns an error when `/dev/tty` cannot be opened, written, or read.
#[cfg(unix)]
fn confirm_icons(capability: FontCapability, default_yes: bool) -> std::io::Result<bool> {
    use std::io::{BufRead, BufReader, Write};

    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;

    let hint = match capability {
        FontCapability::NerdFont => "detected a Nerd Font, so the default is Yes",
        FontCapability::NoNerdFont => "no Nerd Font detected, so the default is No",
        FontCapability::Unknown => "could not detect your fonts, so the default is No",
    };
    let marker = if default_yes { "[Y/n]" } else { "[y/N]" };

    let mut writer = &tty;
    write!(
        writer,
        "\nGPY can use icon glyphs in your prompt. Here's a sample:\n\n    {SAMPLE_PROMPT_LINE}\n\n\
         Do the icons above display correctly (not boxes or question marks)? {marker}\n\
         ({hint}): ",
    )?;
    writer.flush()?;

    let mut reader = BufReader::new(&tty);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(default_yes);
    }

    let answer = line.trim().to_lowercase();
    if answer.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "mirrors the Unix signature, which can fail on /dev/tty"
)]
fn confirm_icons(_capability: FontCapability, default_yes: bool) -> std::io::Result<bool> {
    Ok(default_yes)
}

/// Print the outcome, with opt-in instructions whenever ASCII was chosen.
fn report(path: &Path, capability: FontCapability, show_icons: bool) {
    output::plain(format_args!("Wrote ui.show_icons to {}", path.display()));

    if show_icons {
        output::detail("Icon style: Nerd Font glyphs (ui.show_icons = true)");
        return;
    }

    match capability {
        FontCapability::NoNerdFont => {
            output::detail(
                "Icon style: ASCII (ui.show_icons = false) — no Nerd Font was detected.",
            );
        }
        FontCapability::Unknown => {
            output::detail(
                "Icon style: ASCII (ui.show_icons = false) — your fonts could not be detected.",
            );
        }
        FontCapability::NerdFont => {
            output::detail("Icon style: ASCII (ui.show_icons = false).");
        }
    }
    output::detail("This renders correctly on any terminal.");
    output::blank();
    output::detail("Want the full icon look? Install a Nerd Font (https://www.nerdfonts.com),");
    output::detail("set it as your terminal font, then run:");
    output::detail_at(2, "gpy config set ui.show_icons true");
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn non_interactive_uses_recommended_default() {
        assert!(resolve_show_icons(FontCapability::NerdFont, true));
        assert!(!resolve_show_icons(FontCapability::NoNerdFont, true));
        assert!(
            !resolve_show_icons(FontCapability::Unknown, true),
            "unknown capability must default to ASCII in non-interactive mode"
        );
    }
}
