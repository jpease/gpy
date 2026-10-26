//! Line styles for CLI output.
//!
//! Status markers are ANSI-colored `✓`/`✗`/`!` on an interactive terminal and
//! ASCII (`ok:`, `FAIL:`, `warn:`) when the stream is not a TTY, `NO_COLOR` is
//! set, `TERM=dumb`, or `ui.show_icons = false`. Single-action confirmations
//! print plain text and do not use markers.
//!
//! Every report shares one layout: a [`heading`] (or [`section`]) at column 0,
//! then [`detail`] lines indented one [`INDENT`] per nesting depth.

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

/// One level of detail indentation.
pub const INDENT: &str = "   ";

/// The leading whitespace of a line nested `depth` levels deep.
#[must_use]
pub fn indent(depth: usize) -> String {
    INDENT.repeat(depth)
}

/// An `=` rule as wide as `title`.
#[must_use]
pub fn underline(title: &str) -> String {
    "=".repeat(title.chars().count())
}

/// Print `title`, an `=` rule as wide as it, and a blank line.
pub fn heading(title: impl Display) {
    let text = title.to_string();
    println!("{text}\n{}\n", underline(&text));
}

/// Print a bracketed group label such as `[Agent]`.
pub fn section(title: impl Display) {
    println!("[{title}]");
}

/// Print an empty line.
pub fn blank() {
    println!();
}

/// Print `message` at column 0 without a marker.
pub fn plain(message: impl Display) {
    println!("{message}");
}

/// Print `label...` indented one level, leaving the line open for the result
/// that [`ok`], [`fail`], [`warn`] or [`plain`] completes.
pub fn step(label: impl Display) {
    print!("{INDENT}{label}... ");
}

/// Print `message` indented one level.
pub fn detail(message: impl Display) {
    detail_at(1, message);
}

/// Print `message` indented `depth` levels.
pub fn detail_at(depth: usize, message: impl Display) {
    println!("{}{message}", indent(depth));
}

/// Print `message` behind the marker for `status`, indented `depth` levels.
pub fn detail_status(depth: usize, status: Status, message: impl Display) {
    println!("{}{} {message}", indent(depth), marker(status));
}

#[cfg(test)]
mod tests {
    use super::{INDENT, indent, underline};

    #[test]
    fn underline_matches_title_width_in_characters() {
        assert_eq!(underline("Available Themes:"), "=".repeat(17));
        assert_eq!(underline("naïve"), "=====");
        assert_eq!(underline(""), "");
    }

    #[test]
    fn indent_is_one_unit_per_depth() {
        assert_eq!(indent(0), "");
        assert_eq!(indent(1), INDENT);
        assert_eq!(indent(3), INDENT.repeat(3));
    }
}
