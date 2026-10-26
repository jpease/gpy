//! Icon-style bootstrap run by `gpy setup`: pick `ui.show_icons` so the first
//! prompt renders on this machine.
//!
//! Font presence does not prove the terminal uses that font, so on a terminal
//! the user confirms a rendered sample; detection only pre-selects the answer.
//! A set `GPY_NERD_FONT` is the user's answer and skips the prompt.

use crate::Result;
use crate::commands::output;
use crate::commands::utils::{active_config_path, load_active_config, save_config_to};
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

    if Path::new(&path).exists() && !options.redetect {
        println!("GPY config already exists at {path}");
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
    save_config_to(&path, &original, &config, &["ui.show_icons"])?;

    report(&path, capability, show_icons);
    Ok(())
}

/// The recommended value when non-interactive, else the user's answer.
fn resolve_show_icons(capability: FontCapability, non_interactive: bool) -> bool {
    let recommended = font::recommend_show_icons(capability);
    if non_interactive {
        return recommended;
    }
    confirm_icons(capability, recommended).unwrap_or(recommended)
}

/// A prompt line with a git branch (U+E0A0), the Rust icon (U+F1617) and `❯`.
#[cfg(unix)]
fn sample_prompt_line() -> String {
    "\u{e0a0} main   \u{f1617} rust 1.82   \u{276f}".to_owned()
}

/// Ask on `/dev/tty` whether the sample renders; stdin is the installer's
/// piped script, not the user. `None` when there is no controlling terminal.
#[cfg(unix)]
fn confirm_icons(capability: FontCapability, default_yes: bool) -> Option<bool> {
    use std::io::{BufRead, BufReader, Write};

    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;

    let hint = match capability {
        FontCapability::NerdFont => "detected a Nerd Font, so the default is Yes",
        FontCapability::NoNerdFont => "no Nerd Font detected, so the default is No",
        FontCapability::Unknown => "could not detect your fonts, so the default is No",
    };
    let marker = if default_yes { "[Y/n]" } else { "[y/N]" };

    let mut writer = &tty;
    write!(
        writer,
        "\nGPY can use icon glyphs in your prompt. Here's a sample:\n\n    {sample}\n\n\
         Do the icons above display correctly (not boxes or question marks)? {marker}\n\
         ({hint}): ",
        sample = sample_prompt_line(),
    )
    .ok()?;
    writer.flush().ok()?;

    let mut reader = BufReader::new(&tty);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return Some(default_yes);
    }

    let answer = line.trim().to_lowercase();
    if answer.is_empty() {
        return Some(default_yes);
    }
    Some(matches!(answer.as_str(), "y" | "yes"))
}

#[cfg(not(unix))]
fn confirm_icons(_capability: FontCapability, _default_yes: bool) -> Option<bool> {
    None
}

/// Print the outcome, with opt-in instructions whenever ASCII was chosen.
fn report(path: &str, capability: FontCapability, show_icons: bool) {
    println!("Wrote ui.show_icons to {path}");

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
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

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

    #[cfg(unix)]
    #[test]
    fn sample_prompt_line_contains_nerd_glyphs() {
        let sample = sample_prompt_line();
        assert!(sample.contains('\u{f1617}'));
        assert!(sample.contains('\u{e0a0}'));
    }
}
