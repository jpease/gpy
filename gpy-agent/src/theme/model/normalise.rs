//! The normalisation `N` of ADR-0012 section 4: a theme serialized to a table
//! with each defaulted `Option` field treated as unset, so an absent field and
//! one set to the default its accessor supplies compare equal. `gpy theme
//! rebase` and `gpy theme diff` compare themes through it.

use super::ThemeConfig;
use crate::{Error, Result};

/// The default an accessor supplies for an unset `Option` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fallback {
    /// A `bool` default.
    Bool(bool),
    /// A string default.
    Str(&'static str),
}

impl Fallback {
    /// The default as the value the serializer writes for it.
    fn value(self) -> toml::Value {
        match self {
            Self::Bool(value) => toml::Value::Boolean(value),
            Self::Str(value) => toml::Value::String(value.to_owned()),
        }
    }
}

/// Every `Option` field read through an accessor that supplies a default, as
/// `(segment, field, default)`. Fields where `None` has its own meaning (such
/// as `format`) are not listed.
const DEFAULTED_OPTIONS: [(&str, &str, Fallback); 7] = [
    ("clock", "time_format", Fallback::Str("12")),
    ("clock", "show_leading_zero", Fallback::Bool(false)),
    ("clock", "show_seconds", Fallback::Bool(false)),
    ("git", "show_counts", Fallback::Bool(true)),
    ("git", "show_branch", Fallback::Bool(true)),
    ("git", "show_ahead_behind", Fallback::Bool(true)),
    ("git", "show_stash", Fallback::Bool(true)),
];

/// `N(Table::try_from(theme))`: `theme` serialized to a table, without each
/// defaulted `Option` field whose value equals its accessor's default.
///
/// # Errors
/// Returns an error if the theme can't be serialized.
pub fn normalised_table(theme: &ThemeConfig) -> Result<toml::Table> {
    let mut table = toml::Table::try_from(theme)
        .map_err(|e| Error::config(format!("failed to serialize theme to TOML: {e}")))?;
    normalise(&mut table);
    Ok(table)
}

/// Remove from a serialized theme `table` each defaulted `Option` field whose
/// value equals its accessor's default.
pub fn normalise(table: &mut toml::Table) {
    let Some(toml::Value::Table(segments)) = table.get_mut("segments") else {
        return;
    };
    for (segment, field, default) in DEFAULTED_OPTIONS {
        if let Some(toml::Value::Table(section)) = segments.get_mut(segment)
            && section.get(field) == Some(&default.value())
        {
            section.remove(field);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ClockTheme, GitTheme};

    /// The value each listed field's accessor returns on an unset theme.
    fn accessor_default(segment: &str, field: &str) -> toml::Value {
        let clock = ClockTheme::default();
        let git = GitTheme::default();
        match (segment, field) {
            ("clock", "time_format") => toml::Value::String(clock.time_format().to_owned()),
            ("clock", "show_leading_zero") => toml::Value::Boolean(clock.show_leading_zero()),
            ("clock", "show_seconds") => toml::Value::Boolean(clock.show_seconds()),
            ("git", "show_counts") => toml::Value::Boolean(git.show_counts()),
            ("git", "show_branch") => toml::Value::Boolean(git.show_branch()),
            ("git", "show_ahead_behind") => toml::Value::Boolean(git.show_ahead_behind()),
            ("git", "show_stash") => toml::Value::Boolean(git.show_stash()),
            _ => panic!("no accessor for {segment}.{field}"),
        }
    }

    #[test]
    fn listed_defaults_match_the_accessors() {
        for (segment, field, default) in DEFAULTED_OPTIONS {
            assert_eq!(
                accessor_default(segment, field),
                default.value(),
                "{segment}.{field}"
            );
        }
    }

    #[test]
    fn a_field_set_to_its_default_normalises_as_unset() {
        let mut theme = ThemeConfig::default();
        let clock = &mut theme.segments.clock;
        clock.time_format = Some(clock.time_format().to_owned());
        clock.show_leading_zero = Some(clock.show_leading_zero());
        clock.show_seconds = Some(clock.show_seconds());
        let git = &mut theme.segments.git;
        git.show_counts = Some(git.show_counts());
        git.show_branch = Some(git.show_branch());
        git.show_ahead_behind = Some(git.show_ahead_behind());
        git.show_stash = Some(git.show_stash());
        assert_eq!(
            normalised_table(&theme).expect("normalise set defaults"),
            normalised_table(&ThemeConfig::default()).expect("normalise default"),
        );
    }

    #[test]
    fn a_field_set_off_its_default_is_kept() {
        let mut theme = ThemeConfig::default();
        theme.segments.clock.show_seconds = Some(true);
        theme.segments.git.show_stash = Some(false);
        let table = normalised_table(&theme).expect("normalise");
        let segment = |name: &str| {
            table
                .get("segments")
                .and_then(|segments| segments.get(name))
                .expect("segment present")
                .clone()
        };
        assert_eq!(
            segment("clock").get("show_seconds"),
            Some(&toml::Value::Boolean(true))
        );
        assert_eq!(
            segment("git").get("show_stash"),
            Some(&toml::Value::Boolean(false))
        );
    }

    #[test]
    fn format_is_not_normalised() {
        assert!(
            DEFAULTED_OPTIONS
                .iter()
                .all(|(_, field, _)| *field != "format"),
            "None has its own meaning for format"
        );
    }
}
