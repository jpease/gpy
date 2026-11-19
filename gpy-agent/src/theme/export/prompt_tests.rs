//! `__icon_prompt`/`__prompt_color`: derived from `segments.character`.

use super::prompt_shell_values;
use crate::config::Config;
use crate::config::types::ColorSpec;
use crate::template::Palette;
use crate::theme::model::ThemeConfig;

/// The `name` value for `theme` under the default config.
fn value(theme: &ThemeConfig, name: &str) -> String {
    prompt_shell_values(theme, &Config::default(), &Palette::default())
        .remove(name)
        .unwrap_or_else(|| panic!("no {name} value"))
}

#[test]
fn prompt_symbol_follows_character_theme() {
    let mut theme = ThemeConfig::default();
    theme.segments.character.success_symbol = "λ".to_owned();
    theme.segments.character.success_color = ColorSpec::new("cyan").expect("color");

    assert_eq!(value(&theme, "__icon_prompt"), "λ");
    assert_eq!(value(&theme, "__prompt_color"), "cyan");
}

#[test]
fn prompt_symbol_drops_control_characters() {
    let mut theme = ThemeConfig::default();
    theme.segments.character.success_symbol = "\u{1b}]0;x\u{7}❯".to_owned();
    let icon = value(&theme, "__icon_prompt");
    assert!(
        !icon.contains('\u{1b}') && !icon.contains('\u{7}'),
        "{icon:?}"
    );
    assert!(icon.ends_with('❯'), "{icon:?}");

    theme.segments.character.success_symbol = "\u{1b}\u{7}".to_owned();
    assert_eq!(value(&theme, "__icon_prompt"), "❯");
}

/// Shell metacharacters reach the prompt verbatim: nothing is escaped.
#[test]
fn prompt_symbol_keeps_shell_metacharacters_verbatim() {
    let mut theme = ThemeConfig::default();
    theme.segments.character.success_symbol = r#"a\b$c"d`e"#.to_owned();

    assert_eq!(value(&theme, "__icon_prompt"), r#"a\b$c"d`e"#);
}
