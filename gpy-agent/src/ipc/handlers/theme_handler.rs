//! IPC handler for theme queries and exports.
//!
//! Theme requests expose the currently active prompt theme to shells and CLI
//! callers. This handler reads from [`crate::theme::ThemeManager`] and delegates
//! shell-specific rendering to the formatter layer so theme state stays shared
//! while output formats remain independent.

use super::{HandlerError, RequestHandler};
use crate::config::DEFAULT_UI_PROMPT_COLOR;
use crate::ipc::{Message, Response};
use crate::theme::ThemeManager;
use std::sync::Arc;

/// Valid theme query keys that can be requested
const THEME_QUERY_VALID_KEYS: &[&str] = &[
    "prompt",
    "segment_open_delimiter",
    "segment_close_delimiter",
    "prompt_color",
    "status_ok_icon",
    "status_fail_icon",
    "icon",
    "time_format",
    "prompt_close",
    "prompt_open",
];

/// Handler for theme-related IPC requests
pub struct ThemeHandler {
    theme_manager: Arc<ThemeManager>,
}

impl ThemeHandler {
    /// Create a new `ThemeHandler` with the given theme manager
    #[must_use]
    pub const fn new(theme_manager: Arc<ThemeManager>) -> Self {
        Self { theme_manager }
    }

    /// Handle theme query for a specific key
    ///
    /// # Errors
    ///
    /// Returns an error if the requested theme key is unknown or invalid.
    #[expect(
        clippy::too_many_lines,
        reason = "dispatch table mapping every known theme key to its ThemeValue; each arm is a single field lookup, so splitting it would scatter the single source of truth for which keys are valid"
    )]
    fn handle_theme_query(&self, key: &str) -> Result<Response, HandlerError> {
        // Use the shared theme manager instead of loading from disk
        let theme = self.theme_manager.get();

        // Return the value if the key exists (even if empty string)
        // Return error if the key is unknown
        match key {
            // Delimiters
            "prompt_close" => Ok(Response::ThemeValue {
                value: theme.ui.get_prompt_close_delimiter().to_owned(),
            }),
            "prompt_open" => Ok(Response::ThemeValue {
                value: theme.ui.get_prompt_open_delimiter().to_owned(),
            }),
            "prompt" => Ok(Response::ThemeValue {
                value: theme.ui.prompt_icon.to_string(),
            }),
            "segment_open_delimiter" => Ok(Response::ThemeValue {
                value: theme.ui.get_segment_open_delimiter().to_owned(),
            }),
            "segment_close_delimiter" => Ok(Response::ThemeValue {
                value: theme.ui.get_segment_close_delimiter().to_owned(),
            }),
            // Colors
            "prompt_color" => Ok(Response::ThemeValue {
                value: theme
                    .ui
                    .prompt_color
                    .as_ref()
                    .map_or(DEFAULT_UI_PROMPT_COLOR, |c| c.as_str())
                    .to_owned(),
            }),
            // Icons
            "status_ok_icon" => Ok(Response::ThemeValue {
                value: theme
                    .segments
                    .status
                    .ok_icon
                    .as_ref()
                    .map_or_else(|| "✔".to_owned(), std::string::ToString::to_string),
            }),
            "status_fail_icon" => Ok(Response::ThemeValue {
                value: theme
                    .segments
                    .status
                    .fail_icon
                    .as_ref()
                    .map_or_else(|| "✖".to_owned(), std::string::ToString::to_string),
            }),
            "icon" => Ok(Response::ThemeValue {
                value: theme
                    .segments
                    .duration
                    .icon
                    .as_ref()
                    .map_or_else(|| "󰑧".to_owned(), std::string::ToString::to_string),
            }),
            // Time format
            "time_format" => Ok(Response::ThemeValue {
                value: theme
                    .segments
                    .clock
                    .time_format
                    .clone()
                    .unwrap_or_else(|| "12".to_owned()),
            }),
            _ => {
                let valid_keys = THEME_QUERY_VALID_KEYS.join(", ");
                Err(HandlerError::UnknownThemeKey {
                    key: key.to_owned(),
                    valid_keys,
                })
            }
        }
    }
}

impl RequestHandler for ThemeHandler {
    fn handle(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::ThemeQuery { key } => self.handle_theme_query(key),
            _ => Err(HandlerError::UnexpectedMessage {
                handler: "ThemeHandler",
                qualifier: "non-theme",
                message: format!("{message:?}"),
            }),
        }
    }

    fn name(&self) -> &'static str {
        "ThemeHandler"
    }
}
