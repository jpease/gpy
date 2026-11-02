//! Warnings about a loaded theme that parses and validates but contains keys
//! or templates that do nothing.
//!
//! [`theme_warnings`] runs every entry of [`CHECKS`] over the merged theme and
//! returns human-readable lines, each naming the theme (and its file when it
//! came from disk). They are warnings, never load errors: no theme that loads
//! stops loading, and rendering is unchanged. `gpy theme validate` and
//! `gpy doctor` print them; the per-file `unset` lint is
//! [`overlay::unset_warnings`](super::overlay::unset_warnings).
//!
//! To add a warning, write a [`Check`] and list it in [`CHECKS`].

use super::LoadedTheme;
use super::overlay::merged_table;
use crate::config::Config;
use crate::plugin::{BUNDLED_PLUGIN_NAMES, PluginDiscovery};

/// What a check may look at, with the line prefix naming the theme.
struct Inputs<'a> {
    loaded: &'a LoadedTheme,
    config: &'a Config,
    plugins: &'a PluginDiscovery,
    origin: String,
}

impl Inputs<'_> {
    /// `message` prefixed with the theme (and file) it is about.
    fn line(&self, message: impl std::fmt::Display) -> String {
        format!("{}: {message}", self.origin)
    }
}

/// One check: appends the warnings it finds to the output.
type Check = fn(&Inputs<'_>, &mut Vec<String>);

/// Every check, in the order their warnings are printed.
const CHECKS: &[Check] = &[
    unknown_keys,
    unknown_sections,
    language_colour_keys,
    missing_formats,
    git_icon_controls,
];

/// Warnings for the merged, validated theme `loaded`, under `config` (which
/// segments are enabled) and the installed `plugins`.
#[must_use]
pub fn theme_warnings(
    loaded: &LoadedTheme,
    config: &Config,
    plugins: &PluginDiscovery,
) -> Vec<String> {
    let top = loaded.chain.top();
    let origin = if top.on_disk {
        format!("theme '{}' ({})", top.name, top.path.display())
    } else {
        format!("theme '{}'", top.name)
    };
    let inputs = Inputs {
        loaded,
        config,
        plugins,
        origin,
    };
    let mut warnings = Vec::new();
    for check in CHECKS {
        check(&inputs, &mut warnings);
    }
    warnings
}

/// Keys the merged theme sets that the typed theme dropped.
fn unknown_keys(inputs: &Inputs<'_>, out: &mut Vec<String>) {
    let Some(merged) = merged_table(inputs.loaded.chain.top(), true) else {
        return;
    };
    let Ok(typed) = toml::Table::try_from(&inputs.loaded.theme) else {
        return;
    };
    let mut leaves = Vec::new();
    leaf_paths(&merged, &mut Vec::new(), &mut leaves);
    leaves.sort();
    for path in leaves {
        if !is_known(&typed, &path) {
            out.push(inputs.line(format_args!("unknown key `{}` (ignored)", path.join("."))));
        }
    }
}

/// The dotted-path leaves of `table`, as key lists.
fn leaf_paths(table: &toml::Table, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for (key, value) in table {
        prefix.push(key.clone());
        match value {
            toml::Value::Table(inner) => leaf_paths(inner, prefix, out),
            _ => out.push(prefix.clone()),
        }
        prefix.pop();
    }
}

/// Whether the serialized typed theme holds `path`, or a value at an ancestor
/// of it (a typed scalar or array owns everything below its key).
fn is_known(typed: &toml::Table, path: &[String]) -> bool {
    let mut table = typed;
    for key in path {
        match table.get(key) {
            None => return false,
            Some(toml::Value::Table(inner)) => table = inner,
            Some(_) => return true,
        }
    }
    true
}

/// `[segments.<id>]` sections no built-in, bundled plugin or installed plugin owns.
fn unknown_sections(inputs: &Inputs<'_>, out: &mut Vec<String>) {
    let mut ids: Vec<&String> = inputs.loaded.theme.segments.plugin.keys().collect();
    ids.sort();
    for id in ids {
        let provided = inputs.plugins.plugins.iter().any(|plugin| {
            plugin
                .manifest
                .provided_segments
                .iter()
                .any(|segment| segment.as_str() == id)
        });
        if !provided && !BUNDLED_PLUGIN_NAMES.contains(&id.as_str()) {
            out.push(inputs.line(format_args!(
                "`[segments.{id}]`: no built-in or installed plugin segment has this name"
            )));
        }
    }
}

/// `[segments.language]` per-language colour keys that read as neither
/// `<lang>_bg_color` nor `<lang>_text_color`.
fn language_colour_keys(inputs: &Inputs<'_>, out: &mut Vec<String>) {
    let overrides = &inputs.loaded.theme.segments.language.overrides;
    let mut keys: Vec<&String> = overrides.keys().collect();
    keys.sort();
    for key in keys {
        if !key.ends_with("_bg_color") && !key.ends_with("_text_color") {
            out.push(inputs.line(format_args!(
                "`segments.language.{key}` is not a colour key (`<lang>_bg_color`/`<lang>_text_color`); ignored"
            )));
        }
    }
}

/// Enabled agent-rendered segments with no `format`, which render nothing.
fn missing_formats(inputs: &Inputs<'_>, out: &mut Vec<String>) {
    let segments = &inputs.loaded.theme.segments;
    for (name, format) in [
        ("directory", &segments.directory.format),
        ("duration", &segments.duration.format),
        ("git", &segments.git.format),
        ("language", &segments.language.format),
    ] {
        if format.is_none() && inputs.config.segment_enabled(name) {
            out.push(inputs.line(format_args!(
                "`segments.{name}` has no `format`, so it renders nothing"
            )));
        }
    }
}

/// Git icon overrides holding control characters, which are stripped when drawn.
fn git_icon_controls(inputs: &Inputs<'_>, out: &mut Vec<String>) {
    let git = &inputs.loaded.theme.segments.git;
    for (key, icon) in [
        ("staged_icon", &git.staged_icon),
        ("unstaged_icon", &git.unstaged_icon),
        ("untracked_icon", &git.untracked_icon),
        ("conflicts_icon", &git.conflicts_icon),
        ("stash_icon", &git.stash_icon),
    ] {
        if icon
            .as_deref()
            .is_some_and(|text| text.chars().any(char::is_control))
        {
            out.push(inputs.line(format_args!(
                "`segments.git.{key}` contains control characters; they are removed when the segment is drawn"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::manager::BUILTIN_THEMES;
    use crate::theme::overlay::ChainLink;

    /// Warnings for the embedded builtin `name`, with no plugins installed.
    fn builtin_warnings(name: &str) -> Vec<String> {
        let loaded = crate::theme::overlay::load_link(ChainLink::embedded(name))
            .expect("builtin theme loads");
        theme_warnings(&loaded, &Config::default(), &PluginDiscovery::default())
    }

    /// Warnings for a theme file written to a temp user theme dir.
    fn file_warnings(text: &str, config: &Config) -> Vec<String> {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("probe.toml");
        std::fs::write(&path, text).expect("write theme");
        let loaded = crate::theme::load::load_theme_file(&path).expect("theme loads");
        theme_warnings(&loaded, config, &PluginDiscovery::default())
    }

    /// A full theme text: the default builtin with `extra` appended.
    fn default_plus(extra: &str) -> String {
        format!(
            "{}\n{extra}",
            crate::config::defaults::DEFAULT_THEME_CONTENT
        )
    }

    #[test]
    fn builtin_themes_have_no_warnings_without_installed_plugins() {
        for (name, _) in BUILTIN_THEMES {
            assert_eq!(builtin_warnings(name), Vec::<String>::new(), "{name}");
        }
    }

    #[test]
    fn unknown_typed_key_is_reported() {
        let default = crate::config::defaults::DEFAULT_THEME_CONTENT;
        let spliced = default.replacen(
            "[segments.clock]\n",
            "[segments.clock]\ntime_fromat = \"24\"\n",
            1,
        );
        assert_ne!(spliced, default);

        let warnings = file_warnings(&spliced, &Config::default());

        assert!(
            warnings
                .iter()
                .any(|w| w.contains("unknown key `segments.clock.time_fromat` (ignored)")),
            "{warnings:?}"
        );
    }

    #[test]
    fn unknown_segment_section_is_reported() {
        let warnings = file_warnings(
            &default_plus("[segments.gitt]\nbg_color = \"red\"\n"),
            &Config::default(),
        );

        assert!(
            warnings.iter().any(|w| w.contains("`[segments.gitt]`")),
            "{warnings:?}"
        );
    }

    #[test]
    fn bundled_plugin_section_is_not_reported() {
        // The builtin themes style `username` and `hostname`.
        let warnings = builtin_warnings("default");

        assert!(
            warnings.iter().all(|w| !w.contains("[segments.username]")),
            "{warnings:?}"
        );
    }

    #[test]
    fn misspelled_language_colour_key_is_reported() {
        let spliced = crate::config::defaults::DEFAULT_THEME_CONTENT.replacen(
            "[segments.language]\n",
            "[segments.language]\nrust_bgcolor = \"red\"\n",
            1,
        );

        let warnings = file_warnings(&spliced, &Config::default());

        assert!(
            warnings.iter().any(|w| w.contains("rust_bgcolor")),
            "{warnings:?}"
        );
    }

    #[test]
    fn enabled_segment_without_format_is_reported() {
        let theme = crate::theme::ThemeConfig::default();
        assert!(theme.segments.git.format.is_none());
        let warnings = warnings_for_theme(theme, &Config::default());

        assert!(
            warnings
                .iter()
                .any(|w| w.contains("`segments.git` has no `format`, so it renders nothing")),
            "{warnings:?}"
        );
    }

    #[test]
    fn disabled_segment_without_format_is_not_reported() {
        let mut config = Config::default();
        config
            .ui
            .enabled_segments
            .retain(|segment| segment != "git");
        let warnings = warnings_for_theme(crate::theme::ThemeConfig::default(), &config);

        assert!(
            warnings.iter().all(|w| !w.contains("`segments.git`")),
            "{warnings:?}"
        );
    }

    #[test]
    fn control_character_in_git_icon_is_reported() {
        let spliced = crate::config::defaults::DEFAULT_THEME_CONTENT.replacen(
            "[segments.git]\n",
            "[segments.git]\nstaged_icon = \"+\\u0007\"\n",
            1,
        );

        let warnings = file_warnings(&spliced, &Config::default());

        assert!(
            warnings.iter().any(|w| {
                w.contains("segments.git.staged_icon") && w.contains("control characters")
            }),
            "{warnings:?}"
        );
    }

    /// Warnings for an in-memory theme, as a loaded theme of the embedded `default`
    /// chain with `theme` swapped in (the merged table is not consulted by the
    /// checks under test).
    fn warnings_for_theme(theme: crate::theme::ThemeConfig, config: &Config) -> Vec<String> {
        let mut loaded = crate::theme::overlay::load_link(ChainLink::embedded("default"))
            .expect("builtin theme loads");
        loaded.theme = theme;
        theme_warnings(&loaded, config, &PluginDiscovery::default())
    }
}
