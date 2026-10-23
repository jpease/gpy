//! Theme models of the segments naming where and who the session is: hostname and username.

use crate::config::defaults::{
    default_color_black, default_color_red, default_color_white, default_color_yellow,
    default_hostname_trim_at,
};
use crate::config::types;
use serde::{Deserialize, Serialize};

/// Hostname segment configuration; `None` format = shell fast path, `Some` = agent-rendered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostnameTheme {
    /// Starship format template; `None` lets the shell render the hostname.
    #[serde(default)]
    pub format: Option<String>,
    /// SSH symbol shown before the hostname, SSH sessions only (Starship `ssh_symbol`).
    #[serde(default)]
    pub icon: Option<types::Icon>,
    /// Pill background color (`$bg`/`$style` in the format; shell path when no format).
    #[serde(default = "default_color_black")]
    pub bg_color: types::ColorSpec,
    /// Pill text color (`$style` in the format; shell path when no format).
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
    /// Trim at the first occurrence of this delimiter (default "."); empty disables.
    #[serde(default = "default_hostname_trim_at")]
    pub trim_at: String,
    /// Show on local (non-SSH) sessions too (default: false).
    #[serde(default)]
    pub show_always: bool,
}

impl Default for HostnameTheme {
    fn default() -> Self {
        Self {
            format: None,
            icon: None,
            bg_color: default_color_black(),
            text_color: default_color_white(),
            trim_at: default_hostname_trim_at(),
            show_always: false,
        }
    }
}

/// Username segment (Starship parity), shown under root, sudo or SSH; gated shell-side.
/// Root sessions draw with `format`/`bg_color`/`text_color` (Starship's `style_root`),
/// everyone else with `user_format`/`user_bg_color`/`user_text_color` (`style_user`).
/// `None` formats = shell fast path, `Some` = agent-rendered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsernameTheme {
    /// Starship format template for root; `None` lets the shell render the username.
    #[serde(default)]
    pub format: Option<String>,
    /// Format template for non-root users; `None` uses `format`.
    #[serde(default)]
    pub user_format: Option<String>,
    /// Icon/symbol shown before the username (default: none, matching Starship)
    #[serde(default)]
    pub icon: Option<types::Icon>,
    /// Root pill background color (`$bg`/`$style` in the format; shell path when no format).
    #[serde(default = "default_color_red")]
    pub bg_color: types::ColorSpec,
    /// Root pill text color (`$style` in the format; shell path when no format).
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
    /// Non-root pill background color (`$bg`/`$style`; shell path when no format).
    #[serde(default = "default_color_yellow")]
    pub user_bg_color: types::ColorSpec,
    /// Non-root pill text color (`$style`; shell path when no format).
    #[serde(default = "default_color_black")]
    pub user_text_color: types::ColorSpec,
    /// Show outside root, sudo and SSH sessions too (default: false).
    #[serde(default)]
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
            bg_color: default_color_red(),
            text_color: default_color_white(),
            user_bg_color: default_color_yellow(),
            user_text_color: default_color_black(),
            show_always: false,
        }
    }
}
