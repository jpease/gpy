//! # GPY Agent - Fast background process for Git status and language detection
//!
//! This library provides the core functionality for the GPY prompt system (Bash, Fish, and Zsh).
//! It handles Git repository analysis, programming language detection, and file watching
//! through a background agent process that communicates via IPC.
//!
//! ## This crate is an application, not a library
//!
//! `gpy-agent` is **not published to crates.io** (`publish = false`). It is
//! distributed as two binaries — the `gpy-agent` daemon and the `gpy` CLI —
//! via tagged release archives and the one-line installer. A registry install
//! could deliver only those binaries, never the `fish/`, `bash/` and `zsh/`
//! trees the prompt renders from, so it would leave a broken install rather
//! than a partial one.
//!
//! The modules below are public because the crate's integration tests consume
//! them externally as `gpy_agent::`. **That surface carries no semver
//! guarantee**: any item here may change or disappear in any release, patch
//! releases included. The versioned contracts are the IPC protocol
//! (`schemas/message.json`, `schemas/response.json`, see `SCHEMA_EVOLUTION.md`)
//! and the CLI.
//!
//! ## Key Modules
//!
//! - [`agent`]: The main agent process logic and lifecycle management.
//! - [`cache`]: Cache policy abstractions and timing parameters.
//! - [`config`]: Configuration loading and schema definition.
//! - [`theme`]: Theme management with hot-reload support.
//! - [`git`]: Git repository status detection.
//! - [`language`]: Programming language detection and version parsing.
//! - [`ipc`]: Inter-process communication (IPC) server and protocol.
//! - [`watcher`]: Filesystem watching for real-time updates.
//! - [`security`]: Security features like path validation and rate limiting.
//! - [`error`]: Centralized error and result types.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Justified clippy exceptions
#![expect(clippy::print_stdout, reason = "CLI tool needs stdout output")]
#![expect(clippy::print_stderr, reason = "error reporting to stderr")]
#![expect(
    clippy::multiple_crate_versions,
    reason = "unavoidable transitive duplicates; the reviewed list is `[bans] skip` in deny.toml"
)]

/// The main agent process logic and lifecycle management.
pub mod agent;
/// Cache policy abstractions and timing parameters.
pub mod cache;
/// CLI command implementations.
pub mod commands;
/// Configuration loading and schema definition.
pub mod config;
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
/// Extensibility framework
pub mod plugin;
/// Shared subprocess wait-with-timeout primitive.
pub mod process;
/// Simple profiling utilities for performance analysis.
pub mod profiling;
/// Security features like path validation and rate limiting.
pub mod security;
/// Shell identification and syntax templates.
pub mod shell;
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

/// Agent version for compatibility checking, from `Cargo.toml`.
///
/// This constant is used for version negotiation with clients and for display
/// in the agent's status information.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
