use super::prompt_shell_values;
use super::resolve_shell_color;
use crate::config::Config;
use crate::config::types::ColorSpec;
use crate::shell::Shell;
use crate::template::{Color, Palette};
use crate::theme::model::ThemeConfig;
use std::collections::HashMap;
use std::process::Command;

/// A theme populating every value; values embed `$`, backticks, quotes and a
/// trailing backslash, which reach the prompt verbatim.
const GOLDEN_THEME_TOML: &str = r##"
[ui]
root_prompt_icon = "#`r`"
root_prompt_color = "yellow"
two_line = true

[ui.prompt_open]
icon = "["
icon_color = "cyan"
bg_color = "blue"

[ui.prompt_close]
icon = "]"
icon_color = "green"
bg_color = "red"

[ui.segment_open]
icon = ""
icon_color = "white"
bg_color = "black"

[ui.segment_close]
icon = ""
icon_color = "black"
bg_color = "white"

[segments.character]
success_symbol = "➜$p"
success_color = "magenta"

[segments.clock]
format = "[$time]($style)"
bg_color = "blue"
text_color = "white"
time_format = "%H:%M$(date)"
show_leading_zero = true
show_seconds = true

[segments.directory]
bg_color = "cyan"
text_color = "black"

[segments.duration]
bg_color = "yellow"
text_color = "black"
show_if_exceeds_ms = 2500

[segments.hostname]
ssh_icon = "host\\"
bg_color = "green"
text_color = "black"

[segments.username]
icon = "user`x`"
bg_color = "yellow"
text_color = "black"
root_bg_color = "red"
root_text_color = "white"

[segments.status]
ok_icon = "ok$1"
fail_icon = "fail`2`"
ok_bg_color = "green"
ok_text_color = "black"
fail_bg_color = "red"
fail_text_color = "white"

[segments.git]
bg_color = "purple"
clean_bg_color = "green"
dirty_bg_color = "red"

[segments.language]
bg_color = "blue"

[segments.kubernetes]
icon = "k8s$ctx"
bg_color = "blue"
text_color = "white"
warn_bg_color = "bright-blue"
warn_text_color = "black"

[segments.AWS-Profile]
icon = "aws"
bg_color = "yellow"
text_color = "black"

[segments.AWS-Profile.open]
icon = "<"
icon_color = "white"
bg_color = "black"

[segments.AWS-Profile.close]
icon = ">"
icon_color = "black"
bg_color = "white"
"##;

/// A config moving the settings the values read off their defaults.
const GOLDEN_CONFIG_TOML: &str = r#"
[ui]
show_icons = false
enabled_segments = ["clock", "username", "hostname", "directory", "git", "duration", "status"]
"#;

pub(super) fn golden_theme() -> ThemeConfig {
    toml::from_str(GOLDEN_THEME_TOML).expect("golden theme fixture should parse")
}

pub(super) fn golden_config() -> Config {
    toml::from_str(GOLDEN_CONFIG_TOML).expect("golden config fixture should parse")
}

/// `values` as sorted `name="value"` lines; the quotes keep a trailing space
/// visible (and safe from whitespace-trimming hooks).
fn sorted_lines(values: &HashMap<String, String>) -> String {
    let mut lines: Vec<String> = values
        .iter()
        .map(|(name, value)| format!("{name}={value:?}"))
        .collect();
    lines.sort_unstable();
    lines.join("\n")
}

fn values_with(theme: &ThemeConfig, show_icons: bool) -> HashMap<String, String> {
    let mut config = Config::default();
    config.ui.show_icons = show_icons;
    prompt_shell_values(theme, &config, &Palette::default())
}

fn value<'a>(values: &'a HashMap<String, String>, name: &str) -> &'a str {
    values
        .get(name)
        .unwrap_or_else(|| panic!("no `{name}` value"))
}

fn has_private_use(text: &str) -> bool {
    text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c))
}

#[test]
fn golden_theme_values_match_snapshot() {
    let values = prompt_shell_values(&golden_theme(), &golden_config(), &Palette::default());
    insta::assert_snapshot!("golden_values", sorted_lines(&values));
}

