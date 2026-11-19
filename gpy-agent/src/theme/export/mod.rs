//! Theme values for the prompt renderer: [`ThemeConfig`](crate::theme::model::ThemeConfig)
//! and config as the named values `gpy prompt` reads, plus the enabled script
//! segments it runs.

mod plugin_segments;
#[cfg(unix)]
mod script_segments;
mod segments;
mod shell_colors;
mod values;

pub use plugin_segments::{PluginVariableCollision, first_plugin_variable_collision};
#[cfg(unix)]
pub(crate) use script_segments::{
    ScriptSegmentExport, ScriptSegmentExports, collect_script_segments,
};
pub(crate) use shell_colors::resolve_shell_color;
pub use values::prompt_shell_values;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod export_names_tests;

#[cfg(test)]
mod prompt_tests;
