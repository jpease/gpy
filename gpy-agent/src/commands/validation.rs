//! Shared resolve/validate/report plumbing for `gpy theme validate` and
//! `gpy palette validate`.

use crate::commands::output;
use crate::config::validation::{ValidationError, ValidationErrorKind};
use crate::{Error, Result};
use std::fmt;
use std::path::{Path, PathBuf};

enum ResolvedTarget {
    Active(String),
    Name(String),
    Path(PathBuf),
}

/// What went wrong while validating a theme or palette; selects the
/// remediation hints, so rewording an error message never drops its hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// A theme color is neither an ANSI name nor a palette entry.
    Color,
    /// A theme icon contains control characters.
    IconControlChars,
    /// The file is not valid TOML.
    Parse,
    /// The file is valid TOML, but a value or table in it is rejected.
    Schema,
    /// The target path exists but is not a file.
    NotAFile,
    /// The name is unsafe or matches no discovered resource.
    UnknownName,
    /// A segment `format` template failed to render.
    Template,
    /// Any other failure.
    Other,
}

impl FailureKind {
    /// The kind a typed error in `error`'s source chain names, if any.
    fn of(error: &Error) -> Self {
        let mut cause = std::error::Error::source(error);
        while let Some(current) = cause {
            if current.is::<toml::de::Error>() {
                return Self::Parse;
            }
            if let Some(validation) = current.downcast_ref::<ValidationError>() {
                match &validation.kind {
                    ValidationErrorKind::InvalidColor { .. } => return Self::Color,
                    ValidationErrorKind::IconControlChars { .. } => return Self::IconControlChars,
                    ValidationErrorKind::TemplateRenderFailed { .. } => return Self::Template,
                    _ => {}
                }
            }
            cause = current.source();
        }
        Self::Other
    }
}

/// A validation [`Error`] tagged with the [`FailureKind`] that picks its hints.
#[derive(Debug)]
pub struct Failure {
    kind: FailureKind,
    error: Error,
}

impl Failure {
    /// Tag `error` with an explicit `kind`.
    #[must_use]
    pub const fn new(kind: FailureKind, error: Error) -> Self {
        Self { kind, error }
    }

    /// Why validation failed.
    #[must_use]
    pub const fn kind(&self) -> FailureKind {
        self.kind
    }

    /// The underlying error.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }
}

impl From<Error> for Failure {
    /// Tag by the typed source `error` carries; [`FailureKind::Other`] without one.
    fn from(error: Error) -> Self {
        Self {
            kind: FailureKind::of(&error),
            error,
        }
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        failure.error
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

/// Resolved and validated target and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// Target label.
    pub target: String,
    /// Active config value, named resource, or file path.
    pub source: &'static str,
}

/// Resource-specific hooks for the shared theme/palette validation flow.
pub trait ValidatableResource {
    /// Human label for this resource, e.g. `"Theme"` / `"Palette"`.
    const KIND: &'static str;
    /// Diagnostic header context, e.g. `"theme validate"`.
    const VALIDATE_CONTEXT: &'static str;
    /// Source label for a target resolved from the active config.
    const ACTIVE_LABEL: &'static str;
    /// Source label for a target resolved as a discovered name.
    const NAME_LABEL: &'static str;
    /// Error returned by `validate()` after the diagnostic was printed.
    const SHORT_ERROR: &'static str;

    /// Read this resource's currently configured value from the active config.
    ///
    /// # Errors
    /// Returns an error if the active config cannot be loaded.
    fn active_value() -> Result<String>;

    /// Validate a discovered resource by name.
    ///
    /// # Errors
    /// Returns the failure if the named resource cannot be loaded or validated.
    fn validate_name(name: &str) -> std::result::Result<(), Failure>;

    /// Validate a concrete resource file by path.
    ///
    /// # Errors
    /// Returns the failure if the file cannot be read, parsed, or validated.
    fn validate_file(path: &Path) -> std::result::Result<(), Failure>;

    /// Build the error for "target path exists but is not a file".
    fn not_a_file_error(path: &Path) -> Error;

    /// Remediation hints for a failure of `kind`.
    fn hints(kind: FailureKind) -> &'static [&'static str];
}

