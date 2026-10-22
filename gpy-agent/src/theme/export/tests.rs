#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

use super::{SEGMENT_BG_VARS, escape_shell_value, theme_to_shell};
use crate::config::Config;
use crate::shell::Shell;
use crate::template::Palette;
use crate::theme::model::ThemeConfig;
use std::collections::BTreeMap;
use std::process::Command;

/// Theme name with shell metacharacters, pinning header and `__gpy_theme_name` escaping.
const GOLDEN_THEME_NAME: &str = "night$owl\"";

/// A theme populating every export phase; values embed `$`, backticks, quotes,
/// and a trailing backslash so the goldens pin escaping.
const GOLDEN_THEME_TOML: &str = r##"
[ui]
prompt_icon = "➜$p"
root_prompt_icon = "#`r`"
prompt_color = "magenta"
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
format = "on [$hostname]($style)"
icon = "host\\"
bg_color = "green"
text_color = "black"
trim_at = ".local\""
show_always = true

[segments.username]
format = "[$user]($style)"
icon = "user`x`"
bg_color = "red"
text_color = "white"
show_always = true

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
context_color = "bright-blue"
namespace_icon = "ns`n`"
plain_property = "value\\"

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

/// A config moving every exported setting off its default; disables `language`
/// but keeps `git` to pin both arms of the enabled-segment filter.
const GOLDEN_CONFIG_TOML: &str = r#"
[agent]
enabled = false
live_updates = false
timeout_seconds = 7

[agent.supervisor]
enabled = false
check_interval_seconds = 45
max_restart_attempts = 9

[git]
enabled = true
show_upstream = false
max_branch_length = 17

[language]
enabled = false
show_versions = false

[ui]
show_icons = false
enabled_segments = ["clock", "username", "hostname", "directory", "git", "language", "duration", "status"]

[ui.directory]
display = "truncated"
max_length = 42
"#;

fn golden_theme() -> ThemeConfig {
    toml::from_str(GOLDEN_THEME_TOML).expect("golden theme fixture should parse")
}

fn golden_config() -> Config {
    toml::from_str(GOLDEN_CONFIG_TOML).expect("golden config fixture should parse")
}

/// Plugin segment files: one that is not a segment name (skipped), one with a metacharacter path.
fn golden_plugin_segment_files() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "kubernetes".to_owned(),
            "/usr/share/gpy/segments/kubernetes.fish".to_owned(),
        ),
        (
            "aws-profile".to_owned(),
            "/usr/share/gpy/segments/aws$profile.fish".to_owned(),
        ),
        (
            "aws profile".to_owned(),
            "/skipped/invalid-name.fish".to_owned(),
        ),
    ])
}

fn golden_export(shell: Shell) -> String {
    theme_to_shell(
        &golden_theme(),
        GOLDEN_THEME_NAME,
        &golden_config(),
        shell,
        &Palette::default(),
        &golden_plugin_segment_files(),
    )
}

/// The exported marker list is exactly `marker_file_names()`, sorted.
#[test]
fn export_lists_exactly_the_sorted_marker_file_names() {
    let mut expected: Vec<&str> = crate::language::metadata::marker_file_names()
        .iter()
        .copied()
        .collect();
    expected.sort_unstable();
    for shell in [Shell::Fish, Shell::Zsh, Shell::Bash] {
        let export = golden_export(shell);
        let line = export
            .lines()
            .find(|l| l.contains("__gpy_lang_marker_files"))
            .expect("marker export line");
        let body = match shell {
            Shell::Fish => line.strip_prefix("set -g __gpy_lang_marker_files "),
            Shell::Zsh => line
                .strip_prefix("typeset -g -a __gpy_lang_marker_files=(")
                .and_then(|r| r.strip_suffix(')')),
            Shell::Bash => line
                .strip_prefix("__gpy_lang_marker_files=(")
                .and_then(|r| r.strip_suffix(')')),
        }
        .expect("marker line shape");
        let got: Vec<&str> = body.split(' ').collect();
        assert_eq!(got, expected, "{shell}");
    }
}

