//! The theme-export variables the Bash renderer reads, with its two default
//! forms: `${name-default}` ([`ShellVars::or`]) and `${name:-default}`
//! ([`ShellVars::or_nonempty`]).

use std::collections::HashMap;

/// Variable name to value, as the shell holds them after sourcing the export.
#[derive(Debug, Default)]
pub(crate) struct ShellVars(HashMap<String, String>);

impl ShellVars {
    /// Wrap the export's values.
    pub(crate) const fn new(values: HashMap<String, String>) -> Self {
        Self(values)
    }

    /// `${name-default}`: the value when set, even empty.
    pub(crate) fn or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.0.get(name).map_or(default, String::as_str)
    }

    /// `${name:-default}`: the value when set and non-empty.
    pub(crate) fn or_nonempty<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        match self.0.get(name) {
            Some(value) if !value.is_empty() => value,
            _ => default,
        }
    }

    /// Whether `name` holds a non-empty value (`[[ -n ${name:-} ]]`).
    pub(crate) fn is_set(&self, name: &str) -> bool {
        self.0.get(name).is_some_and(|value| !value.is_empty())
    }
}
