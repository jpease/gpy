//! Shell identification

use clap::{ValueEnum, builder::PossibleValue};
use serde::{Deserialize, Serialize};

/// Supported shells; serialized as the lowercase name.
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
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::Zsh => "zsh",
            Self::Bash => "bash",
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
