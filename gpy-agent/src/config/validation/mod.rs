//! Validation layer for configuration and theme inputs.

/// Color validation utilities.
pub mod colors;
/// Core configuration validation (non-theme).
pub mod config;
/// Per-segment template validation.
pub mod templates;
/// Theme-related validation logic.
pub mod theme;

pub use colors::is_valid_color;
pub use config::{normalize_config, validate_config};
pub use theme::validate_theme_config;

use crate::Error;

/// A config value failed a structural/semantic validation check.
///
/// `field_path` locates the failing field; `kind` says why.
#[derive(Debug)]
pub struct ValidationError {
    /// Dotted path (or indexed slot) to the offending config field.
    pub field_path: String,
    /// Why the field failed validation.
    pub kind: ValidationErrorKind,
}

/// The reason a [`ValidationError`] was raised.
#[derive(Debug)]
pub enum ValidationErrorKind {
    /// The segment's format template failed to render against the palette.
    TemplateRenderFailed {
        /// Segment whose `format` failed, e.g. `git`.
        segment: &'static str,
        /// The template text that failed to render.
        template: String,
        /// The underlying render failure.
        source: crate::template::TemplateError,
    },
    /// A value did not parse into, or fit the bounds of, the expected type.
    InvalidValue {
        /// A short description of what was expected, e.g. `"agent timeout"`.
        expected: &'static str,
        /// The offending raw value, as given by the caller.
        value: String,
    },
    /// The dotted key does not name any known config field.
    UnknownKey,
    /// The dotted key names a real field, but it cannot be set via `config set`.
    ReadOnlyKey,
    /// A segment name (a `ui.enabled_segments` entry or a theme `[segments.<name>]`
    /// key) uses characters `SegmentName` rejects.
    InvalidSegmentName {
        /// The offending segment name.
        value: String,
    },
    /// Two distinct segment names share one shell variable token, so one
    /// segment's file and styles would shadow the other's.
    SegmentTokenCollision {
        /// The colliding names.
        collision: crate::plugin::SegmentTokenCollision,
    },
    /// Two plugin segment theme fields would export the same shell variable
    /// (segment `a` with an `open` cap and segment `a_open` with an icon).
    PluginVariableCollision {
        /// The colliding fields and the shared variable.
        collision: crate::theme::export::PluginVariableCollision,
    },
    /// A `git.skip_paths` entry is neither an absolute path, `~`, nor `~/…`.
    InvalidSkipPath {
        /// The offending path.
        value: String,
    },
    /// A theme color token is neither a standard ANSI name nor a palette entry.
    InvalidColor {
        /// The offending color value.
        value: String,
        /// The theme the color came from (file path or theme name).
        theme: String,
    },
    /// A theme icon string contains control characters.
    IconControlChars {
        /// The theme the icon came from (file path or theme name).
        theme: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ValidationErrorKind::TemplateRenderFailed {
                segment,
                template,
                source,
            } => write!(f, "segment '{segment}': {source} (template: {template})"),
            ValidationErrorKind::InvalidValue { expected, value } => {
                write!(f, "Invalid {expected}: {value}")
            }
            ValidationErrorKind::UnknownKey => {
                write!(f, "Unknown config key: {}", self.field_path)
            }
            ValidationErrorKind::ReadOnlyKey => {
                write!(f, "Config key {} is read-only", self.field_path)
            }
            ValidationErrorKind::InvalidSegmentName { value } => write!(
                f,
                "Invalid segment name: {value}. Segment names may contain only alphanumeric characters, underscores, and hyphens."
            ),
            ValidationErrorKind::SegmentTokenCollision { collision } => {
                write!(f, "{}: {collision}", self.field_path)
            }
            ValidationErrorKind::PluginVariableCollision { collision } => {
                write!(f, "{}: {collision}", self.field_path)
            }
            ValidationErrorKind::InvalidSkipPath { value } => write!(
                f,
                "{} must be an absolute path, '~', or start with '~/': {value}",
                self.field_path
            ),
            ValidationErrorKind::InvalidColor { value, theme } => write!(
                f,
                "Invalid color '{value}' for field {} in theme {theme}",
                self.field_path
            ),
            ValidationErrorKind::IconControlChars { theme } => write!(
                f,
                "Icon for field {} in theme {theme} contains control characters",
                self.field_path
            ),
        }
    }
}

