//! Shared resolve/validate/report plumbing for `gpy theme validate` and
//! `gpy palette validate`.
//!
//! Both commands resolve a CLI target (omitted → active config value; an
//! existing file path → explicit file; anything else → a discovered name),
//! validate it, and either print a short success summary or a detailed
//! diagnostic. That shape is identical between themes and palettes; only the
//! label strings and the actual name/file validators differ. This module
//! factors the shared shape into one implementation via [`ValidatableResource`]
//! so `commands::theme` and `commands::palette` each provide only their
//! resource-specific bits.

use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// CLI input resolved into one of three concrete validation targets.
enum ResolvedTarget {
    /// No explicit target was given; this is the active config's value.
    Active(String),
    /// An explicit target that is not an existing file; treated as a name.
    Name(String),
    /// An explicit target that exists as a file on disk.
    Path(PathBuf),
}

/// Outcome of resolving and validating a target, before the caller wraps it
/// into its own public, resource-specific result type (e.g.
/// `ThemeValidationResult`).
pub struct ValidationReport {
    /// Human-readable target label.
    pub target: String,
    /// Source descriptor (active config value, named resource, or file path).
    pub source: &'static str,
}

/// Resource-specific behavior needed to resolve, validate, and report on a
/// theme- or palette-like target.
///
/// Implementations own the actual validation logic (segment-template
/// validation for themes, plain parsing for palettes) and their
/// already-drifted remediation hint tables; this trait only captures the
/// shared orchestration around them.
pub trait ValidatableResource {
    /// Human label for this resource, e.g. `"Theme"` / `"Palette"`.
    const KIND: &'static str;
    /// Context string used in the diagnostic header for `validate()`, e.g.
    /// `"theme validate"` / `"palette validate"`.
    const VALIDATE_CONTEXT: &'static str;
    /// Source label for a target resolved from the active config.
    const ACTIVE_LABEL: &'static str;
    /// Source label for a target resolved as a discovered name.
    const NAME_LABEL: &'static str;
    /// Short, generic error message returned by `validate()` on failure,
    /// after the detailed diagnostic has already been printed once.
    const SHORT_ERROR: &'static str;

    /// Read this resource's currently configured value from the active config.
    ///
    /// # Errors
    ///
    /// Returns an error if the active config cannot be loaded.
    fn active_value() -> Result<String>;

    /// Validate a discovered resource by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the named resource cannot be loaded or validated.
    fn validate_name(name: &str) -> Result<()>;

    /// Validate a concrete resource file by path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or validated.
    fn validate_file(path: &Path) -> Result<()>;

    /// Build the error for "target path exists but is not a file".
    fn not_a_file_error(path: &Path) -> Error;

    /// Resource-specific remediation hints for a validation error message.
    fn hints(message: &str) -> Vec<&'static str>;
}

/// Resolve CLI input into an explicit validation target for resource `R`.
///
/// # Errors
///
/// Returns an error if the provided target is a path that exists but is not
/// a file, or if the active config cannot be loaded (when `target` is `None`).
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

/// Resolve and validate a target for resource `R`, returning structured
/// information for callers like `doctor`.
///
/// # Errors
///
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

/// Validate a resource strictly by name, bypassing the CWD-sensitive path
/// resolver.
///
/// Use this when the argument is always a resource name (e.g., from
/// `gpy doctor`) and must not be treated as a file path regardless of the
/// working directory.
///
/// # Errors
///
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
    println!("❌ {context} failed");
    println!("   reason: {message}");
    println!("   remediation:");
    for hint in R::hints(&message) {
        println!("   - {hint}");
    }
}

/// Validate a target for resource `R`, printing a short success summary or a
/// detailed diagnostic.
///
/// On failure, the detailed diagnostic is printed exactly once via
/// [`print_validation_error`], and a short, generic error is returned instead
/// of the original detailed error — mirroring `doctor.rs`'s
/// accumulate-then-summarize pattern. This keeps the process exit code
/// non-zero without letting the caller's fallback error printing duplicate
/// the diagnostic already shown.
///
/// # Errors
///
/// Returns a short, generic error when validation fails; the detailed reason
/// has already been printed.
pub fn validate<R: ValidatableResource>(target: Option<&str>) -> Result<()> {
    match validate_target::<R>(target) {
        Ok(report) => {
            println!("✅ {} validation passed", R::KIND);
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
