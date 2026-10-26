//! Shell-export serialization: [`ThemeConfig`](crate::theme::model::ThemeConfig) → Fish/Bash/Zsh variable assignments.

mod plugin_segments;
mod segments;
mod shell_assignment;
mod shell_colors;
mod theme_to_shell;

pub use plugin_segments::{PluginVariableCollision, first_plugin_variable_collision};
#[cfg(test)]
use shell_assignment::escape_shell_value;
#[cfg(test)]
use theme_to_shell::SEGMENT_BG_VARS;
pub(crate) use theme_to_shell::theme_to_shell;

#[cfg(test)]
mod tests;
