//! Shell-export serialization: [`ThemeConfig`](crate::theme::model::ThemeConfig) → Fish/Bash/Zsh variable assignments.

mod plugin_segments;
mod script_segments;
mod segments;
mod shell_assignment;
mod shell_colors;
mod theme_to_shell;

pub use plugin_segments::{PluginVariableCollision, first_plugin_variable_collision};
pub(crate) use script_segments::collect_script_segments;
pub(crate) use shell_colors::resolve_shell_color;
pub(crate) use theme_to_shell::{PluginExports, theme_to_shell};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod export_names_tests;

#[cfg(test)]
mod cfg_mirror_tests;
