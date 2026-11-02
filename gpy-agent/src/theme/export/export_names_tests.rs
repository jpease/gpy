use super::tests::{GOLDEN_THEME_NAME, golden_config, golden_theme};
use super::theme_to_shell::{PluginExports, assigned_name, builtin_export_names, theme_to_shell};
use crate::shell::Shell;
use crate::template::Palette;

/// The built-in name set covers every variable the emitter writes ahead of the
/// plugin segments, for the golden theme and config as well as the defaults.
#[test]
fn builtin_export_names_cover_everything_the_builtin_emitter_writes() {
    let mut theme = golden_theme();
    theme.segments.plugin.clear();
    let export = theme_to_shell(
        &theme,
        GOLDEN_THEME_NAME,
        &golden_config(),
        Shell::Bash,
        &Palette::default(),
        &PluginExports::default(),
    );
    let builtin = builtin_export_names();
    // Written outside the built-in appenders: the version sentinel (first line), then the
    // script, enabled-segment and marker-file appenders after them.
    let later = [
        "__gpy_export_version",
        "__gpy_script_sh",
        "__gpy_script_segments",
        "__enabled_segments",
        "__gpy_lang_marker_files",
    ];
    let function_start = export.find("__gpy_segment_bg()").unwrap_or(export.len());
    for name in export[..function_start].lines().filter_map(assigned_name) {
        assert!(
            builtin.contains(name) || later.contains(&name),
            "`{name}` is exported but missing from builtin_export_names():\n{export}"
        );
    }
    for expected in [
        "__color_git_clean_bg",
        "__icon_status_fail",
        "__gpy_cfg_agent_enabled",
    ] {
        assert!(builtin.contains(expected), "missing `{expected}`");
    }
}
