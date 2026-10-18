//! Programming language detection and version probing.

pub mod cache;
pub mod detection_cache;
pub mod detector;
/// Building [`crate::ipc::LanguageInfo`] display entries.
pub mod display;
mod filters;
/// Language metadata (aliases, icons, default colors).
pub mod metadata;
mod project_root;
pub mod venv;
pub mod version;

pub use detection_cache::DetectionCache;
pub use detector::{DetectedLanguage, Detector};
#[cfg(test)]
pub(crate) use project_root::age_non_git_root_for_test;
pub use project_root::{ProjectRoot, project_root};