/// Companion snapshot for the unset/absent arms the populated fixture misses.
#[test]
fn default_theme_values_match_snapshot() {
    let values = prompt_shell_values(
        &ThemeConfig::default(),
        &Config::default(),
        &Palette::default(),
    );
    insta::assert_snapshot!("default_theme_values", sorted_lines(&values));
}

/// `ui.show_icons = false` empties Nerd Font delimiter glyphs; plain-text delimiters survive.
#[test]
fn icons_off_drops_private_use_delimiters() {
    let on = values_with(&golden_theme(), true);
    let off = values_with(&golden_theme(), false);
    for name in [
        "__segment_delim_start",
        "__segment_delim_end",
        "__icon_powerline_segment_start",
        "__icon_powerline_segment_end",
    ] {
        assert!(has_private_use(value(&on, name)), "icons on keep {name}");
        assert_eq!(value(&off, name), "", "icons off empty {name}");
    }
    assert_eq!(value(&off, "__segment_delim_first"), "[");
    assert_eq!(value(&off, "__segment_delim_last"), "]");
}

/// `ui.show_icons = false` unsets the default theme's Nerd Font status icons so
/// the renderer draws its ASCII fallback; with icons on they are set.
#[test]
fn icons_off_unsets_private_use_status_icons() {
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("default theme should always load")
        .get();
    let on = values_with(&theme, true);
    let off = values_with(&theme, false);
    for name in ["__icon_status_ok", "__icon_status_fail"] {
        assert!(has_private_use(value(&on, name)), "icons on keep {name}");
        assert!(!off.contains_key(name), "icons off unset {name}");
    }
}

/// `ui.show_icons = false` empties a Nerd Font plugin segment icon, a variant
/// icon and open/close cap icons, which the renderer reads as "no icon".
#[test]
fn icons_off_empties_private_use_plugin_segment_icons() {
    let theme: ThemeConfig = toml::from_str(
        "[segments.hostname]\nssh_icon = \"\u{f108}\"\n[segments.username]\nicon = \"\u{f007}\"\n[segments.username.open]\nicon = \"\u{e0b6}\"\n[segments.username.close]\nicon = \"\u{e0b4}\"\n",
    )
    .expect("theme parses");
    let on = values_with(&theme, true);
    let off = values_with(&theme, false);
    for name in [
        "__icon_hostname_ssh",
        "__gpy_segment_hostname_variant_ssh_icon",
        "__icon_username",
        "__gpy_segment_username_icon",
        "__gpy_segment_username_open_icon",
        "__gpy_segment_username_close_icon",
    ] {
        assert!(has_private_use(value(&on, name)), "icons on keep {name}");
        assert_eq!(value(&off, name), "", "icons off empty {name}");
    }
}

/// `two_line` and `add_newline` are `1`/`0` flags.
#[test]
fn layout_flags_follow_the_theme() {
    let mut theme = ThemeConfig::default();
    theme.ui.two_line = true;
    theme.ui.add_newline = false;
    let values = values_with(&theme, true);
    assert_eq!(value(&values, "__gpy_two_line"), "1");
    assert_eq!(value(&values, "__gpy_add_newline"), "0");
    theme.ui.two_line = false;
    assert_eq!(value(&values_with(&theme, true), "__gpy_two_line"), "0");
}

/// Variant `open` keeps off the `open` delimiter's colour (`_variant_` namespace);
/// a section whose names collide with an earlier one's (segment `a` against
/// `a_b`, `a_open`, `a_variant`, `a_variant_x`) is skipped whole.
#[test]
fn plugin_segment_sections_that_collide_are_skipped() {
    let theme: ThemeConfig = toml::from_str(
        r#"
[segments.a]
open_bg_color = "red"
b_c_bg_color = "green"
x_bg_color = "green"
variant_b_bg_color = "green"

[segments.a.open]
icon = "<"
icon_color = "white"
bg_color = "black"

[segments.a_b]
c_bg_color = "blue"
[segments.a_open]
icon = "*"
[segments.a_variant]
b_bg_color = "blue"
[segments.a_variant_x]
bg_color = "blue"
[segments.k8s]
ok_bg_color = "blue"
"#,
    )
    .expect("collision fixture should parse");
    let values = values_with(&theme, true);
    for (name, expected) in [
        ("__gpy_segment_a_open_bg_color", "black"),
        ("__gpy_segment_a_variant_open_bg_color", "red"),
        ("__gpy_segment_a_variant_b_c_bg_color", "green"),
        ("__gpy_segment_k8s_variant_ok_bg_color", "blue"),
    ] {
        assert_eq!(value(&values, name), expected, "{name}");
    }
    for skipped in ["a_b", "a_open", "a_variant", "a_variant_x"] {
        assert!(
            !values.contains_key(&format!("__gpy_segment_{skipped}_id")),
            "`{skipped}` must be skipped"
        );
    }
}

