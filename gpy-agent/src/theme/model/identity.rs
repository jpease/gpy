//! Theme models of the segments naming where and who the session is: hostname and username.

use crate::config::defaults::literal;
use crate::config::types;
use serde::{Deserialize, Serialize};

/// Hostname segment configuration; `None` format = shell fast path, `Some` = agent-rendered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostnameTheme {
    /// Starship format template; `None` lets the shell render the hostname.
    pub format: Option<String>,
    /// SSH symbol shown before the hostname, SSH sessions only (Starship `ssh_symbol`).
    pub icon: Option<types::Icon>,
    /// Pill background color (`$bg`/`$style` in the format; shell path when no format).
    pub bg_color: types::ColorSpec,
    /// Pill text color (`$style` in the format; shell path when no format).
    pub text_color: types::ColorSpec,
    /// Trim at the first occurrence of this delimiter (default "."); empty disables.
    pub trim_at: String,
    /// Show on local (non-SSH) sessions too (default: false).
    pub show_always: bool,
}

impl Default for HostnameTheme {
    fn default() -> Self {
        Self {
            format: None,
            icon: None,
            bg_color: literal("black"),
            text_color: literal("white"),
            trim_at: ".".to_owned(),
            show_always: false,
        }
    }
}

/// Username segment (Starship parity), shown under root, sudo or SSH; gated shell-side.
/// Root sessions draw with `format`/`bg_color`/`text_color` (Starship's `style_root`),
/// everyone else with `user_format`/`user_bg_color`/`user_text_color` (`style_user`).
/// `None` formats = shell fast path, `Some` = agent-rendered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UsernameTheme {
    /// Starship format template for root; `None` lets the shell render the username.
    pub format: Option<String>,
    /// Format template for non-root users; `None` uses `format`.
    pub user_format: Option<String>,
    /// Icon/symbol shown before the username (default: none, matching Starship)
    pub icon: Option<types::Icon>,
    /// Root pill background color (`$bg`/`$style` in the format; shell path when no format).
    pub bg_color: types::ColorSpec,
    /// Root pill text color (`$style` in the format; shell path when no format).
    pub text_color: types::ColorSpec,
    /// Non-root pill background color (`$bg`/`$style`; shell path when no format).
    pub user_bg_color: types::ColorSpec,
    /// Non-root pill text color (`$style`; shell path when no format).
    pub user_text_color: types::ColorSpec,
    /// Show outside root, sudo and SSH sessions too (default: false).
    pub show_always: bool,
}

impl UsernameTheme {
    /// The format for a root (`is_root`) or non-root session.
    #[must_use]
    pub fn format_for(&self, is_root: bool) -> Option<&str> {
        if is_root {
            self.format.as_deref()
        } else {
            self.user_format.as_deref().or(self.format.as_deref())
        }
    }
}

impl Default for UsernameTheme {
    fn default() -> Self {
        Self {
            format: None,
            user_format: None,
            icon: None,
            bg_color: literal("red"),
            text_color: literal("white"),
            user_bg_color: literal("yellow"),
            user_text_color: literal("black"),
            show_always: false,
        }
    }
}
