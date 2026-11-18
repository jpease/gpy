//! What differs between the Bash `PS1` and the Zsh `PROMPT` that `gpy prompt`
//! prints: the agent formats, the zero-width markers, the escape rules and the
//! live clock token.

use crate::formatter::{Format, PromptDialect};
use crate::shell::Shell;

/// The shell a prompt is drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Dialect {
    /// Bash `PS1` under `promptvars`.
    Bash,
    /// Zsh `PROMPT` under `prompt_subst`.
    Zsh,
}

impl Dialect {
    pub(crate) const fn format(self) -> Format {
        match self {
            Self::Bash => Format::BashPrompt,
            Self::Zsh => Format::ZshPrompt,
        }
    }

    pub(crate) const fn prompt_dialect(self) -> PromptDialect {
        match self {
            Self::Bash => PromptDialect::BashPrompt,
            Self::Zsh => PromptDialect::ZshPrompt,
        }
    }

    /// The `format` of a data request.
    pub(crate) const fn request_format(self) -> &'static str {
        match self {
            Self::Bash => "bash-prompt",
            Self::Zsh => "zsh-prompt",
        }
    }

    pub(crate) const fn shell(self) -> Shell {
        match self {
            Self::Bash => Shell::Bash,
            Self::Zsh => Shell::Zsh,
        }
    }

    /// `sequence` marked zero-width: `\[…\]` or `%{…%}`.
    pub(crate) fn invisible(self, sequence: &str) -> String {
        match self {
            Self::Bash => format!("\\[{sequence}\\]"),
            Self::Zsh => format!("%{{{sequence}%}}"),
        }
    }

    /// The live clock token: Bash's `\D{spec}`, Zsh's `%D{spec}`.
    pub(crate) fn clock_token(self, spec: &str) -> String {
        match self {
            Self::Bash => format!("\\D{{{spec}}}"),
            Self::Zsh => format!("%D{{{spec}}}"),
        }
    }

    /// Escape data for the prompt, as the shell's `__gpy_prompt_escape`:
    /// Bash (`promptvars`) needs four backslashes for `\` and two for `$`
    /// and a backtick; Zsh (`prompt_subst`) one each, and `%%` for `%`.
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
