//! Shared resolve/validate/report plumbing for `gpy theme validate` and
//! `gpy palette validate`.

use crate::{Error, Result};
use std::path::{Path, PathBuf};

enum ResolvedTarget {
    Active(String),
    Name(String),
    Path(PathBuf),
}

/// Resolved and validated target, before the caller wraps it in its own type.
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
    /// Returns an error if the named resource cannot be loaded or validated.
    fn validate_name(name: &str) -> Result<()>;

    /// Validate a concrete resource file by path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, parsed, or validated.
    fn validate_file(path: &Path) -> Result<()>;

    /// Build the error for "target path exists but is not a file".
    fn not_a_file_error(path: &Path) -> Error;

    /// Resource-specific remediation hints for a validation error message.
    fn hints(message: &str) -> Vec<&'static str>;
}

/// Omitted → active config value; existing file → path; otherwise a name.
fn resolve_target<R: ValidatableResource>(target: Option<&str>) -> Result<ResolvedTarget> {
    if let Some(raw_target) = target {
        let path = Path::new(raw_target);
        if path.exists() {
            if path.is_file() {
                return Ok(ResolvedTarget::Path(path.to_path_buf()));
            }
            return Err(R::not_a_file_error(path));
        }
        Ok(ResolvedTarget::Name(raw_target.to_owned()))
    } else {
        Ok(ResolvedTarget::Active(R::active_value()?))
    }
}

/// Resolve and validate a target for resource `R`.
///
/// # Errors
/// Returns an error if the resource fails to parse/validate.
pub fn validate_target<R: ValidatableResource>(target: Option<&str>) -> Result<ValidationReport> {
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
/// Returns an error if the named resource cannot be loaded or validated.
pub fn validate_by_name<R: ValidatableResource>(name: &str) -> Result<ValidationReport> {
    R::validate_name(name)?;
    Ok(ValidationReport {
        target: name.to_owned(),
        source: R::NAME_LABEL,
    })
}

/// Print remediation-focused diagnostics for a resource validation error.
pub fn print_validation_error<R: ValidatableResource>(context: &str, error: &Error) {
    let message = error.to_string();
    crate::commands::output::fail(format_args!("{context} failed"));
    println!("   reason: {message}");
    println!("   remediation:");
    for hint in R::hints(&message) {
        println!("   - {hint}");
    }
}

/// Validate a target for resource `R`, printing a success summary or diagnostic.
///
/// # Errors
/// Returns a short generic error so the caller doesn't reprint the diagnostic.
pub fn validate<R: ValidatableResource>(target: Option<&str>) -> Result<()> {
    match validate_target::<R>(target) {
        Ok(report) => {
            crate::commands::output::ok(format_args!("{} validation passed", R::KIND));
            println!("   target: {}", report.target);
            println!("   source: {}", report.source);
            Ok(())
        }
        Err(error) => {
            print_validation_error::<R>(R::VALIDATE_CONTEXT, &error);
            Err(Error::config(R::SHORT_ERROR.to_owned()))
        }
    }
}
