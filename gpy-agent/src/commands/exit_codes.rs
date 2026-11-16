//! Exit codes of the lifecycle commands and `setup` (the LSB init-script codes) and the
//! `--help` text that documents them, shared by `gpy` and `gpy-agent` so the
//! two binaries cannot disagree.

/// LSB "program is not running": `status` found no socket, or a socket file
/// that refuses connections.
pub const NOT_RUNNING: u8 = 3;

/// LSB "program or service status is unknown": the socket accepts but the
/// agent never replies, or the status could not be determined.
pub const STATUS_UNKNOWN: u8 = 4;

/// LSB "program is not configured": `start` found `agent.enabled = false`.
pub const DISABLED: u8 = 6;

/// LSB "program is not installed": `setup` found an incomplete GPY block in
/// the rc file, so the shell does not load GPY. Everything else was installed.
pub const SETUP_INCOMPLETE: u8 = 5;

/// `after_help` of `status`.
pub const STATUS_HELP: &str =
    "Exit status: 0 running (or busy); 3 not running; 4 not answering or unknown; 2 usage error.";

/// `after_help` of `start`.
pub const START_HELP: &str = "Exit status: 0 agent running afterwards; 1 error (including an agent that cannot be verified); 6 disabled by agent.enabled = false; 2 usage error.";

/// `after_help` of `stop`.
pub const STOP_HELP: &str = "Exit status: 0 agent stopped afterwards (including not running); 1 the agent still answers or did not answer the shutdown; 2 usage error.";

/// `after_help` of `restart`.
pub const RESTART_HELP: &str = "Exit status: as start: 0 agent running afterwards; 1 error; 6 disabled by agent.enabled = false; 2 usage error.";

/// `after_help` of `config reload`.
pub const RELOAD_HELP: &str =
    "Exit status: 0 applied, or no agent running; 1 rejected or not answered; 2 usage error.";

/// `after_help` of `doctor`.
pub const DOCTOR_HELP: &str =
    "Exit status: 0 no failed check (warnings allowed); 1 a check failed; 2 usage error.";

/// `after_help` of `setup`.
pub const SETUP_HELP: &str = "Exit status: 0 set up; 1 error; 5 files installed, but the rc file holds an incomplete GPY block, so GPY is not active (delete those lines and re-run); 2 usage error.";

/// `after_help` of every command without a table row of its own.
pub const GENERIC_HELP: &str =
    "Exit status: 0 success, 1 failure (one `Error:` line on stderr), 2 usage error.";

/// Top-level `after_help` of `gpy`.
pub const GPY_SUMMARY_HELP: &str = "Exit status: 0 success; 1 failure (one `Error:` line on stderr); 2 usage error.\n\
status: 0 running (or busy); 3 not running; 4 not answering or unknown.\n\
start, restart: 6 disabled by agent.enabled = false.\n\
setup: 5 the rc file holds an incomplete GPY block; GPY is not active.\n\
doctor: 1 only when a check failed.";

/// Top-level `after_help` of `gpy-agent`.
pub const AGENT_SUMMARY_HELP: &str = "Exit status: 0 success; 1 failure (one `Error:` line on stderr); 2 usage error.\n\
status: 0 running (or busy); 3 not running; 4 not answering or unknown.\n\
start, restart: 6 disabled by agent.enabled = false.\n\
stop: 0 agent stopped afterwards, including not running.";
