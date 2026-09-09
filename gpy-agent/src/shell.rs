//! Shell identification and syntax templates

use clap::{ValueEnum, builder::PossibleValue};
use serde::{Deserialize, Serialize};

/// Supported shell types
///
/// Serializes as its canonical lowercase name (`"fish"`, `"zsh"`, `"bash"`),
/// matching [`Shell::as_str`] and the spelling accepted on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    /// Fish shell
    Fish,
    /// Zsh shell
    Zsh,
    /// Bash shell
    Bash,
}

impl Shell {
    /// The canonical lowercase name of this shell.
    ///
    /// This is the exact spelling accepted on the command line and by
    /// [`FromStr`](std::str::FromStr).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::Zsh => "zsh",
            Self::Bash => "bash",
        }
    }

    /// Get the variable assignment syntax for this shell
    ///
    /// # Examples
    /// - Fish: `set -g my_var "value"`
    /// - Zsh: `typeset -g my_var="value"`
    /// - Bash: `export my_var="value"`
    #[must_use]
    pub const fn variable_syntax(self) -> VariableSyntax {
        match self {
            Self::Fish => VariableSyntax {
                prefix: "set -g",
                export_prefix: "set -gx",
                separator: " ",
                quote: "\"",
            },
            Self::Zsh => VariableSyntax {
                prefix: "typeset -g",
                export_prefix: "export",
                separator: "=",
                quote: "\"",
            },
            Self::Bash => VariableSyntax {
                prefix: "export",
                export_prefix: "export",
                separator: "=",
                quote: "\"",
            },
        }
    }
}

impl std::fmt::Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ValueEnum for Shell {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Fish, Self::Zsh, Self::Bash]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self.as_str()))
    }
}

/// Error returned when parsing an unrecognized `Shell` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseShellError(String);

impl std::fmt::Display for ParseShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unrecognized shell: {}", self.0)
    }
}

impl std::error::Error for ParseShellError {}

impl std::str::FromStr for Shell {
    type Err = ParseShellError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fish" => Ok(Self::Fish),
            "zsh" => Ok(Self::Zsh),
            "bash" => Ok(Self::Bash),
            other => Err(ParseShellError(other.to_owned())),
        }
    }
}

/// Variable assignment syntax template
#[derive(Debug, Clone, Copy)]
pub struct VariableSyntax {
    /// Prefix command (e.g., "set -g", "typeset -g")
    pub prefix: &'static str,
    /// Prefix command for exported variables (e.g., "set -gx", "export")
    pub export_prefix: &'static str,
    /// Separator between name and value (e.g., " " or "=")
    pub separator: &'static str,
    /// Quote character for values
    pub quote: &'static str,
}

impl VariableSyntax {
    /// Format a variable assignment (internal/global scope)
    ///
    /// # Example
    /// ```
    /// use gpy_agent::shell::Shell;
    /// let syntax = Shell::Fish.variable_syntax();
    /// let line = syntax.format("my_var", "value");
    /// assert_eq!(line, "set -g my_var \"value\"");
    /// ```
    #[must_use]
    pub fn format(self, name: &str, value: &str) -> String {
        format!(
            "{} {}{}{}{}{}",
            self.prefix, name, self.separator, self.quote, value, self.quote
        )
    }

    /// Format an exported variable assignment (environment variable)
    ///
    /// # Example
    /// ```
    /// use gpy_agent::shell::Shell;
    /// let syntax = Shell::Fish.variable_syntax();
    /// let line = syntax.format_export("MY_ENV_VAR", "value");
    /// assert_eq!(line, "set -gx MY_ENV_VAR \"value\"");
    /// ```
    #[must_use]
    pub fn format_export(self, name: &str, value: &str) -> String {
        format!(
            "{} {}{}{}{}{}",
            self.export_prefix, name, self.separator, self.quote, value, self.quote
        )
    }
}