/// An ANSI index keeps the terminal's palette; a palette role takes its value.
#[test]
fn index_and_role_colors_resolve_through_the_palette() {
    let mut theme = ThemeConfig::default();
    theme.segments.clock.bg_color = ColorSpec::new("42").expect("index color");
    theme.segments.clock.text_color = ColorSpec::new("3").expect("index color");
    theme.segments.status.ok_bg_color = ColorSpec::new("orange").expect("role color");
    let palette = Palette::new(HashMap::from([(
        "orange".to_owned(),
        Color::Rgb {
            r: 0xfa,
            g: 0xb3,
            b: 0x87,
        },
    )]));
    let values = prompt_shell_values(&theme, &Config::default(), &palette);
    assert_eq!(value(&values, "__color_clock_bg"), "42");
    assert_eq!(value(&values, "__color_clock_fg"), "3");
    assert_eq!(value(&values, "__color_status_ok_bg"), "#fab387");
}

/// A palette entry shadows an ANSI name, including its alias spellings, as it
/// does in the agent's renderer; an ANSI index still names the terminal's own color.
#[test]
fn ansi_names_follow_the_palette_and_indices_do_not() {
    let mut theme = ThemeConfig::default();
    theme.segments.clock.bg_color = ColorSpec::new("red").expect("named color");
    theme.segments.clock.text_color = ColorSpec::new("1").expect("index color");
    theme.segments.status.ok_bg_color = ColorSpec::new("magenta").expect("named color");
    let palette = Palette::new(HashMap::from([
        (
            "red".to_owned(),
            Color::Rgb {
                r: 0xf3,
                g: 0x8b,
                b: 0xa8,
            },
        ),
        // `magenta` reaches this entry only through the ANSI alias.
        (
            "purple".to_owned(),
            Color::Rgb {
                r: 0xcb,
                g: 0xa6,
                b: 0xf7,
            },
        ),
    ]));
    let values = prompt_shell_values(&theme, &Config::default(), &palette);
    assert_eq!(value(&values, "__color_clock_bg"), "#f38ba8");
    assert_eq!(value(&values, "__color_clock_fg"), "1");
    assert_eq!(value(&values, "__color_status_ok_bg"), "#cba6f7");
}

/// Fish's `set_color` rejects bare ANSI indices and palette roles, so
/// [`resolve_shell_color`] gives Fish names/hex for them; checked against a
/// real `set_color` when Fish is installed.
#[test]
fn fish_colors_reach_real_set_color() {
    let palette = Palette::new(HashMap::from([(
        "orange".to_owned(),
        Color::Rgb {
            r: 0xfa,
            g: 0xb3,
            b: 0x87,
        },
    )]));
    let colors =
        ["42", "3", "orange"].map(|color| resolve_shell_color(Shell::Fish, &palette, color));
    assert_eq!(colors, ["#00d787", "yellow", "#fab387"]);

    let fish = Command::new("fish")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .args([
            "--no-config",
            "-c",
            "set -g fish_term24bit 1; set_color $argv[1]; set_color $argv[2]; set_color -b $argv[3]",
            "--",
        ])
        .args(&colors)
        .output();
    let Ok(output) = fish else {
        eprintln!("SKIP: fish is not available");
        let ci = ["CI", "GPY_GATE_STRICT"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()));
        assert!(
            !ci || std::env::var_os("GPY_CI_NO_SHELLS").is_some(),
            "fish is not available under CI; install it or set GPY_CI_NO_SHELLS=1"
        );
        return;
    };
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "\x1b[38;2;0;215;135m\x1b[33m\x1b[48;2;250;179;135m",
        "42 is xterm #00d787, 3 is yellow, orange is the palette's #fab387"
    );
}