/// Omitted → active config value; existing file → path; otherwise a name.
fn resolve_target<R: ValidatableResource>(
    target: Option<&str>,
) -> std::result::Result<ResolvedTarget, Failure> {
    if let Some(raw_target) = target {
        let path = Path::new(raw_target);
        if path.exists() {
            if path.is_file() {
                return Ok(ResolvedTarget::Path(path.to_path_buf()));
            }
            return Err(Failure::new(
                FailureKind::NotAFile,
                R::not_a_file_error(path),
            ));
        }
        Ok(ResolvedTarget::Name(raw_target.to_owned()))
    } else {
        Ok(ResolvedTarget::Active(R::active_value()?))
    }
}

/// Resolve and validate a target for resource `R`.
///
/// # Errors
/// Returns the failure if the resource fails to parse/validate.
pub fn validate_target<R: ValidatableResource>(
    target: Option<&str>,
) -> std::result::Result<ValidationReport, Failure> {
    match resolve_target::<R>(target)? {
        ResolvedTarget::Active(name) => {
            R::validate_name(&name)?;
            Ok(ValidationReport {
                target: name,
                source: R::ACTIVE_LABEL,
            })
        }
        ResolvedTarget::Name(name) => {
            R::validate_name(&name)?;
            Ok(ValidationReport {
                target: name,
                source: R::NAME_LABEL,
            })
        }
        ResolvedTarget::Path(path) => {
            R::validate_file(&path)?;
            Ok(ValidationReport {
                target: path.display().to_string(),
                source: "explicit file path",
            })
        }
    }
}

/// Validate a resource strictly by name, never treating it as a file path.
///
/// # Errors
/// Returns the failure if the named resource cannot be loaded or validated.
pub fn validate_by_name<R: ValidatableResource>(
    name: &str,
) -> std::result::Result<ValidationReport, Failure> {
    R::validate_name(name)?;
    Ok(ValidationReport {
        target: name.to_owned(),
        source: R::NAME_LABEL,
    })
}

/// Print remediation-focused diagnostics for a resource validation failure.
pub fn print_validation_error<R: ValidatableResource>(context: &str, failure: &Failure) {
    output::fail(format_args!("{context} failed"));
    output::detail(format_args!("reason: {failure}"));
    output::detail("remediation:");
    for hint in R::hints(failure.kind()) {
        output::detail(format_args!("- {hint}"));
    }
}

/// Validate a target for resource `R`, printing a success summary or diagnostic.
///
/// # Errors
/// Returns a short generic error so the caller doesn't reprint the diagnostic.
pub fn validate<R: ValidatableResource>(target: Option<&str>) -> Result<()> {
    match validate_target::<R>(target) {
        Ok(report) => {
            output::ok(format_args!("{} validation passed", R::KIND));
            output::detail(format_args!("target: {}", report.target));
            output::detail(format_args!("source: {}", report.source));
            Ok(())
        }
        Err(failure) => {
            print_validation_error::<R>(R::VALIDATE_CONTEXT, &failure);
            Err(Error::config(R::SHORT_ERROR.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Failure, FailureKind};
    use crate::Error;
    use crate::config::validation::{ValidationError, ValidationErrorKind};

    fn invalid_color() -> Error {
        ValidationError {
            field_path: "ui.root_prompt_color".to_owned(),
            kind: ValidationErrorKind::InvalidColor {
                value: "nonsuch".to_owned(),
                theme: "t".to_owned(),
            },
        }
        .into()
    }

    #[test]
    fn kind_follows_the_typed_source_not_the_message() {
        assert_eq!(Failure::from(invalid_color()).kind(), FailureKind::Color);

        let icon: Error = ValidationError {
            field_path: "ui.icon".to_owned(),
            kind: ValidationErrorKind::IconControlChars {
                theme: "t".to_owned(),
            },
        }
        .into();
        assert_eq!(Failure::from(icon).kind(), FailureKind::IconControlChars);

        let toml_error = toml::from_str::<toml::Table>("= nope").expect_err("invalid toml");
        assert_eq!(
            Failure::from(Error::from(toml_error)).kind(),
            FailureKind::Parse
        );
    }

    #[test]
    fn identical_text_without_a_typed_source_is_other() {
        let lookalike = Error::config("Invalid color 'nonsuch' for field x in theme t");
        assert_eq!(Failure::from(lookalike).kind(), FailureKind::Other);
    }
}
