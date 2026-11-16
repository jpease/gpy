//! Doctor checks that the shell integrations are wired to files that exist and
//! to the `gpy-agent` the shells will actually run.

use super::setup::InstalledIntegration;
use super::utils::find_in_path;
use crate::commands::output;
use std::path::Path;
use std::process::Command;

/// Check each shell whose rc file holds the GPY block: the block is complete,
/// the file it sources exists, and the scripts were written by this `gpy`.
/// Returns `false` when a shell would fail to load GPY. An older version
/// stamp only warns, since those scripts still work against the same protocol.
pub(super) fn check_shell_integrations(installed: &[InstalledIntegration]) -> bool {
    let mut healthy = true;
    for integration in installed {
        let shell = integration.shell;
        output::step(format_args!("Checking {shell} integration"));
        if integration.incomplete {
            output::fail(format_args!(
                "The GPY block in {} is incomplete, so {shell} does not load GPY",
                integration.rc.display()
            ));
            output::detail(format_args!(
                "Remediation: delete the lines from '# >>> gpy-init >>>' on that GPY added to {}, then run `gpy setup --shell {shell}`.",
                integration.rc.display()
            ));
            healthy = false;
            continue;
        }
        if let Some(entry) = integration
            .entry
            .as_deref()
            .filter(|entry| !entry.is_file())
        {
            output::fail(format_args!(
                "{} sources {}, which does not exist",
                integration.rc.display(),
                entry.display()
            ));
            output::detail(format_args!(
                "Remediation: run `gpy setup --shell {shell}` to install the scripts and update the block."
            ));
            healthy = false;
            continue;
        }
        match integration.version.as_deref() {
            Some(version) if version == crate::VERSION => {
                output::ok(format_args!("{version} ({})", integration.dir.display()));
            }
            found => {
                let from = found.map_or_else(
                    || "an older gpy".to_owned(),
                    |version| format!("gpy {version}"),
                );
                output::warn(format_args!(
                    "installed by {from}, but this is gpy {} ({})",
                    crate::VERSION,
                    integration.dir.display()
                ));
                output::detail(format_args!(
                    "Remediation: run `gpy setup --shell {shell}` to install the current scripts."
                ));
            }
        }
    }
    healthy
}

/// Warn when the `gpy-agent` the shell scripts run (the first one in `PATH`)
/// is missing or is not the binary `used`, whose `--version` output is
/// `used_version`. Only ever warns.
pub(super) fn check_path_agent(used: &Path, used_version: &[u8]) {
    let Some(on_path) = find_in_path("gpy-agent") else {
        output::warn("No gpy-agent in PATH, which is where the shell scripts look for it");
        output::detail(
            "Remediation: put the directory holding gpy-agent on your PATH, or the shells fall back to slower paths.",
        );
        return;
    };
    if same_file(&on_path, used) {
        return;
    }
    let ours = String::from_utf8_lossy(used_version).trim().to_owned();
    match Command::new(&on_path).arg("--version").output() {
        Ok(probe) if probe.status.success() => {
            let theirs = String::from_utf8_lossy(&probe.stdout).trim().to_owned();
            if theirs != ours {
                output::warn(format_args!(
                    "The gpy-agent your shells run ({}) reports '{theirs}', but gpy uses {} ('{ours}')",
                    on_path.display(),
                    used.display()
                ));
                output::detail(
                    "Remediation: remove the stale gpy-agent, or put the directory of the current one first in PATH.",
                );
            }
        }
        _ => {
            output::warn(format_args!(
                "The gpy-agent your shells run ({}) fails `--version`",
                on_path.display()
            ));
            output::detail(
                "Remediation: reinstall gpy-agent, or put the directory of a working one first in PATH.",
            );
        }
    }
}

/// Whether both paths name the same file once symlinks are resolved.
fn same_file(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(first), Ok(second)) => first == second,
        _ => left == right,
    }
}
