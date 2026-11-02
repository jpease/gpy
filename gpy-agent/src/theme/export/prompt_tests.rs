//! `__icon_prompt`/`__prompt_color`: derived from `segments.character`.

use super::theme_to_shell::{PluginExports, theme_to_shell};
use crate::config::Config;
use crate::config::types::ColorSpec;
use crate::shell::Shell;
use crate::template::Palette;
use crate::theme::model::ThemeConfig;

fn bash_export(theme: &ThemeConfig) -> String {
    theme_to_shell(
        theme,
        "t",
        &Config::default(),
        Shell::Bash,
        &Palette::default(),
        &PluginExports::default(),
    )
}

/// The value of the `name="…"` assignment line in a Bash export.
fn assigned(export: &str, name: &str) -> String {
    let prefix = format!("{name}=\"");
    export
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("no {name} assignment in export"))
        .to_owned()
}

#[test]
fn prompt_symbol_export_follows_character_theme() {
    let mut theme = ThemeConfig::default();
    theme.segments.character.success_symbol = "λ".to_owned();
    theme.segments.character.success_color = ColorSpec::new("cyan").expect("color");

    let export = bash_export(&theme);

    assert_eq!(assigned(&export, "__icon_prompt"), "λ");
    assert_eq!(assigned(&export, "__prompt_color"), "cyan");
}

#[test]
fn prompt_symbol_export_drops_control_characters() {
    let mut theme = ThemeConfig::default();
    theme.segments.character.success_symbol = "\u{1b}]0;x\u{7}❯".to_owned();
    let export = bash_export(&theme);
    let icon = assigned(&export, "__icon_prompt");
    assert!(
        !icon.contains('\u{1b}') && !icon.contains('\u{7}'),
        "{icon:?}"
    );
    assert!(icon.ends_with('❯'), "{icon:?}");

    theme.segments.character.success_symbol = "\u{1b}\u{7}".to_owned();
    assert_eq!(assigned(&bash_export(&theme), "__icon_prompt"), "❯");
}