/// `ui.show_icons = false` exports delimiter glyphs empty; plain-text delimiters survive.
#[test]
fn export_drops_private_use_delimiters_when_icons_are_off() {
    const DELIMITER_VARS: [&str; 6] = [
        "__segment_delim_start",
        "__segment_delim_end",
        "__icon_powerline_segment_start",
        "__icon_powerline_segment_end",
        "__segment_delim_first",
        "__segment_delim_last",
    ];
    let has_private_use = |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
    let mut icons_off = golden_config();
    icons_off.ui.show_icons = false;
    let mut icons_on = golden_config();
    icons_on.ui.show_icons = true;
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        let export = |config: &Config| {
            theme_to_shell(
                &golden_theme(),
                GOLDEN_THEME_NAME,
                config,
                shell,
                &Palette::default(),
                &golden_plugin_segment_files(),
            )
        };
        let line_of = |script: &str, name: &str| {
            script
                .lines()
                .find(|l| l.contains(&format!("{name} ")) || l.contains(&format!("{name}=")))
                .unwrap_or_else(|| panic!("{shell}: no `{name}` line in export"))
                .to_owned()
        };
        let on = export(&icons_on);
        let off = export(&icons_off);
        for name in DELIMITER_VARS {
            assert!(
                !has_private_use(&line_of(&off, name)),
                "{shell}: show_icons=false still exports a private-use glyph in {name}"
            );
        }
        for name in [
            "__segment_delim_start",
            "__segment_delim_end",
            "__icon_powerline_segment_start",
            "__icon_powerline_segment_end",
        ] {
            assert!(
                has_private_use(&line_of(&on, name)),
                "{shell}: show_icons=true control: {name} must keep its glyph"
            );
        }
        // The plain-text delimiters of the fixture are untouched.
        for (name, text) in [
            ("__segment_delim_first", "["),
            ("__segment_delim_last", "]"),
        ] {
            assert!(
                line_of(&off, name).contains(text),
                "{shell}: {name} lost {text:?}"
            );
        }
    }
}

/// `ui.show_icons = false` erases the default theme's Nerd Font status icons so
/// the shells draw their ASCII fallback; with icons on they are exported.
#[test]
fn export_erases_private_use_status_icons_when_icons_are_off() {
    let has_private_use = |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("default theme should always load")
        .get();
    let export = |show_icons: bool, shell: Shell| {
        let mut config = Config::default();
        config.ui.show_icons = show_icons;
        theme_to_shell(
            &theme,
            "default",
            &config,
            shell,
            &Palette::default(),
            &BTreeMap::new(),
        )
    };
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        let on = export(true, shell);
        let off = export(false, shell);
        for name in ["__icon_status_ok", "__icon_status_fail"] {
            let line_of = |script: &str| {
                script
                    .lines()
                    .find(|l| l.split([' ', '=']).any(|word| word == name))
                    .unwrap_or_else(|| panic!("{shell}: no `{name}` line in export"))
                    .to_owned()
            };
            assert!(
                has_private_use(&line_of(&on)),
                "{shell}: show_icons=true control: {name} must keep its glyph"
            );
            let off_line = line_of(&off);
            assert!(
                off_line.trim_end().ends_with(name) && !has_private_use(&off_line),
                "{shell}: show_icons=false must erase {name}, got {off_line:?}"
            );
        }
    }
}

#[test]
fn fish_export_matches_golden() {
    insta::assert_snapshot!("fish_export", golden_export(Shell::Fish));
}

#[test]
fn bash_export_matches_golden() {
    insta::assert_snapshot!("bash_export", golden_export(Shell::Bash));
}

#[test]
fn zsh_export_matches_golden() {
    insta::assert_snapshot!("zsh_export", golden_export(Shell::Zsh));
}

