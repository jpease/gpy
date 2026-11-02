use super::theme_to_shell::{PluginExports, theme_to_shell};
use crate::config::Config;
use crate::shell::Shell;
use crate::template::Palette;
use crate::theme::model::ThemeConfig;

const MIRRORS: [&str; 5] = [
    "__gpy_cfg_agent_enabled",
    "__gpy_cfg_supervisor_enabled",
    "__gpy_cfg_supervisor_check_interval",
    "__gpy_cfg_supervisor_max_restarts",
    "__gpy_cfg_transient_prompt",
];

fn export(shell: Shell) -> String {
    theme_to_shell(
        &ThemeConfig::default(),
        "default",
        &Config::default(),
        shell,
        &Palette::default(),
        &PluginExports::default(),
    )
}

#[test]
fn version_sentinel_is_the_first_line_of_every_export() {
    for (shell, expected) in [
        (Shell::Fish, "set -g __gpy_export_version \"1\""),
        (Shell::Bash, "__gpy_export_version=\"1\""),
        (Shell::Zsh, "typeset -g __gpy_export_version=\"1\""),
    ] {
        let out = export(shell);
        assert_eq!(out.lines().next(), Some(expected), "{shell}: {out}");
    }
}

#[test]
fn each_mirror_appears_once_as_an_unexported_assignment() {
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        let out = export(shell);
        for name in MIRRORS {
            let count = out
                .lines()
                .filter(|l| l.split([' ', '=']).any(|word| word == name))
                .count();
            assert_eq!(count, 1, "{shell}: `{name}` count in:\n{out}");
        }
    }
}

#[test]
fn no_gpy_name_is_assigned_or_exported() {
    for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
        let out = export(shell);
        for line in out.lines().filter(|l| !l.starts_with('#')) {
            let assigns_gpy = line
                .split([' ', '='])
                .take(4)
                .any(|word| word.starts_with("GPY_"));
            assert!(!assigns_gpy, "{shell}: GPY_ name in `{line}`");
            assert!(
                !line.starts_with("export ") && !line.starts_with("set -gx "),
                "{shell}: exported line `{line}`"
            );
        }
    }
}
