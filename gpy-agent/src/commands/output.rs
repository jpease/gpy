//! Pass/fail/warn markers for CLI status lines.
//!
//! Markers are ANSI-colored `✓`/`✗`/`!` on an interactive terminal and ASCII
//! (`ok:`, `FAIL:`, `warn:`) when the stream is not a TTY, `NO_COLOR` is set,
//! `TERM=dumb`, or `ui.show_icons = false`. Single-action confirmations print
//! plain text and do not use this module.

use std::fmt::Display;
use std::io::IsTerminal;
use std::sync::LazyLock;

/// Outcome a status line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A check passed.
    Ok,
    /// A check failed.
    Fail,
    /// Something needs attention but did not fail.
    Warn,
}

impl Status {
    const fn glyph(self) -> &'static str {
        match self {
            Self::Ok => "\x1b[32m✓\x1b[0m",
            Self::Fail => "\x1b[31m✗\x1b[0m",
            Self::Warn => "\x1b[33m!\x1b[0m",
        }
    }

    const fn ascii(self) -> &'static str {
        match self {
            Self::Ok => "ok:",
            Self::Fail => "FAIL:",
            Self::Warn => "warn:",
        }
    }
}

/// Whether the environment and config allow emoji, independent of the stream.
static GLYPHS_ALLOWED: LazyLock<bool> = LazyLock::new(|| {
    std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
        && crate::commands::utils::read_config()
            .ok()
            .is_none_or(|config| config.ui.show_icons)
});

const fn pick(status: Status, glyphs: bool) -> &'static str {
    if glyphs {
        status.glyph()
    } else {
        status.ascii()
    }
}

/// The marker for `status` on stdout.
#[must_use]
pub fn marker(status: Status) -> &'static str {
    pick(status, std::io::stdout().is_terminal() && *GLYPHS_ALLOWED)
}

/// Print a passed check.
pub fn ok(message: impl Display) {
    println!("{} {message}", marker(Status::Ok));
}

/// Print a failed check.
pub fn fail(message: impl Display) {
    println!("{} {message}", marker(Status::Fail));
}

/// Print a warning on stdout.
pub fn warn(message: impl Display) {
    println!("{} {message}", marker(Status::Warn));
}

/// Print a warning on stderr.
pub fn warn_stderr(message: impl Display) {
    let marker = pick(
        Status::Warn,
        std::io::stderr().is_terminal() && *GLYPHS_ALLOWED,
    );
    eprintln!("{marker} {message}");
}
