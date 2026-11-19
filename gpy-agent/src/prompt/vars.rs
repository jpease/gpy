//! The theme values the renderer reads
//! ([`prompt_shell_values`](crate::theme::export::prompt_shell_values)), with
//! two default forms: `${name-default}` ([`ShellVars::or`]) and
//! `${name:-default}` ([`ShellVars::or_nonempty`]).

use std::collections::HashMap;

/// Variable name to value; a name without an entry is unset.
#[derive(Debug, Default)]
pub(crate) struct ShellVars(HashMap<String, String>);

impl ShellVars {
    /// Wrap the theme values.
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

    /// The value when set, even empty (`${name+x}` holds).
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Whether `name` holds a non-empty value (`[[ -n ${name:-} ]]`).
    pub(crate) fn is_set(&self, name: &str) -> bool {
        self.0.get(name).is_some_and(|value| !value.is_empty())
    }
}
