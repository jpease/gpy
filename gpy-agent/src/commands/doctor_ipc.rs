//! Doctor check for the IPC client Bash and Fish need to reach the agent.

use super::setup::InstalledIntegration;
use super::utils::find_in_path;
use crate::commands::output;
use crate::config;
use crate::shell::Shell;
use std::path::Path;
use std::process::{Command, Stdio};

/// Warn when an installed Bash or Fish integration has no way to talk to the
/// agent: they use `socat`, or an `nc` that supports `-U` (Unix sockets), the
/// same choice the shell scripts make at startup. Without one, every prompt
/// segment falls back to `gpy-agent oneshot`. Zsh has a socket builtin and
/// needs neither, and a disabled agent is never contacted, so both are
/// skipped silently.
///
/// Only ever warns, so it always returns `true`.
pub(super) fn check_ipc_client(
    installed: &[InstalledIntegration],
    cfg: Option<&config::Config>,
) -> bool {
    if cfg.is_some_and(|loaded| !loaded.agent.enabled) {
        return true;
    }
    if !installed
        .iter()
        .any(|integration| matches!(integration.shell, Shell::Bash | Shell::Fish))
    {
        return true;
    }

    output::step("Checking IPC client");
    if let Some(socat) = find_in_path("socat") {
        output::ok(format_args!("socat ({})", socat.display()));
    } else if let Some(nc) = find_in_path("nc").filter(|nc| nc_supports_unix_sockets(nc)) {
        output::ok(format_args!("nc -U ({})", nc.display()));
    } else {
        output::warn("No IPC client for Bash/Fish: every prompt runs `gpy-agent oneshot`");
        output::detail(
            "Remediation: install socat (macOS: `brew install socat`; Debian/Ubuntu: `apt install socat`), or an `nc` that supports `-U`.",
        );
    }
    true
}

/// Whether `nc -h` mentions `-U`, in either stream and whatever its exit
/// status (many implementations print usage to stderr and exit non-zero).
fn nc_supports_unix_sockets(nc: &Path) -> bool {
    let Ok(help) = Command::new(nc).arg("-h").stdin(Stdio::null()).output() else {
        return false;
    };
    String::from_utf8_lossy(&help.stdout).contains("-U")
        || String::from_utf8_lossy(&help.stderr).contains("-U")
}
