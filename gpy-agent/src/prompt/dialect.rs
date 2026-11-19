//! What differs between the Bash `PS1`, the Zsh `PROMPT` and the Fish prompt
//! that `gpy prompt` prints: the agent formats, the zero-width markers, the
//! escape rules and the clock.

use crate::formatter::{Format, PromptDialect};
use crate::shell::Shell;

/// The shell a prompt is drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Dialect {
    /// Bash `PS1` under `promptvars`.
    Bash,
    /// Zsh `PROMPT` under `prompt_subst`.
    Zsh,
    /// Fish: the bytes `fish_prompt` prints, plain ANSI.
    Fish,
}

impl Dialect {
    pub(crate) const fn format(self) -> Format {
        match self {
            Self::Bash => Format::BashPrompt,
            Self::Zsh => Format::ZshPrompt,
            Self::Fish => Format::Ansi,
        }
    }

    pub(crate) const fn prompt_dialect(self) -> PromptDialect {
        match self {
            Self::Bash => PromptDialect::BashPrompt,
            Self::Zsh => PromptDialect::ZshPrompt,
            Self::Fish => PromptDialect::Ansi,
        }
    }

    /// The `format` of a data request.
    pub(crate) const fn request_format(self) -> &'static str {
        match self {
            Self::Bash => "bash-prompt",
            Self::Zsh => "zsh-prompt",
            Self::Fish => "ansi",
        }
    }

    pub(crate) const fn shell(self) -> Shell {
        match self {
            Self::Bash => Shell::Bash,
            Self::Zsh => Shell::Zsh,
            Self::Fish => Shell::Fish,
        }
    }

    /// The `shell` of a registration.
    pub(crate) const fn request_shell(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }

    /// `sequence` marked zero-width: `\[…\]`, `%{…%}`, or as is in Fish,
    /// which measures escape sequences itself.
    pub(crate) fn invisible(self, sequence: &str) -> String {
        match self {
            Self::Bash => format!("\\[{sequence}\\]"),
            Self::Zsh => format!("%{{{sequence}%}}"),
            Self::Fish => sequence.to_owned(),
        }
    }

    /// The clock: Bash's live `\D{spec}`, Zsh's live `%D{spec}`, or for
    /// Fish the local time now (Fish has no time token; its shell path ran
    /// `date`).
    pub(crate) fn clock_token(self, spec: &str) -> String {
        match self {
            Self::Bash => format!("\\D{{{spec}}}"),
            Self::Zsh => format!("%D{{{spec}}}"),
            Self::Fish => local_time(spec),
        }
    }

    /// Escape data for the prompt, as the shell's `__gpy_prompt_escape`:
    /// Bash (`promptvars`) needs four backslashes for `\` and two for `$`
    /// and a backtick; Zsh (`prompt_subst`) one each, and `%%` for `%`. Fish
    /// prints the prompt with `printf '%s'`: nothing to escape.
    pub(crate) fn escape(self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            match (self, ch) {
                (Self::Bash, '\\') => out.push_str("\\\\\\\\"),
                (Self::Bash, '$') => out.push_str("\\\\$"),
                (Self::Bash, '`') => out.push_str("\\\\`"),
                (Self::Zsh, '\\') => out.push_str("\\\\"),
                (Self::Zsh, '$') => out.push_str("\\$"),
                (Self::Zsh, '`') => out.push_str("\\`"),
                (Self::Zsh, '%') => out.push_str("%%"),
                _ => out.push(ch),
            }
        }
        out
    }
}

/// The local time now as `strftime` `spec` formats it, in the zone `TZ` or
/// `/etc/localtime` names (UTC when neither resolves, as `date` does); empty
/// when the spec does not format.
pub(crate) fn local_time(spec: &str) -> String {
    use std::fmt::Write as _;
    let now = jiff::Timestamp::now().to_zoned_datetime(jiff::tz::TimeZone::system());
    let mut out = String::new();
    if write!(out, "{}", now.strftime(spec)).is_err() {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Dialect;

    /// The escapes `__gpy_prompt_escape` writes in each shell.
    #[test]
    fn escape_follows_each_shell() {
        assert_eq!(Dialect::Bash.escape(r"a\b$c`d%"), r"a\\\\b\\$c\\`d%");
        assert_eq!(Dialect::Zsh.escape(r"a\b$c`d%"), r"a\\b\$c\`d%%");
    }
}
