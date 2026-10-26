//! Crate-wide error type.

use thiserror::Error;

/// Main error type for GPY Agent operations
#[derive(Error, Debug)]
pub enum Error {
    /// Git repository operation failed
    #[error("Git operation failed: {message}")]
    Git {
        /// A description of the error.
        message: String,
    },

    /// Language detection failed
    #[error("Language detection failed: {message}")]
    Language {
        /// A description of the error.
        message: String,
    },

    /// IPC communication failed
    #[error("IPC communication failed: {message}")]
    Ipc {
        /// A description of the error.
        message: String,
    },

    /// File watching failed
    #[error("File watcher failed: {message}")]
    Watcher {
        /// A description of the error.
        message: String,
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

    /// Starship importer translation failed
    #[error(transparent)]
    StarshipImport(#[from] crate::import::starship::ImportError),

    /// base16/base24 importer failed
    #[error(transparent)]
    Base16Import(#[from] crate::import::base16::ImportError),

    /// Process execution failed
    #[error("Process execution failed: command '{command}' {message}")]
    Process {
        /// The command that failed to execute.
        command: String,
        /// A description of the error.
        message: String,
    },

    /// `gpy doctor` found at least one problem (details already printed)
    #[error("Health check failed")]
    HealthCheckFailed,

    /// Invalid input or state
    #[error("Invalid operation: {message}")]
    Invalid {
        /// A description of the error.
        message: String,
    },
}

impl Error {
    /// Create a Git error with context
    pub fn git<S: Into<String>>(message: S) -> Self {
        Self::Git {
            message: message.into(),
        }
    }

    /// Create a Language error
    pub fn language<S: Into<String>>(message: S) -> Self {
        Self::Language {
            message: message.into(),
        }
    }

    /// Create an IPC error
    pub fn ipc<S: Into<String>>(message: S) -> Self {
        Self::Ipc {
            message: message.into(),
        }
    }

    /// Create a JSON error that has no underlying `serde_json::Error`
    pub fn json<S: Into<String>>(message: S) -> Self {
        Self::Json {
            message: message.into(),
            source: None,
        }
    }

    /// Create a Watcher error
    pub fn watcher<S: Into<String>>(message: S) -> Self {
        Self::Watcher {
            message: message.into(),
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

    /// Create an I/O error whose message names the failed action (`context`),
    /// keeping the original [`std::io::ErrorKind`].
    pub fn io_context(context: impl std::fmt::Display, source: &std::io::Error) -> Self {
        Self::Io(std::io::Error::new(
            source.kind(),
            format!("{context}: {source}"),
        ))
    }

    /// Create a Process error
    pub fn process(command: impl Into<String>, message: impl Into<String>) -> Self {
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
}

/// Exit status after a failed command (`2` is reserved for clap usage errors).
pub const CLI_FAILURE_EXIT_CODE: u8 = 1;

/// Render an error and its `source()` chain on one line for CLI stderr.
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

/// Whether an error is a broken stdout pipe, which a CLI treats as success.
#[must_use]
pub fn is_broken_pipe(err: &Error) -> bool {
    matches!(err, Error::Io(io_err) if io_err.kind() == std::io::ErrorKind::BrokenPipe)
}

/// Turn a command's result into its process exit code: a broken stdout pipe is
/// success, any other error prints one `Error: ...` line and exits
/// [`CLI_FAILURE_EXIT_CODE`].
#[must_use]
pub fn report(result: Result<std::process::ExitCode>) -> std::process::ExitCode {
    match result {
        Ok(code) => code,
        Err(err) if is_broken_pipe(&err) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {}", user_message(&err));
            std::process::ExitCode::from(CLI_FAILURE_EXIT_CODE)
        }
    }
}

/// Install a panic hook that exits 0 when a CLI pipeline closes stdout early
/// (`gpy segments | head -1`) and hands every other panic to `report`.
/// `unsafe` is forbidden, so `SIGPIPE` cannot be reset to its default.
#[allow(
    clippy::exit,
    reason = "a panic hook cannot return an exit code; a broken stdout pipe must end the process with status 0"
)]
pub fn install_broken_pipe_hook<F>(report: F)
where
    F: Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static,
{
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or_default();
        if message.contains("failed printing to stdout") && message.contains("Broken pipe") {
            std::process::exit(0);
        }
        report(info);
    }));
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Json {
            message: err.to_string(),
            source: Some(err),
        }
    }
}

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