/// Bash/Zsh exports carry a generated `__gpy_segment_bg`; Fish gets none.
#[test]
fn bash_and_zsh_export_include_segment_bg_function_for_every_builtin_segment() {
    for shell in [Shell::Bash, Shell::Zsh] {
        let export = golden_export(shell);
        assert!(
            export.contains("__gpy_segment_bg()"),
            "{shell:?} export must define __gpy_segment_bg(), got:\n{export}"
        );
        for (segment, var) in SEGMENT_BG_VARS {
            let arm = format!("{segment}) __gpy_sbg=");
            assert!(
                export.contains(&arm),
                "{shell:?} export's __gpy_segment_bg must have a case arm for \
                     segment '{segment}', got:\n{export}"
            );
            let var_ref = format!("${{{var}:-}}");
            assert!(
                export.contains(&var_ref),
                "{shell:?} export's __gpy_segment_bg case arm for '{segment}' must \
                     reference {var_ref}, got:\n{export}"
            );
        }
    }
}

/// The `status` arm reads `__gpy_last_status`, falling back to `green`/`red`.
#[test]
fn bash_and_zsh_segment_bg_function_has_a_status_arm_keyed_on_last_status() {
    for shell in [Shell::Bash, Shell::Zsh] {
        let export = golden_export(shell);
        for needle in [
            "status)",
            "${__gpy_last_status:-0}",
            "${__color_status_ok_bg:-green}",
            "${__color_status_fail_bg:-red}",
        ] {
            assert!(
                export.contains(needle),
                "{shell:?} export's __gpy_segment_bg must carry {needle:?}, got:\n{export}"
            );
        }
    }
}

#[test]
fn fish_export_has_no_segment_bg_function() {
    let export = golden_export(Shell::Fish);
    assert!(
        !export.contains("__gpy_segment_bg"),
        "Fish export must be untouched by the Bash/Zsh __gpy_segment_bg addition, got:\n{export}"
    );
}

/// Companion golden for the unset/absent arms the populated fixture misses.
#[test]
fn default_theme_export_matches_golden() {
    let export = theme_to_shell(
        &ThemeConfig::default(),
        "default",
        &Config::default(),
        Shell::Fish,
        &Palette::default(),
        &BTreeMap::new(),
    );
    insta::assert_snapshot!("default_theme_fish_export", export);
}

fn shell_bin(shell: Shell) -> &'static str {
    match shell {
        Shell::Fish => "fish",
        Shell::Bash => "bash",
        Shell::Zsh => "zsh",
    }
}

/// Skip guard for hosts missing a shell. Checks exit status, not just spawn: a
/// broken `bash.exe` on Windows CI otherwise passes. Diagnostics show on failure.
fn shell_available(shell: Shell) -> bool {
    let bin = shell_bin(shell);
    match Command::new(bin).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let banner = String::from_utf8_lossy(&output.stdout);
            let first_line = banner.lines().next().unwrap_or("(no version banner)");
            eprintln!("using {shell:?}: `{bin} --version` -> {first_line}");
            true
        }
        Ok(output) => {
            eprintln!(
                "skipping {shell:?}: `{bin} --version` spawned but exited {}",
                output.status
            );
            false
        }
        Err(error) => {
            eprintln!("skipping {shell:?}: cannot run `{bin}`: {error}");
            false
        }
    }
}

/// Skip a missing shell locally; fail under `CI` unless `GPY_CI_NO_SHELLS=1`.
fn skip_shell_or_fail_under_ci(shell: Shell) {
    eprintln!("SKIP: {shell:?} is not available");
    let ci = std::env::var_os("CI").is_some_and(|value| !value.is_empty());
    let no_shells_declared = std::env::var_os("GPY_CI_NO_SHELLS").is_some();
    assert!(
        !ci || no_shells_declared,
        "{shell:?} is not available under CI; install it or set GPY_CI_NO_SHELLS=1"
    );
}

/// A collision-free marker path a successful injection would `touch`.
fn unique_marker(tag: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |d| d.as_nanos());
    std::env::temp_dir().join(format!(
        "gpy_test_pwned_{tag}_{}_{nanos}",
        std::process::id()
    ))
}