impl std::error::Error for ValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ValidationErrorKind::TemplateRenderFailed { source, .. } => Some(source),
            ValidationErrorKind::InvalidValue { .. }
            | ValidationErrorKind::UnknownKey
            | ValidationErrorKind::ReadOnlyKey
            | ValidationErrorKind::InvalidSegmentName { .. }
            | ValidationErrorKind::SegmentTokenCollision { .. }
            | ValidationErrorKind::PluginVariableCollision { .. }
            | ValidationErrorKind::InvalidSkipPath { .. }
            | ValidationErrorKind::InvalidColor { .. }
            | ValidationErrorKind::IconControlChars { .. } => None,
        }
    }
}

impl From<ValidationError> for Error {
    fn from(err: ValidationError) -> Self {
        Self::Config {
            message: err.to_string(),
            source: Some(Box::new(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::too_many_lines)]

    use super::{ValidationError, ValidationErrorKind};
    use crate::Error;

    /// Every message shape renders its exact user-facing wording.
    #[test]
    fn display_text_for_every_converted_shape() {
        let cases: &[(ValidationError, &str)] = &[
            (
                ValidationError {
                    field_path: "agent.timeout_seconds".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "agent timeout",
                        value: "0".to_owned(),
                    },
                },
                "Invalid agent timeout: 0",
            ),
            (
                ValidationError {
                    field_path: "agent.supervisor.check_interval_seconds".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "supervisor check interval",
                        value: "0".to_owned(),
                    },
                },
                "Invalid supervisor check interval: 0",
            ),
            (
                ValidationError {
                    field_path: "agent.supervisor.max_restart_attempts".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "supervisor max restart attempts",
                        value: "abc".to_owned(),
                    },
                },
                "Invalid supervisor max restart attempts: abc",
            ),
            (
                ValidationError {
                    field_path: "git.timeout_seconds".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "git timeout",
                        value: "0".to_owned(),
                    },
                },
                "Invalid git timeout: 0",
            ),
            (
                ValidationError {
                    field_path: "git.max_branch_length".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "max branch length",
                        value: "abc".to_owned(),
                    },
                },
                "Invalid max branch length: abc",
            ),
            (
                ValidationError {
                    field_path: "language.cache_ttl_hours".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "cache TTL",
                        value: "abc".to_owned(),
                    },
                },
                "Invalid cache TTL: abc",
            ),
            (
                ValidationError {
                    field_path: "language.confidence_threshold".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "threshold",
                        value: "abc".to_owned(),
                    },
                },
                "Invalid threshold: abc",
            ),
            (
                ValidationError {
                    field_path: "language.confidence_threshold".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "confidence threshold",
                        value: "2.0".to_owned(),
                    },
                },
                "Invalid confidence threshold: 2.0",
            ),
            (
                ValidationError {
                    field_path: "ui.theme".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "theme name",
                        value: String::new(),
                    },
                },
                "Invalid theme name: ",
            ),
            (
                ValidationError {
                    field_path: "ui.directory.truncation_length".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "truncation length",
                        value: "0".to_owned(),
                    },
                },
                "Invalid truncation length: 0",
            ),
            (
                ValidationError {
                    field_path: "ui.directory.truncation_symbol".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "truncation symbol",
                        value: String::new(),
                    },
                },
                "Invalid truncation symbol: ",
            ),
            (
                ValidationError {
                    field_path: "ui.directory.max_length".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "max path length",
                        value: "0".to_owned(),
                    },
                },
                "Invalid max path length: 0",
            ),
            (
                ValidationError {
                    field_path: "language.confidence_threshold".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "threshold",
                        value: "high".to_owned(),
                    },
                },
                "Invalid threshold: high",
            ),
            (
                ValidationError {
                    field_path: "agent.enabled".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "boolean value",
                        value: "maybe".to_owned(),
                    },
                },
                "Invalid boolean value: maybe",
            ),
            (
                ValidationError {
                    field_path: "agent.timeout_seconds".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "whole number",
                        value: "-1".to_owned(),
                    },
                },
                "Invalid whole number: -1",
            ),
            (
                ValidationError {
                    field_path: "agent.supervisor.max_restart_attempts".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "whole number",
                        value: "-1".to_owned(),
                    },
                },
                "Invalid whole number: -1",
            ),
            (
                ValidationError {
                    field_path: "git.max_branch_length".to_owned(),
                    kind: ValidationErrorKind::InvalidValue {
                        expected: "whole number",
                        value: "-1".to_owned(),
                    },
                },
                "Invalid whole number: -1",
            ),
            (
                ValidationError {
                    field_path: "not.a.key".to_owned(),
                    kind: ValidationErrorKind::UnknownKey,
                },
                "Unknown config key: not.a.key",
            ),
            (
                ValidationError {
                    field_path: "ui.enabled_segments".to_owned(),
                    kind: ValidationErrorKind::ReadOnlyKey,
                },
                "Config key ui.enabled_segments is read-only",
            ),
            (
                ValidationError {
                    field_path: "ui.enabled_segments[1]".to_owned(),
                    kind: ValidationErrorKind::InvalidSegmentName {
                        value: "bad seg".to_owned(),
                    },
                },
                "Invalid segment name: bad seg. Segment names may contain only alphanumeric characters, underscores, and hyphens.",
            ),
            (
                ValidationError {
                    field_path: "git.skip_paths[0]".to_owned(),
                    kind: ValidationErrorKind::InvalidSkipPath {
                        value: "relative/path".to_owned(),
                    },
                },
                "git.skip_paths[0] must be an absolute path, '~', or start with '~/': relative/path",
            ),
            (
                ValidationError {
                    field_path: "style".to_owned(),
                    kind: ValidationErrorKind::InvalidColor {
                        value: "nonsuch".to_owned(),
                        theme: "my-theme".to_owned(),
                    },
                },
                "Invalid color 'nonsuch' for field style in theme my-theme",
            ),
            (
                ValidationError {
                    field_path: "symbol".to_owned(),
                    kind: ValidationErrorKind::IconControlChars {
                        theme: "my-theme".to_owned(),
                    },
                },
                "Icon for field symbol in theme my-theme contains control characters",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), *expected, "field_path={}", err.field_path);
        }
    }

    /// `Error::Config` built from a `ValidationError` downcasts back via `source()`.
    #[test]
    fn error_config_source_downcasts_to_validation_error_for_every_kind() {
        let samples: Vec<ValidationError> = vec![
            ValidationError {
                field_path: "segments.duration.format".to_owned(),
                kind: ValidationErrorKind::TemplateRenderFailed {
                    segment: "duration",
                    template: "[$duration](fg:nonsuch)".to_owned(),
                    source: crate::template::render(
                        "[$duration](fg:nonsuch)",
                        &crate::template::RenderContext::new(&NoopResolver)
                            .with_palette(crate::template::Palette::default()),
                    )
                    .unwrap_err(),
                },
            },
            ValidationError {
                field_path: "agent.timeout_seconds".to_owned(),
                kind: ValidationErrorKind::InvalidValue {
                    expected: "agent timeout",
                    value: "0".to_owned(),
                },
            },
            ValidationError {
                field_path: "not.a.key".to_owned(),
                kind: ValidationErrorKind::UnknownKey,
            },
            ValidationError {
                field_path: "ui.enabled_segments".to_owned(),
                kind: ValidationErrorKind::ReadOnlyKey,
            },
            ValidationError {
                field_path: "ui.enabled_segments[0]".to_owned(),
                kind: ValidationErrorKind::InvalidSegmentName {
                    value: "bad seg".to_owned(),
                },
            },
            ValidationError {
                field_path: "git.skip_paths[0]".to_owned(),
                kind: ValidationErrorKind::InvalidSkipPath {
                    value: "relative/path".to_owned(),
                },
            },
            ValidationError {
                field_path: "style".to_owned(),
                kind: ValidationErrorKind::InvalidColor {
                    value: "nonsuch".to_owned(),
                    theme: "my-theme".to_owned(),
                },
            },
            ValidationError {
                field_path: "symbol".to_owned(),
                kind: ValidationErrorKind::IconControlChars {
                    theme: "my-theme".to_owned(),
                },
            },
        ];

        for sample in samples {
            let field_path = sample.field_path.clone();
            let display = sample.to_string();
            let err: Error = sample.into();

            let Error::Config { message, source } = &err else {
                panic!("expected Error::Config, got {err:?}");
            };
            assert_eq!(*message, display, "message must equal Display text");

            let source_ref = source.as_ref().expect("source must be Some");
            let downcast = source_ref
                .downcast_ref::<ValidationError>()
                .expect("source must downcast back to ValidationError");
            assert_eq!(downcast.field_path, field_path);
        }
    }

    struct NoopResolver;
    impl crate::template::VariableResolver for NoopResolver {
        fn resolve(&self, _name: &str) -> Option<String> {
            Some("x".to_owned())
        }
    }
}
