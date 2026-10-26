//! GPY agent: Git status and language detection for the Bash, Fish, and Zsh
//! prompts, served by a background daemon over IPC.
//!
//! Not published to crates.io. Modules are public only for integration tests
//! and carry no semver guarantee; the versioned contracts are the IPC schemas
//! (`schemas/`) and the CLI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![expect(clippy::print_stdout, reason = "CLI tool needs stdout output")]
#![expect(clippy::print_stderr, reason = "error reporting to stderr")]

/// The main agent process logic and lifecycle management.
pub mod agent;
/// Cache policy abstractions and timing parameters.
pub mod cache;
/// CLI command implementations.
pub mod commands;
/// Configuration loading and schema definition.
pub mod config;
/// Bounded-wait deadlines for polling loops.
mod deadline;
/// Two-level logging: opt-in `debug_log!` and always-visible `warn_log!`.
pub mod debug;
/// Centralized error and result types.
pub mod error;
/// Nerd Font capability detection for icon defaults.
pub mod font;
/// Output formatter layer (JSON, Fish, ANSI, etc.).
pub mod formatter;
/// Small filesystem predicates shared across handlers.
pub mod fs_util;
/// Git repository status detection.
pub mod git;
/// Importers for third-party prompt configurations (Starship).
pub mod import;
/// Inter-process communication (IPC) server and protocol.
pub mod ipc;
/// Programming language detection and version parsing.
pub mod language;
/// Named color palette loading and conversion.
pub mod palette;
/// Pure path resolution for the runtime, cache, and config roots.
pub mod paths;
/// Extensibility framework.
pub mod plugin;
/// Shared subprocess wait-with-timeout primitive.
pub mod process;
/// Simple profiling utilities for performance analysis.
pub mod profiling;
/// Security features like path validation and rate limiting.
pub mod security;
/// Shell identification and syntax templates.
pub mod shell;
/// Poison-recovering lock acquisition (ADR-0008).
mod sync_ext;
/// Starship-compatible prompt template engine (parser + evaluator).
pub mod template;
/// Theme management with hot-reload support.
pub mod theme;
/// Filesystem watching for real-time updates.
pub mod watcher;
/// Windows Subsystem for Linux detection.
pub mod wsl;

pub use error::{Error, Result};
pub use shell::Shell;

/// Agent version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