/// Build the exact double-quoted assignment the exporter emits for `raw`,
/// source it in the real shell, and echo the resulting variable back.
/// Returns `(sourcing_succeeded, echoed_value)`.
fn source_value(shell: Shell, raw: &str) -> (bool, String) {
    let syntax = shell.variable_syntax();
    let escaped = escape_shell_value(raw, shell);
    let assignment = syntax.format("__gpy_injection_probe", &escaped);
    let script = format!("{assignment}\nprintf '%s' \"$__gpy_injection_probe\"\n");
    let output = Command::new(shell_bin(shell))
        .arg("-c")
        .arg(&script)
        .output()
        .expect("spawning shell should succeed");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn escape_shell_value_neutralizes_metacharacters() {
    // `$(…)` command substitution is neutralized for all shells.
    assert_eq!(escape_shell_value("$(x)", Shell::Fish), "\\$(x)");
    assert_eq!(escape_shell_value("$(x)", Shell::Bash), "\\$(x)");
    assert_eq!(escape_shell_value("$(x)", Shell::Zsh), "\\$(x)");
    // Backslash is doubled (so a trailing backslash cannot escape the quote).
    assert_eq!(escape_shell_value("a\\", Shell::Fish), "a\\\\");
    // Double quote is escaped last (preserves historical behavior).
    assert_eq!(escape_shell_value("a\"b", Shell::Fish), "a\\\"b");
    // Backtick: escaped for bash/zsh, literal for fish (no backtick substitution).
    assert_eq!(escape_shell_value("`x`", Shell::Bash), "\\`x\\`");
    assert_eq!(escape_shell_value("`x`", Shell::Zsh), "\\`x\\`");
    assert_eq!(escape_shell_value("`x`", Shell::Fish), "`x`");
    // Backslash doubled before `$` is escaped, so the inserted `\` isn't re-doubled.
    assert_eq!(escape_shell_value("\\$", Shell::Fish), "\\\\\\$");
}

#[test]
fn theme_values_do_not_execute_command_substitution_when_sourced() {
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        if !shell_available(shell) {
            skip_shell_or_fail_under_ci(shell);
            continue;
        }
        let marker = unique_marker("cmdsub");
        let _ = std::fs::remove_file(&marker);
        assert!(!marker.exists(), "marker must not pre-exist");
        let payload = format!("$(touch {})", marker.display());

        let (ok, echoed) = source_value(shell, &payload);

        assert!(ok, "{shell:?}: sourcing the escaped assignment errored");
        assert!(
            !marker.exists(),
            "{shell:?}: command substitution EXECUTED — marker was created"
        );
        assert_eq!(
            echoed, payload,
            "{shell:?}: value should round-trip as inert literal text"
        );
        let _ = std::fs::remove_file(&marker);
    }
}

#[test]
fn theme_values_do_not_execute_backtick_substitution_when_sourced() {
    // Fish has no backtick command substitution, so this vector is bash/zsh.
    for shell in [Shell::Bash, Shell::Zsh] {
        if !shell_available(shell) {
            skip_shell_or_fail_under_ci(shell);
            continue;
        }
        let marker = unique_marker("backtick");
        let _ = std::fs::remove_file(&marker);
        assert!(!marker.exists(), "marker must not pre-exist");
        let payload = format!("`touch {}`", marker.display());

        let (ok, echoed) = source_value(shell, &payload);

        assert!(ok, "{shell:?}: sourcing the escaped assignment errored");
        assert!(
            !marker.exists(),
            "{shell:?}: backtick substitution EXECUTED — marker was created"
        );
        assert_eq!(
            echoed, payload,
            "{shell:?}: value should round-trip as inert literal text"
        );
        let _ = std::fs::remove_file(&marker);
    }
}

#[test]
fn theme_values_with_shell_metacharacters_source_cleanly() {
    let values = [
        "back\\slash",
        "dollar$var",
        "tick`cmd`",
        "quote\"end",
        "line1\nline2",
        "mix $(x) `y` \"z\" \\ end",
    ];
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        if !shell_available(shell) {
            skip_shell_or_fail_under_ci(shell);
            continue;
        }
        for raw in values {
            let (ok, echoed) = source_value(shell, raw);
            assert!(ok, "{shell:?}: sourcing value {raw:?} errored");
            assert_eq!(
                echoed, raw,
                "{shell:?}: value {raw:?} did not round-trip losslessly"
            );
        }
    }
}

