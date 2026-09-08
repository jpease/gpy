//! `gpy-agent init` — first-run configuration bootstrap.
//!
//! Run by the installers (`install.sh`, `install-oneline.sh`) right after the
//! binary lands. Its one job is to make sure the icon style a fresh user *ends
//! up with* renders on their machine, so the very first prompt is never tofu
//! (#411).
//!
//! Behavior:
//!
//! 1. **Idempotent / non-destructive.** If a config file already exists, it is
//!    left byte-for-byte untouched (existing users, and Nerd-Font users, keep
//!    exactly what they had). Only a genuinely fresh install writes anything.
//! 2. **Capability-aware default.** [`crate::font::detect_nerd_font`] suggests a
//!    default; a machine with no detectable Nerd Font (or an undetectable one)
//!    defaults to ASCII rather than glyphs.
//! 3. **Human confirmation when possible.** Font *presence* does not prove the
//!    terminal is *using* that font, so when a terminal is attached we print a
//!    rendered sample and let the user confirm against their own eyes — the
//!    only reliable oracle. The scan just pre-selects the default answer.
//!    A set `GPY_NERD_FONT` is already the user's answer and skips the
//!    prompt, so provisioning scripts never block on `/dev/tty` (#640).
//! 4. **Clear opt-in on ASCII.** Whenever ASCII is chosen, print exactly how to
//!    switch to the full glyph set later.

use crate::Result;
use crate::commands::utils::{active_config_path, save_config_to};
use crate::config::Config;
use crate::font::{self, FontCapability};
use std::path::Path;

/// Options for [`run`], mirroring the `gpy-agent init` CLI flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct InitOptions {
    /// Skip the interactive confirmation and use the detected capability
    /// directly. Used by non-interactive installs (piped `curl | sh`) and tests.
    pub non_interactive: bool,
    /// Overwrite an existing config file instead of leaving it untouched.
    pub force: bool,
}

/// Run the first-run config bootstrap.
///
/// # Errors
///
/// Returns an error if the config path cannot be resolved or the generated
/// config fails to validate or write.
pub fn run(options: InitOptions) -> Result<()> {
    let path = active_config_path()?;

    if Path::new(&path).exists() && !options.force {
        println!("✅ GPY config already exists at {path}");
        println!("   Leaving your existing settings unchanged.");
        return Ok(());
    }

    let capability = font::detect_nerd_font();
    // A set `GPY_NERD_FONT` is the user's explicit answer, so it must never be
    // followed by a `/dev/tty` prompt: the installers run plain `gpy-agent
    // init`, and a provisioning script executed from a terminal would block
    // on the confirmation while the docs promise a non-interactive choice (#640).
    let non_interactive = options.non_interactive || font::override_from_env().is_some();
    let show_icons = resolve_show_icons(capability, non_interactive);

    let mut config = Config::default();
    config.ui.show_icons = show_icons;
    save_config_to(&config, &path)?;

    report(&path, capability, show_icons);
    Ok(())
}

/// Decide the `show_icons` value from detected capability and interactivity.
///
/// Non-interactive (`--non-interactive`, a set `GPY_NERD_FONT` override, or
/// no attached terminal): use the capability's recommended default.
/// Interactive: show a sample and let the user confirm, with the
/// recommendation pre-selecting the default answer.
fn resolve_show_icons(capability: FontCapability, non_interactive: bool) -> bool {
    let recommended = font::recommend_show_icons(capability);
    if non_interactive {
        return recommended;
    }
    confirm_icons(capability, recommended).unwrap_or(recommended)
}

/// A representative prompt line built from real Nerd Font glyphs.
///
/// So the user is judging the exact kind of glyph the prompt would emit — a git branch icon
/// (U+E0A0), the Rust language icon GPY ships (U+F1617), and the `❯` prompt symbol.
///
#[cfg(unix)]
fn sample_prompt_line() -> String {
    "\u{e0a0} main   \u{f1617} rust 1.82   \u{276f}".to_owned()
}

/// Print the sample and ask the user whether the glyphs render.
///
/// Reads the answer from the controlling terminal (`/dev/tty`) rather than stdin — the
/// installer pipes its script over stdin, so stdin is not the user here.
///
/// Returns `None` when no controlling terminal is attached (e.g. piped
/// `curl | sh` in CI), signalling the caller to fall back to the detected
/// default without prompting.
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
        // EOF (no interactive input available): take the default.
        return Some(default_yes);
    }

    let answer = line.trim().to_lowercase();
    if answer.is_empty() {
        return Some(default_yes);
    }
    Some(matches!(answer.as_str(), "y" | "yes"))
}

/// No `/dev/tty` concept on this platform: never prompt, always fall back to the
/// detected default.
#[cfg(not(unix))]
fn confirm_icons(_capability: FontCapability, _default_yes: bool) -> Option<bool> {
    None
}

/// Print the outcome, including the opt-in instructions whenever ASCII was chosen.
fn report(path: &str, capability: FontCapability, show_icons: bool) {
    println!("✅ Wrote GPY config to {path}");

    if show_icons {
        println!("   Icon style: Nerd Font glyphs (ui.show_icons = true)");
        return;
    }

    match capability {
        FontCapability::NoNerdFont => {
            println!("   Icon style: ASCII (ui.show_icons = false) — no Nerd Font was detected.");
        }
        FontCapability::Unknown => {
            println!(
                "   Icon style: ASCII (ui.show_icons = false) — your fonts could not be detected."
            );
        }
        FontCapability::NerdFont => {
            println!("   Icon style: ASCII (ui.show_icons = false).");
        }
    }
    println!("   This renders correctly on any terminal.");
    println!();
    println!("   Want the full icon look? Install a Nerd Font (https://www.nerdfonts.com),");
    println!("   set it as your terminal font, then run:");
    println!("       gpy config set ui.show_icons true");
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
        assert!(
            sample.contains('\u{f1617}'),
            "sample should include the Rust Nerd glyph the user would actually see"
        );
        assert!(
            sample.contains('\u{e0a0}'),
            "sample should include a branch glyph"
        );
    }
}
