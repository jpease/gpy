//! Typed settings declared by script segment manifests (`[settings.<key>]`).
//!
//! A declaration gives a setting's type, default and range or choices; the
//! user may override the value in `config.toml` (`[plugin.<segment>]`). Every
//! value reaches the script as the string `GPY_OPT_<key>`.

use super::script::detail;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A setting key (`^[a-z_][a-z0-9_]*$`, at most 64 bytes); the script reads it
/// as `GPY_OPT_<key>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct SettingKey(
    #[cfg_attr(test, schemars(regex(pattern = r"^[a-z_][a-z0-9_]{0,63}$")))] String,
);

impl SettingKey {
    /// Longest accepted key.
    pub const MAX_LEN: usize = 64;

    /// Validate a setting key.
    ///
    /// # Errors
    /// Returns an error naming the key if it is not lowercase snake case.
    pub fn new(key: &str) -> Result<Self> {
        let mut chars = key.chars();
        let valid = key.len() <= Self::MAX_LEN
            && chars
                .next()
                .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if valid {
            Ok(Self(key.to_owned()))
        } else {
            Err(Error::invalid(format!(
                "{key:?} is not a setting key (^[a-z_][a-z0-9_]*$, at most 64 bytes)"
            )))
        }
    }

    /// The key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Environment variable that carries the value to the script.
    #[must_use]
    pub fn env_var(&self) -> String {
        format!("GPY_OPT_{}", self.0)
    }
}

impl<'de> Deserialize<'de> for SettingKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(|e| serde::de::Error::custom(detail(e)))
    }
}

impl fmt::Display for SettingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A setting value: a TOML boolean, integer or string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SettingValue {
    /// `true` or `false`; the script sees `true`/`false`.
    Bool(bool),
    /// A 64-bit integer; the script sees its decimal form.
    Int(i64),
    /// A string without control characters.
    String(String),
}

impl SettingValue {
    const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::String(_) => "string",
        }
    }
}

impl fmt::Display for SettingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::String(value) => f.write_str(value),
        }
    }
}

/// One `[settings.<key>]` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum SettingDecl {
    /// A boolean switch.
    Bool {
        /// Value when the user sets none.
        default: bool,
        /// What the setting does.
        #[serde(default)]
        description: Option<String>,
    },
    /// An integer, optionally bounded (inclusive).
    Int {
        /// Value when the user sets none.
        default: i64,
        /// Smallest accepted value.
        #[serde(default)]
        min: Option<i64>,
        /// Largest accepted value.
        #[serde(default)]
        max: Option<i64>,
        /// What the setting does.
        #[serde(default)]
        description: Option<String>,
    },
    /// A string, optionally limited to `choices`.
    String {
        /// Value when the user sets none.
        default: String,
        /// The accepted values; empty accepts any string.
        #[serde(default)]
        choices: Vec<String>,
        /// What the setting does.
        #[serde(default)]
        description: Option<String>,
    },
}

impl SettingDecl {
    /// The declared default.
    #[must_use]
    pub fn default_value(&self) -> SettingValue {
        match self {
            Self::Bool { default, .. } => SettingValue::Bool(*default),
            Self::Int { default, .. } => SettingValue::Int(*default),
            Self::String { default, .. } => SettingValue::String(default.clone()),
        }
    }

    /// The declared description, if any.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        match self {
            Self::Bool { description, .. }
            | Self::Int { description, .. }
            | Self::String { description, .. } => description.as_deref(),
        }
    }

    /// Type and constraints, for CLI listings (`int 0..=10`, `string bold|dim`).
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Bool { .. } => "bool".to_owned(),
            Self::Int { min, max, .. } if min.is_none() && max.is_none() => "int".to_owned(),
            Self::Int { min, max, .. } => format!(
                "int {}..={}",
                min.map_or_else(String::new, |v| v.to_string()),
                max.map_or_else(String::new, |v| v.to_string())
            ),
            Self::String { choices, .. } if choices.is_empty() => "string".to_owned(),
            Self::String { choices, .. } => format!("string {}", choices.join("|")),
        }
    }

    /// Check the declaration itself: bounds ordered, choices unique and
    /// clean, and the default acceptable.
    ///
    /// # Errors
    /// Returns an error describing the problem (the caller adds the key).
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Int {
                min: Some(min),
                max: Some(max),
                ..
            } if min > max => {
                return Err(Error::invalid(format!("min {min} is above max {max}")));
            }
            Self::String { choices, .. } => {
                for (index, choice) in choices.iter().enumerate() {
                    check_string(choice)?;
                    if choices.iter().take(index).any(|earlier| earlier == choice) {
                        return Err(Error::invalid(format!("choice '{choice}' is listed twice")));
                    }
                }
            }
            _ => {}
        }
        self.check(&self.default_value())
            .map_err(|e| Error::invalid(format!("default: {}", detail(e))))
    }

    /// Accept `value` if it has the declared type and meets the constraints.
    ///
    /// # Errors
    /// Returns an error describing the mismatch (the caller adds the key).
    pub fn check(&self, value: &SettingValue) -> Result<()> {
        match (self, value) {
            (Self::Bool { .. }, SettingValue::Bool(_)) => Ok(()),
            (Self::Int { min, max, .. }, SettingValue::Int(number)) => {
                if let Some(lowest) = min
                    && number < lowest
                {
                    return Err(Error::invalid(format!("{number} is below min {lowest}")));
                }
                if let Some(highest) = max
                    && number > highest
                {
                    return Err(Error::invalid(format!("{number} is above max {highest}")));
                }
                Ok(())
            }
            (Self::String { choices, .. }, SettingValue::String(text)) => {
                check_string(text)?;
                if choices.is_empty() || choices.contains(text) {
                    Ok(())
                } else {
                    Err(Error::invalid(format!(
                        "{text:?} is not one of {}",
                        choices.join(", ")
                    )))
                }
            }
            (declared, _) => Err(Error::invalid(format!(
                "expected {}, got {} {value}",
                declared.type_name(),
                value.type_name()
            ))),
        }
    }

    /// Parse a CLI string (`gpy config set`) as this setting's type and check it.
    ///
    /// # Errors
    /// Returns an error if the text does not parse or fails the constraints.
    pub fn parse_str(&self, raw: &str) -> Result<SettingValue> {
        let value = match self {
            Self::Bool { .. } => SettingValue::Bool(match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => true,
                "false" | "0" | "no" | "off" => false,
                _ => return Err(Error::invalid(format!("{raw:?} is not a boolean"))),
            }),
            Self::Int { .. } => SettingValue::Int(
                raw.trim()
                    .parse()
                    .map_err(|e| Error::invalid(format!("{raw:?} is not an integer: {e}")))?,
            ),
            Self::String { .. } => SettingValue::String(raw.to_owned()),
        };
        self.check(&value)?;
        Ok(value)
    }

    const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool { .. } => "bool",
            Self::Int { .. } => "int",
            Self::String { .. } => "string",
        }
    }
}

/// Strings reach the script through the environment and the prompt, so
/// control characters (NUL included) are refused.
fn check_string(value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        Err(Error::invalid(format!(
            "{value:?} contains a control character"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