#[test]
fn trailing_backslash_value_does_not_break_sourcing() {
    // A trailing backslash must not escape the closing quote.
    let raw = "icon\\";
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        if !shell_available(shell) {
            skip_shell_or_fail_under_ci(shell);
            continue;
        }
        let (ok, echoed) = source_value(shell, raw);
        assert!(
            ok,
            "{shell:?}: trailing-backslash value broke sourcing (unterminated string)"
        );
        assert_eq!(
            echoed, raw,
            "{shell:?}: trailing backslash did not round-trip"
        );
    }
}

/// Shell script body printing every `__*` variable as `name=value` lines.
const fn dump_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Fish => "for v in (set -n | string match '__*'); echo $v=$$v; end",
        Shell::Bash => r#"for v in $(compgen -v __); do printf '%s=%s\n' "$v" "${!v}"; done"#,
        Shell::Zsh => r#"for v in ${(k)parameters[(I)__*]}; do print -r -- "$v=${(P)v}"; done"#,
    }
}

/// Source `files` in order in a clean shell and return the sorted dump.
fn sourced_vars(shell: Shell, files: &[&std::path::Path]) -> Vec<String> {
    let mut script = String::new();
    for index in 1..=files.len() {
        let line = match shell {
            Shell::Fish => format!("source $argv[{index}]\n"),
            Shell::Bash | Shell::Zsh => format!("source \"${index}\"\n"),
        };
        script.push_str(&line);
    }
    script.push_str(dump_script(shell));
    let mut cmd = Command::new(shell_bin(shell));
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    match shell {
        Shell::Fish => cmd.args(["--no-config", "-c", &script, "--"]),
        Shell::Bash => cmd.args(["--noprofile", "--norc", "-c", &script, "bash"]),
        Shell::Zsh => cmd.args(["-f", "-c", &script, "zsh"]),
    };
    let output = cmd.args(files).output().expect("spawning shell");
    assert!(
        output.status.success(),
        "{shell:?}: sourcing failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

/// Hot-reload invariant: sourcing B over A equals sourcing only B (no `__*` leaks).
#[test]
fn optional_shell_vars_reset_on_resource() {
    use crate::config::types::Icon;

    let mut theme_a = ThemeConfig::default();
    theme_a.segments.clock.time_format = Some("24".to_owned());
    theme_a.segments.status.ok_icon = Some(Icon::new("OK").expect("valid icon"));
    theme_a.segments.status.fail_icon = Some(Icon::new("NO").expect("valid icon"));
    let theme_b = ThemeConfig::default();
    let config = Config::default();
    let dir = tempfile::tempdir().expect("tempdir");

    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        if !shell_available(shell) {
            skip_shell_or_fail_under_ci(shell);
            continue;
        }
        let export = |theme: &ThemeConfig| {
            theme_to_shell(
                theme,
                "t",
                &config,
                shell,
                &Palette::default(),
                &BTreeMap::new(),
            )
        };
        let path_a = dir
            .path()
            .join(format!("a.{shell_bin}", shell_bin = shell_bin(shell)));
        let path_b = dir
            .path()
            .join(format!("b.{shell_bin}", shell_bin = shell_bin(shell)));
        std::fs::write(&path_a, export(&theme_a)).expect("write a");
        std::fs::write(&path_b, export(&theme_b)).expect("write b");

        let only_a = sourced_vars(shell, &[&path_a]);
        assert!(
            only_a.contains(&"__time_format=24".to_owned()),
            "{shell:?}: export A must set the 24h clock, got {only_a:?}"
        );
        let fresh = sourced_vars(shell, &[&path_b]);
        let reloaded = sourced_vars(shell, &[&path_a, &path_b]);
        assert_eq!(reloaded, fresh, "{shell:?}: A-then-B must equal fresh B");
        assert!(
            reloaded.contains(&"__time_format=12".to_owned()),
            "{shell:?}: B must reset the clock to 12h, got {reloaded:?}"
        );
    }
}

/// Fish's `set_color` rejects bare ANSI indices and palette roles, so the Fish
/// export carries names/hex for them; Bash/Zsh keep the index their renderers
/// draw as a 256-color SGR, and get the role's palette value.
#[test]
fn exported_index_and_role_colors_reach_real_set_color() {
    use crate::config::types::ColorSpec;
    use crate::template::Color;
    use std::collections::HashMap;

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
    let export = |shell| {
        theme_to_shell(
            &theme,
            "t",
            &Config::default(),
            shell,
            &palette,
            &BTreeMap::new(),
        )
    };
    let dir = tempfile::tempdir().expect("tempdir");

    if shell_available(Shell::Fish) {
        let path = dir.path().join("export.fish");
        std::fs::write(&path, export(Shell::Fish)).expect("write fish export");
        let output = Command::new("fish")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .args([
                "--no-config",
                "-c",
                "source $argv[1]; set -g fish_term24bit 1
                 set_color $__color_clock_bg; set_color $__color_clock_fg
                 set_color -b $__color_status_ok_bg",
                "--",
            ])
            .arg(&path)
            .output()
            .expect("spawning fish");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "",
            "set_color must accept every exported color"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "\x1b[38;2;0;215;135m\x1b[33m\x1b[48;2;250;179;135m",
            "42 is xterm #00d787, 3 is yellow, orange is the palette's #fab387"
        );
    } else {
        skip_shell_or_fail_under_ci(Shell::Fish);
    }

    if shell_available(Shell::Bash) {
        let path = dir.path().join("export.bash");
        std::fs::write(&path, export(Shell::Bash)).expect("write bash export");
        let vars = sourced_vars(Shell::Bash, &[&path]);
        for expected in [
            "__color_clock_bg=42",
            "__color_clock_fg=3",
            "__color_status_ok_bg=#fab387",
        ] {
            assert!(
                vars.contains(&expected.to_owned()),
                "bash export lacks {expected}"
            );
        }
    } else {
        skip_shell_or_fail_under_ci(Shell::Bash);
    }
}

