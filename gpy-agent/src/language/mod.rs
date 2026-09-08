//! Programming language detection and version analysis
//!
//! This module provides fast and accurate programming language detection for
//! directories, along with version detection for common languages.
//!
//! # Architecture
//!
//! - **`detector.rs`** - Core detection logic over `gengo-language`'s matcher tables
//! - **`filters.rs`** - Linguist vendor/documentation exclusion globs for the walk
//! - **`cache.rs`** - version-lookup cache, 24h default TTL (configurable via `language.cache_ttl_hours`; reduces I/O)
//! - **`version.rs`** - Version string parsing and extraction from tool output
//!
//! # Performance
//!
//! - **Detection**: 10-100ms depending on file count in directory
//! - **Version lookup**: Cached for 24 hours (configurable via `language.cache_ttl_hours`)
//! - **Cache hit**: <1ms for version retrieval
//!
//! # Supported Languages
//!
//! Currently detects and provides version info for:
//! - Rust (via `rustc --version`)
//! - Node.js (via `node --version`)
//! - Python (via `python --version`)
//! - Go (via `go version`)
//! - Java (via `java -version`)
//! - Ruby (via `ruby --version`)
//! - Swift (via `swift --version`)
//! - Elixir (via `elixir --version`)
//! - Erlang (via `erl -eval 'io:format("~s~n", [erlang:system_info(otp_release)]), halt().' -noshell`)
//! - Fish (via `fish --version`)
//!
//! # Example
//!
//! ```rust,no_run
//! use gpy_agent::language::detector::Detector;
//! use std::path::Path;
//!
//! let path = Path::new("/path/to/project");
//! let languages = Detector::detect_directory(path);
//!
//! for lang in languages {
//!     println!("{}: {} files, {} bytes", lang.name, lang.file_count, lang.total_bytes);
//! }
//! ```
//!
//! # Configuration
//!
//! Language detection can be configured in `~/.config/gpy/config.toml`:
//!
//! ```toml
//! [language]
//! enabled = true                    # Enable/disable language segment
//! show_versions = true              # Show version numbers in prompt
//! cache_ttl_hours = 24              # How long to cache version lookups
//! enabled_languages = []            # Empty = all, or specify: ["rust", "node", "python"]
//! ```

pub mod cache;
pub mod detection_cache;
pub mod detector;
/// Building [`crate::ipc::LanguageInfo`] display entries from detected
/// languages (filtering, version probing, color assignment).
pub mod display;
mod filters;
/// Language metadata (aliases, icons, default colors).
pub mod metadata;
pub mod venv;
pub mod version;

pub use detection_cache::DetectionCache;
pub use detector::{DetectedLanguage, Detector};
