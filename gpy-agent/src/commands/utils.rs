//! Config load/save, agent binary lookup and agent reload helpers shared by CLI commands.

use crate::commands::output;
use crate::config::discovery::Discovered;
use crate::{Error, Result, config};
use std::path::{Path, PathBuf};

/// The config path CLI commands read and write: the highest-priority existing
/// config file, else the highest-priority candidate, so a new file is never
/// shadowed by a higher-priority one.
///
/// # Errors
/// Returns an error if no candidate path can be constructed.
pub fn active_config_path() -> Result<PathBuf> {
    let paths = config::schema::get_config_paths();

    config::schema::resolve_active_config(&paths)
        .map(|(write_path, _existing)| write_path)
        .ok_or_else(|| {
            Error::config(
                "Unable to determine a config path from GPY_CONFIG_PATH, XDG_CONFIG_HOME, or HOME"
                    .to_owned(),
            )
        })
}

/// Load the config at `path`; a missing file yields defaults, but an invalid
/// one is an error, so a failed mutation never overwrites it.
///
/// # Errors
/// Returns an error if the file exists but cannot be read, parsed, or validated.
pub fn load_active_config(path: &Path) -> Result<config::Config> {
    match std::fs::read_to_string(path) {
        Ok(contents) => config::loader::parse_config_toml(&contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(config::Config::default()),
        Err(e) => Err(Error::config(format!(
            "Failed to read config file {}: {e}",
            path.display()
        ))),
    }
}

/// Load the active configuration; an existing-but-invalid config is an error.
///
/// # Errors
/// Returns an error if the active config exists but cannot be read, parsed, or validated.
pub fn read_config() -> Result<config::Config> {
    let path = active_config_path()?;
    load_active_config(&path)
}

/// One `name [source]` line per discovered theme or palette, with ` *` after
/// the `active` one, and `, <note>` added inside the brackets of every
/// item `note` returns text for.
fn discovered_lines_noted(
    active: &str,
    items: &[Discovered],
    note: impl Fn(&Discovered) -> Option<String>,
) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            let marker = if item.name == active { " *" } else { "" };
            note(item).map_or_else(
                || format!("{}{marker} [{}]", item.name, item.source),
                |text| format!("{}{marker} [{}, {text}]", item.name, item.source),
            )
        })
        .collect()
}

/// Print `items` under `title`, marking the `active` one.
pub fn print_discovered(title: &str, active: &str, items: &[Discovered]) {
    print_discovered_noted(title, active, items, |_| None);
}

/// As [`print_discovered`], appending `note`'s text to an item's source label.
pub fn print_discovered_noted(
    title: &str,
    active: &str,
    items: &[Discovered],
    note: impl Fn(&Discovered) -> Option<String>,
) {
    output::heading(title);
    for line in discovered_lines_noted(active, items, note) {
        output::detail(line);
    }
}

/// The `gpy-agent` installed beside this `gpy`, else `gpy-agent` from `PATH`
/// (split installs).
#[must_use]
pub fn agent_binary() -> PathBuf {
    std::env::current_exe().map_or_else(
        |_| PathBuf::from("gpy-agent"),
        |exe| agent_binary_beside(&exe),
    )
}

fn agent_binary_beside(exe: &Path) -> PathBuf {
    exe.parent()
        .map(|dir| dir.join(format!("gpy-agent{}", std::env::consts::EXE_SUFFIX)))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| PathBuf::from("gpy-agent"))
}

/// `name` as an executable on `PATH`.
pub(crate) fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Reload the running agent's config. Prints nothing when no agent runs (the
/// agent reads the config when it starts) or when the reload was confirmed;
/// warns when a running agent rejected the config or did not answer. The
/// config is already written, so there is nothing to return.
#[cfg(unix)]
pub fn reload_agent_and_notify() {
    use crate::agent::lifecycle::ConfigReload;

    let outcome = tokio::runtime::Runtime::new()
        .map_err(|e| Error::agent(format!("cannot create the async runtime: {e}")))
        .and_then(|runtime| {
            runtime.block_on(crate::agent::lifecycle::send_config_reload_command())
        });

    match outcome {
        Ok(ConfigReload::Applied | ConfigReload::NotRunning) => {}
        Ok(ConfigReload::Rejected(reason)) => crate::commands::output::warn(format_args!(
            "Agent rejected the new config and kept its previous one: {reason}"
        )),
        Err(e) => crate::commands::output::warn(format_args!("Agent not reloaded: {e}")),
    }
}

/// Native Windows never runs the agent, so there is nothing to reload; see
/// `crate::ipc::native_windows_unsupported`.
#[cfg(not(unix))]
pub fn reload_agent_and_notify() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::discovery::Source;

    fn discovered(name: &str, source: Source) -> Discovered {
        Discovered {
            name: name.to_owned(),
            source,
            path: None,
        }
    }

    #[test]
    fn discovered_lines_mark_only_the_active_entry_and_label_each_source() {
        let items = [
            discovered("default", Source::Builtin),
            discovered("mine", Source::User),
            discovered(
                "from-plugin",
                Source::Plugin {
                    plugin_id: "acme".to_owned(),
                },
            ),
        ];
        assert_eq!(
            discovered_lines_noted("mine", &items, |_| None),
            [
                "default [builtin]",
                "mine * [user]",
                "from-plugin [plugin:acme]"
            ]
        );
    }

    #[test]
    fn agent_binary_falls_back_to_path_without_a_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gpy");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(agent_binary_beside(&exe), PathBuf::from("gpy-agent"));
    }

    #[test]
    fn agent_binary_prefers_the_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gpy");
        let sibling = dir
            .path()
            .join(format!("gpy-agent{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&exe, "").unwrap();
        std::fs::write(&sibling, "").unwrap();
        assert_eq!(agent_binary_beside(&exe), sibling);
    }
}