/// A palette entry shadows an ANSI name for shell-drawn colors, including its
/// alias spellings, as it does in the agent's renderer; an ANSI index still
/// names the terminal's own color. Checked on what each shell actually reads.
#[test]
fn exported_ansi_names_follow_the_palette_and_indices_do_not() {
    use crate::config::types::ColorSpec;
    use crate::template::Color;
    use std::collections::HashMap;

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
    let export = |shell| {
        theme_to_shell(
            &theme,
            "t",
            &Config::default(),
            shell,
            &palette,
            &BTreeMap::new(),
        )
    };
    let dir = tempfile::tempdir().expect("tempdir");

    if shell_available(Shell::Fish) {
        let path = dir.path().join("export.fish");
        std::fs::write(&path, export(Shell::Fish)).expect("write fish export");
        let output = Command::new("fish")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .args([
                "--no-config",
                "-c",
                "source $argv[1]; set -g fish_term24bit 1
                 set_color -b $__color_clock_bg; set_color $__color_clock_fg
                 set_color -b $__color_status_ok_bg",
                "--",
            ])
            .arg(&path)
            .output()
            .expect("spawning fish");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "\x1b[48;2;243;139;168m\x1b[31m\x1b[48;2;203;166;247m",
            "red and magenta take the palette's hex, index 1 stays the terminal's red"
        );
    } else {
        skip_shell_or_fail_under_ci(Shell::Fish);
    }

    for shell in [Shell::Bash, Shell::Zsh] {
        if !shell_available(shell) {
            skip_shell_or_fail_under_ci(shell);
            continue;
        }
        let path = dir.path().join(format!("export.{shell:?}"));
        std::fs::write(&path, export(shell)).expect("write export");
        let vars = sourced_vars(shell, &[&path]);
        for expected in [
            "__color_clock_bg=#f38ba8",
            "__color_clock_fg=1",
            "__color_status_ok_bg=#cba6f7",
        ] {
            assert!(
                vars.contains(&expected.to_owned()),
                "{shell:?} export lacks {expected}"
            );
        }
    }
}
