//! Centralized error handling for GPY Agent
//!
//! Provides a unified error type that can represent failures from any subsystem
//! while maintaining good error context and user-friendly messages.

use thiserror::Error;

/// Main error type for GPY Agent operations
#[derive(Error, Debug)]
pub enum Error {
    /// Git repository operation failed
    #[error("Git operation failed: {message}")]
    Git {
        /// A description of the error.
        message: String,
        /// The underlying error source, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Language detection failed
    #[error("Language detection failed: {message}")]
    Language {
        /// A description of the error.
        message: String,
        /// The underlying error source, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// IPC communication failed
    #[error("IPC communication failed: {message}")]
    Ipc {
        /// A description of the error.
        message: String,
        /// The underlying error source, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// File watching failed
    #[error("File watcher failed: {message}")]
    Watcher {
        /// A description of the error.
        message: String,
        /// The underlying error source, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Configuration loading/validation failed
    #[error("Configuration error: {message}")]
    Config {
        /// A description of the error.
        message: String,
        /// The underlying error source, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Agent process management failed
    #[error("Agent process error: {message}")]
    Agent {
        /// A description of the error.
        message: String,
        /// The underlying error source, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// I/O operation failed
    #[error("I/O operation failed")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization failed
    #[error("JSON processing failed: {message}")]
    Json {
        /// A description of the error.
        message: String,
        /// The underlying `serde_json` error.
        #[source]
        source: Option<serde_json::Error>,
    },

    /// UTF-8 processing failed
    #[error("UTF-8 processing failed")]
    Utf8(#[from] std::str::Utf8Error),

    /// Process execution failed
    #[error("Process execution failed: command '{command}' {message}")]
    Process {
        /// The command that failed to execute.
        command: String,
        /// A description of the error.
        message: String,
    },

    /// Invalid input or state
    #[error("Invalid operation: {message}")]
    Invalid {
        /// A description of the error.
        message: String,
    },

    /// Timeout occurred
    #[error("Operation timed out: {operation}")]
    Timeout {
        /// The operation that timed out.
        operation: String,
    },
}

impl Error {
    /// Create a Git error with context
    pub fn git<S: Into<String>>(message: S) -> Self {
        Self::Git {
            message: message.into(),
            source: None,
        }
    }

    /// Create a Language error
    pub fn language<S: Into<String>>(message: S) -> Self {
        Self::Language {
            message: message.into(),
            source: None,
        }
    }

    /// Create an IPC error
    pub fn ipc<S: Into<String>>(message: S) -> Self {
        Self::Ipc {
            message: message.into(),
            source: None,
        }
    }

    /// Create a Watcher error
    pub fn watcher<S: Into<String>>(message: S) -> Self {
        Self::Watcher {
            message: message.into(),
            source: None,
        }
    }

    /// Create a Config error
    pub fn config<S: Into<String>>(message: S) -> Self {
        Self::Config {
            message: message.into(),
            source: None,
        }
    }

    /// Create an Agent error
    pub fn agent<S: Into<String>>(message: S) -> Self {
        Self::Agent {
            message: message.into(),
            source: None,
        }
    }

    /// Create a Process error
    pub fn process<S: Into<String>>(command: S, message: S) -> Self {
        Self::Process {
            command: command.into(),
            message: message.into(),
        }
    }

    /// Create an Invalid error
    pub fn invalid<S: Into<String>>(message: S) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }

    /// Create a Timeout error
    pub fn timeout<S: Into<String>>(operation: S) -> Self {
        Self::Timeout {
            operation: operation.into(),
        }
    }
}

/// Exit status a CLI binary should use after a command fails (#641).
///
/// `0` is success, `1` is any error this crate reports, and `2` is reserved
/// for clap usage errors (which never reach this code). Documented in
/// `docs/user/cli-reference.md` under "Exit Codes".
pub const CLI_FAILURE_EXIT_CODE: u8 = 1;

/// Render an error and its whole `source()` chain on one line, for the
/// `Error: ...` message a CLI prints to stderr before exiting non-zero.
///
/// `Display` alone hides the cause for wrapper variants such as
/// [`Error::Io`] ("I/O operation failed"), and the `Debug` form that
/// `std::process::Termination` prints (`Config { message: ... }`) is not a
/// message for a person, so both binaries route through this (#641).
#[must_use]
pub fn user_message(err: &(dyn std::error::Error + 'static)) -> String {
    let mut rendered = err.to_string();
    let mut cursor = err.source();
    while let Some(cause) = cursor {
        let text = cause.to_string();
        // Skip a cause that only restates the message wrapping it.
        if !rendered.ends_with(&text) {
            rendered.push_str(": ");
            rendered.push_str(&text);
        }
        cursor = cause.source();
    }
    rendered
}

/// Whether an error is a broken stdout pipe (the reader went away, e.g.
/// `gpy segments | head -1`), which a CLI should treat as a quiet, successful
/// end rather than an error (#641).
#[must_use]
pub fn is_broken_pipe(err: &Error) -> bool {
    matches!(err, Error::Io(io_err) if io_err.kind() == std::io::ErrorKind::BrokenPipe)
}

// Conversion from serde_json::Error
impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Json {
            message: err.to_string(),
            source: Some(err),
        }
    }
}

// Conversion from toml::de::Error
impl From<toml::de::Error> for Error {
    fn from(err: toml::de::Error) -> Self {
        Self::Config {
            message: format!("TOML parsing failed: {err}"),
            source: Some(Box::new(err)),
        }
    }
}

/// Result type alias for GPY Agent operations
pub type Result<T> = std::result::Result<T, Error>;
