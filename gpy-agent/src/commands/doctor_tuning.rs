//! The `gpy doctor` "Agent tuning" and "Prompt environment" sections: the
//! effective value of every `GPY_*` variable the agent and `gpy prompt`
//! read, and where it came from.

use crate::commands::output::{self, Status};
use crate::env_config::{AGENT_ENV_VARS, EnvVar, PROMPT_ENV_VARS, Source};

/// Print one line per [`AGENT_ENV_VARS`] entry. An ignored value is a
/// warning; it never fails `doctor`.
pub(super) fn print_agent_tuning() {
    output::section("Agent tuning");
    output::detail(
        "Values from this shell's environment; a running agent keeps the values it started with.",
    );
    print_vars(AGENT_ENV_VARS);
}

/// Print one line per [`PROMPT_ENV_VARS`] entry, as [`print_agent_tuning`].
pub(super) fn print_prompt_environment() {
    output::section("Prompt environment");
    output::detail(
        "Values from this shell's environment; `gpy prompt` reads them on every prompt.",
    );
    print_vars(PROMPT_ENV_VARS);
}

fn print_vars(vars: &[&EnvVar]) {
    for var in vars {
        let (value, source) = var.effective();
        match source {
            Source::Env => output::detail(format_args!("{} = {value} (env)", var.name)),
            Source::Default => output::detail(format_args!("{} = {value} (default)", var.name)),
            Source::Ignored { raw, expected } => output::detail_status(
                1,
                Status::Warn,
                format_args!(
                    "{} = {value} (default; ignored \"{raw}\": expected {expected})",
                    var.name
                ),
            ),
        }
    }
}
