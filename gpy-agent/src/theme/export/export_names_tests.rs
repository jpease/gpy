use super::tests::{golden_config, golden_theme};
use super::values::builtin_value_names;
use crate::template::Palette;

/// The built-in name set covers every value of a theme without plugin
/// segments, for the golden theme and config as well as the defaults.
#[test]
fn builtin_value_names_cover_every_builtin_value() {
    let mut theme = golden_theme();
    theme.segments.plugin.clear();
    let values = super::prompt_shell_values(&theme, &golden_config(), &Palette::default());
    let builtin = builtin_value_names();
    for name in values.keys() {
        assert!(
            builtin.contains(name),
            "`{name}` is set but missing from builtin_value_names()"
        );
    }
    for expected in [
        "__color_git_clean_bg",
        "__icon_status_fail",
        "__prompt_icons",
    ] {
        assert!(builtin.contains(expected), "missing `{expected}`");
    }
}
