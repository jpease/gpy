//! The `gpy doctor` "Agent tuning" section: the effective value of every
//! `GPY_*` variable the agent reads, and where it came from.

use crate::commands::output::{self, Status};
use crate::env_config::{AGENT_ENV_VARS, Source};

/// Print one line per [`AGENT_ENV_VARS`] entry. An ignored value is a
/// warning; it never fails `doctor`.
pub(super) fn print_agent_tuning() {
    output::section("Agent tuning");
    output::detail(
        "Values from this shell's environment; a running agent keeps the values it started with.",
    );
    for var in AGENT_ENV_VARS {
        let (value, source) = var.effective();
        match source {
            Source::Env => output::detail(format_args!("{} = {value} (env)", var.name)),
            Source::Default => output::detail(format_args!("{} = {value} (default)", var.name)),
            Source::Ignored(raw) => output::detail_status(
                1,
                Status::Warn,
                format_args!("{} = {value} (default; ignored \"{raw}\")", var.name),
            ),
        }
    }
}
