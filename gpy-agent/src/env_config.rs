//! Every `GPY_*` environment variable the Rust code reads, in one place.
//!
//! [`AGENT_ENV_VARS`] lists each variable with its kind, and every read goes
//! through that entry's typed accessor, so all variables treat whitespace,
//! zero and garbage the same way. The `parse_*` forms take the raw value so
//! callers and tests need no `std::env::set_var`.
//!
//! Rules per kind:
//! - Flags accept `1/true/yes/on` and `0/false/no/off`, case-insensitive and
//!   trimmed; the shells read the same table
//!   (`tests/fixtures/shell_env_flag_vectors.tsv` is checked against
//!   [`parse_flag`]).
//! - Positive integers: `0`, garbage or an out-of-range number is ignored.
//! - "Zero disables" integers: `0` means off; garbage is ignored.
//! - Paths: empty is unset; a relative path resolves against the launch
//!   directory.
//! - Empty or whitespace-only is unset for every kind, with no warning.
//!
//! An ignored value falls back to the default and is logged once per variable
//! per process through [`warn_once`].
//!
//! Precedence is environment variable, then `config.toml`, then the built-in
//! default, for every setting that has both. `GPY_DISABLE_WATCHER` is a kill
//! switch with no config key: `1` turns the file watcher off whatever
//! `agent.live_updates` says, and `0` or unset leaves the decision to config.

use crate::font::{FontCapability, parse_nerd_font};
use crate::sync_ext::MutexExt;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Mutex;

/// Spellings read as `true` by [`parse_flag`], case-insensitive.
const TRUTHY: [&str; 4] = ["1", "true", "yes", "on"];

/// Spellings read as `false` by [`parse_flag`], case-insensitive.
const FALSEY: [&str; 4] = ["0", "false", "no", "off"];

/// Seconds between agent reconcile ticks (`GPY_RECONCILE_INTERVAL_SECS`); above
/// the git-status cache TTL so scans see fresh state.
pub(crate) const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 45;

/// Debounce window for filesystem events, in milliseconds (`GPY_DEBOUNCE_MS`).
pub(crate) const DEFAULT_DEBOUNCE_MS: u64 = 100;

/// Minimum gap between `SIGURG` notifications, in milliseconds
/// (`GPY_SIGURG_THROTTLE_MS`).
pub(crate) const DEFAULT_THROTTLE_MS: u64 = 150;

/// How long a native watcher has to deliver a probe event before the
/// backend falls back to polling, in milliseconds (`GPY_WATCH_PROBE_MS`).
pub(crate) const DEFAULT_WATCH_PROBE_MS: u64 = 2_000;

/// Poll interval of the fallback watcher, in milliseconds (`GPY_WATCH_POLL_MS`).
pub(crate) const DEFAULT_WATCH_POLL_MS: u64 = 2_000;

/// The outcome of parsing one raw value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reading<T> {
    /// Unset, empty or whitespace-only.
    Unset,
    /// A recognised value.
    Value(T),
    /// A value that is not recognised, kept verbatim for the warning.
    Invalid(String),
}

/// A whole number, ignoring surrounding whitespace.
#[must_use]
pub(crate) fn parse_u64(raw: Option<&str>) -> Reading<u64> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Reading::Unset;
    };
    value
        .parse()
        .map_or_else(|_| Reading::Invalid(value.to_owned()), Reading::Value)
}

/// [`parse_u64`] where `0` is also [`Reading::Invalid`].
#[must_use]
pub(crate) fn parse_positive(raw: Option<&str>) -> Reading<u64> {
    match parse_u64(raw) {
        Reading::Value(0) => {
            Reading::Invalid(raw.map_or_else(String::new, |r| r.trim().to_owned()))
        }
        other => other,
    }
}

/// A boolean: a truthy spelling is `true`, a falsey one `false`.
#[must_use]
pub(crate) fn parse_flag(raw: Option<&str>) -> Reading<bool> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Reading::Unset;
    };
    if TRUTHY.iter().any(|word| value.eq_ignore_ascii_case(word)) {
        Reading::Value(true)
    } else if FALSEY.iter().any(|word| value.eq_ignore_ascii_case(word)) {
        Reading::Value(false)
    } else {
        Reading::Invalid(value.to_owned())
    }
}

/// A path; empty (or whitespace-only) is `None`.
#[must_use]
pub(crate) fn parse_path(raw: Option<&OsStr>) -> Option<PathBuf> {
    raw.filter(|value| !value.is_empty() && value.to_str().is_none_or(|s| !s.trim().is_empty()))
        .map(PathBuf::from)
}

/// Free text; empty or whitespace-only is `None`.
fn parse_text(raw: Option<&str>) -> Option<String> {
    raw.filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/// What shape a variable has; the shape holds its default.
#[derive(Debug, Clone, Copy)]
pub(crate) enum EnvKind {
    /// A boolean flag.
    Flag { default: bool },
    /// A positive integer; `0` and garbage give `default`.
    Positive { default: u64 },
    /// An integer where `0` turns the feature off; `None` default is "off".
    ZeroDisables { default: Option<u64> },
    /// A filesystem path.
    Path,
    /// A path read through `paths::root_var_os`, which lib tests stub out.
    ConfigRoot,
    /// `GPY_NERD_FONT`: a flag plus `nerd`, `none`, `ascii`, `unknown`, `auto`.
    NerdFont,
    /// Free text.
    Text,
}

/// One `GPY_*` variable.
#[derive(Debug)]
pub(crate) struct EnvVar {
    /// The variable name.
    pub name: &'static str,
    /// Its shape and default.
    pub kind: EnvKind,
}

/// Where an effective value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A recognised value in the environment.
    Env,
    /// Unset, so the built-in default applies.
    Default,
    /// The environment value was not recognised; the default applies.
    Ignored(String),
}

/// `GPY_PROFILE`: log timed operations to stderr.
pub(crate) const PROFILE: EnvVar = EnvVar {
    name: "GPY_PROFILE",
    kind: EnvKind::Flag { default: false },
};
/// `GPY_DEBUG_LOG`: debug log file.
pub(crate) const DEBUG_LOG: EnvVar = EnvVar {
    name: "GPY_DEBUG_LOG",
    kind: EnvKind::Path,
};
/// `GPY_AGENT_SOCKET_PATH`: agent socket.
pub(crate) const AGENT_SOCKET_PATH: EnvVar = EnvVar {
    name: "GPY_AGENT_SOCKET_PATH",
    kind: EnvKind::Path,
};
/// `GPY_CONFIG_PATH`: config file.
pub(crate) const CONFIG_PATH: EnvVar = EnvVar {
    name: "GPY_CONFIG_PATH",
    kind: EnvKind::ConfigRoot,
};
/// `GPY_DISABLE_WATCHER`: kill switch for the file watcher.
pub(crate) const DISABLE_WATCHER: EnvVar = EnvVar {
    name: "GPY_DISABLE_WATCHER",
    kind: EnvKind::Flag { default: false },
};
/// `GPY_RECONCILE_INTERVAL_SECS`: reconcile tick.
pub(crate) const RECONCILE_INTERVAL_SECS: EnvVar = EnvVar {
    name: "GPY_RECONCILE_INTERVAL_SECS",
    kind: EnvKind::Positive {
        default: DEFAULT_RECONCILE_INTERVAL_SECS,
    },
};
/// `GPY_DEBOUNCE_MS`: filesystem event debounce.
pub(crate) const DEBOUNCE_MS: EnvVar = EnvVar {
    name: "GPY_DEBOUNCE_MS",
    kind: EnvKind::Positive {
        default: DEFAULT_DEBOUNCE_MS,
    },
};
/// `GPY_SIGURG_THROTTLE_MS`: gap between `SIGURG` notifications.
pub(crate) const SIGURG_THROTTLE_MS: EnvVar = EnvVar {
    name: "GPY_SIGURG_THROTTLE_MS",
    kind: EnvKind::Positive {
        default: DEFAULT_THROTTLE_MS,
    },
};
/// `GPY_WATCH_FORCE_POLL`: always poll instead of using native events.
pub(crate) const WATCH_FORCE_POLL: EnvVar = EnvVar {
    name: "GPY_WATCH_FORCE_POLL",
    kind: EnvKind::Flag { default: false },
};
/// `GPY_WATCH_PROBE_MS`: native watcher probe timeout; `0` disables the probe.
pub(crate) const WATCH_PROBE_MS: EnvVar = EnvVar {
    name: "GPY_WATCH_PROBE_MS",
    kind: EnvKind::ZeroDisables {
        default: Some(DEFAULT_WATCH_PROBE_MS),
    },
};
/// `GPY_WATCH_POLL_MS`: fallback poll interval.
pub(crate) const WATCH_POLL_MS: EnvVar = EnvVar {
    name: "GPY_WATCH_POLL_MS",
    kind: EnvKind::Positive {
        default: DEFAULT_WATCH_POLL_MS,
    },
};
/// `GPY_WATCH_WORKTREE`: overrides `git.watch_worktree` (default on).
pub(crate) const WATCH_WORKTREE: EnvVar = EnvVar {
    name: "GPY_WATCH_WORKTREE",
    kind: EnvKind::Flag { default: true },
};
/// `GPY_BUNDLED_PLUGIN_DIR`: bundled plugin directory.
pub(crate) const BUNDLED_PLUGIN_DIR: EnvVar = EnvVar {
    name: "GPY_BUNDLED_PLUGIN_DIR",
    kind: EnvKind::Path,
};
/// `GPY_CONFIG_WATCH_POLL_MS`: config poll fallback; `0` disables it.
pub(crate) const CONFIG_WATCH_POLL_MS: EnvVar = EnvVar {
    name: "GPY_CONFIG_WATCH_POLL_MS",
    kind: EnvKind::ZeroDisables { default: None },
};
/// `GPY_THEME_WATCH_POLL_MS`: theme poll fallback; `0` disables it.
pub(crate) const THEME_WATCH_POLL_MS: EnvVar = EnvVar {
    name: "GPY_THEME_WATCH_POLL_MS",
    kind: EnvKind::ZeroDisables { default: None },
};
/// `GPY_NERD_FONT`: overrides Nerd Font detection.
pub(crate) const NERD_FONT: EnvVar = EnvVar {
    name: "GPY_NERD_FONT",
    kind: EnvKind::NerdFont,
};
/// `GPY_SHELL`: the shell `gpy setup` targets.
pub(crate) const SHELL: EnvVar = EnvVar {
    name: "GPY_SHELL",
    kind: EnvKind::Text,
};
/// `GPY_SHARE_DIR`: where `gpy setup` finds the shipped shell integration.
pub(crate) const SHARE_DIR: EnvVar = EnvVar {
    name: "GPY_SHARE_DIR",
    kind: EnvKind::Path,
};

/// Every `GPY_*` variable the Rust code reads; `gpy doctor` prints them all.
/// Test-only `GPY_UPDATE_SCHEMAS` and `GPY_CI_NO_SHELLS` are not listed.
pub(crate) const AGENT_ENV_VARS: &[&EnvVar] = &[
    &PROFILE,
    &DEBUG_LOG,
    &AGENT_SOCKET_PATH,
    &CONFIG_PATH,
    &DISABLE_WATCHER,
    &RECONCILE_INTERVAL_SECS,
    &DEBOUNCE_MS,
    &SIGURG_THROTTLE_MS,
    &WATCH_FORCE_POLL,
    &WATCH_PROBE_MS,
    &WATCH_POLL_MS,
    &WATCH_WORKTREE,
    &BUNDLED_PLUGIN_DIR,
    &CONFIG_WATCH_POLL_MS,
    &THEME_WATCH_POLL_MS,
    &NERD_FONT,
    &SHELL,
    &SHARE_DIR,
];

/// Names that have already warned in this process.
static WARNED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Emit `message` the first time `name` warns in this process.
///
/// With `to_debug_log` it goes through [`crate::debug::warn`] (stderr and
/// `GPY_DEBUG_LOG`); without, to stderr only, which `debug.rs` needs because
/// calling `debug::warn` while its path initialises would re-enter it. The
/// lock is released before emitting: `debug::warn` reads `GPY_DEBUG_LOG`
/// through this module and may call `warn_once` again.
pub(crate) fn warn_once(name: &'static str, message: &str, to_debug_log: bool) {
    let first = {
        let mut warned = WARNED.lock_or_recover();
        if warned.contains(&name) {
            false
        } else {
            warned.push(name);
            true
        }
    };
    if !first {
        return;
    }
    if to_debug_log {
        crate::debug::warn("env", message);
    } else {
        use std::io::Write as _;
        // Not `eprintln!`: `clippy::print_stderr` is denied.
        let _ = writeln!(std::io::stderr(), "Warning: [env] {message}");
    }
}

/// Warn once that `name` was set to `raw`, which is not `expected`.
fn warn_ignored(name: &'static str, raw: &str, expected: &str) {
    warn_once(
        name,
        &format!("ignoring {name}=\"{raw}\": expected {expected}"),
        true,
    );
}

const EXPECTED_FLAG: &str = "1, true, yes, on, 0, false, no or off";
const EXPECTED_POSITIVE: &str = "a positive whole number";
const EXPECTED_NUMBER: &str = "a whole number (0 disables)";
const EXPECTED_NERD_FONT: &str = "a flag value, nerd, none, ascii, unknown or auto";

impl EnvVar {
    /// The unparsed value, or `None` when unset.
    #[must_use]
    pub(crate) fn raw(&self) -> Option<OsString> {
        if matches!(self.kind, EnvKind::ConfigRoot) {
            crate::paths::root_var_os(self.name)
        } else {
            std::env::var_os(self.name)
        }
    }

    /// The unparsed value as text, non-UTF-8 bytes replaced.
    fn raw_text(&self) -> Option<String> {
        self.raw().map(|value| value.to_string_lossy().into_owned())
    }

    fn flag_reading(&self) -> Reading<bool> {
        let reading = parse_flag(self.raw_text().as_deref());
        if let Reading::Invalid(raw) = &reading {
            warn_ignored(self.name, raw, EXPECTED_FLAG);
        }
        reading
    }

    /// The flag's value; the default when unset or invalid.
    #[must_use]
    pub(crate) fn flag(&self) -> bool {
        let default = matches!(self.kind, EnvKind::Flag { default: true });
        match self.flag_reading() {
            Reading::Value(value) => value,
            Reading::Unset | Reading::Invalid(_) => default,
        }
    }

    /// The flag's value, `None` when unset or invalid.
    #[must_use]
    pub(crate) fn flag_override(&self) -> Option<bool> {
        match self.flag_reading() {
            Reading::Value(value) => Some(value),
            Reading::Unset | Reading::Invalid(_) => None,
        }
    }

    /// The positive integer; the default when unset, `0` or invalid.
    #[must_use]
    pub(crate) fn positive(&self) -> u64 {
        let default = if let EnvKind::Positive { default } = self.kind {
            default
        } else {
            0
        };
        match parse_positive(self.raw_text().as_deref()) {
            Reading::Value(value) => value,
            Reading::Unset => default,
            Reading::Invalid(raw) => {
                warn_ignored(self.name, &raw, EXPECTED_POSITIVE);
                default
            }
        }
    }

    /// The integer where `0` disables: `None` is off; unset or invalid gives
    /// the default.
    #[must_use]
    pub(crate) fn zero_disables(&self) -> Option<u64> {
        let default = if let EnvKind::ZeroDisables { default } = self.kind {
            default
        } else {
            None
        };
        match parse_u64(self.raw_text().as_deref()) {
            Reading::Value(0) => None,
            Reading::Value(value) => Some(value),
            Reading::Unset => default,
            Reading::Invalid(raw) => {
                warn_ignored(self.name, &raw, EXPECTED_NUMBER);
                default
            }
        }
    }

    /// The path; `None` when unset or empty. Read as an `OsString`, so a
    /// non-UTF-8 path survives.
    #[must_use]
    pub(crate) fn path(&self) -> Option<PathBuf> {
        parse_path(self.raw().as_deref())
    }

    /// The text; `None` when unset, empty or not valid Unicode.
    #[must_use]
    pub(crate) fn text(&self) -> Option<String> {
        parse_text(
            self.raw()
                .and_then(|value| value.into_string().ok())
                .as_deref(),
        )
    }

    /// The `GPY_NERD_FONT` override; `None` means detect. An unrecognised
    /// value warns once.
    #[must_use]
    pub(crate) fn nerd_font(&self) -> Option<FontCapability> {
        match parse_nerd_font(self.raw_text().as_deref()) {
            Reading::Value(capability) => capability,
            Reading::Unset => None,
            Reading::Invalid(raw) => {
                warn_ignored(self.name, &raw, EXPECTED_NERD_FONT);
                None
            }
        }
    }

    /// The value in effect and where it came from, for `gpy doctor`. Pure
    /// parse: it never warns, because `doctor` prints the ignored value itself.
    #[must_use]
    pub(crate) fn effective(&self) -> (String, Source) {
        let raw = self.raw_text();
        let unit = if self.name.ends_with("_SECS") {
            " s"
        } else {
            " ms"
        };
        let number = |value: u64| format!("{value}{unit}");
        let on_off = |value: bool| if value { "on" } else { "off" }.to_owned();
        match self.kind {
            EnvKind::Flag { default } => match parse_flag(raw.as_deref()) {
                Reading::Value(value) => (on_off(value), Source::Env),
                Reading::Unset => (on_off(default), Source::Default),
                Reading::Invalid(bad) => (on_off(default), Source::Ignored(bad)),
            },
            EnvKind::Positive { default } => match parse_positive(raw.as_deref()) {
                Reading::Value(value) => (number(value), Source::Env),
                Reading::Unset => (number(default), Source::Default),
                Reading::Invalid(bad) => (number(default), Source::Ignored(bad)),
            },
            EnvKind::ZeroDisables { default } => {
                let shown = |value: Option<u64>| value.map_or_else(|| "off".to_owned(), number);
                match parse_u64(raw.as_deref()) {
                    Reading::Value(0) => ("off".to_owned(), Source::Env),
                    Reading::Value(value) => (number(value), Source::Env),
                    Reading::Unset => (shown(default), Source::Default),
                    Reading::Invalid(bad) => (shown(default), Source::Ignored(bad)),
                }
            }
            EnvKind::Path | EnvKind::ConfigRoot => self.path().map_or_else(
                || ("unset".to_owned(), Source::Default),
                |path| (path.display().to_string(), Source::Env),
            ),
            EnvKind::Text => parse_text(raw.as_deref()).map_or_else(
                || ("unset".to_owned(), Source::Default),
                |text| (text, Source::Env),
            ),
            EnvKind::NerdFont => match parse_nerd_font(raw.as_deref()) {
                Reading::Value(Some(FontCapability::NerdFont)) => ("nerd".to_owned(), Source::Env),
                Reading::Value(Some(FontCapability::NoNerdFont)) => {
                    ("none".to_owned(), Source::Env)
                }
                Reading::Value(Some(FontCapability::Unknown)) => {
                    ("unknown".to_owned(), Source::Env)
                }
                Reading::Value(None) => ("auto".to_owned(), Source::Env),
                Reading::Unset => ("auto".to_owned(), Source::Default),
                Reading::Invalid(bad) => ("auto".to_owned(), Source::Ignored(bad)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_ENV_VARS, EnvKind, Reading, WARNED, parse_flag, parse_path, parse_positive,
        parse_u64, warn_once,
    };
    use crate::sync_ext::MutexExt as _;
    use std::ffi::OsStr;

    #[test]
    fn parse_flag_reads_known_spellings_case_insensitively() {
        for word in ["1", "true", "YES", " On ", "TrUe"] {
            assert_eq!(parse_flag(Some(word)), Reading::Value(true), "{word}");
        }
        for word in ["0", "False", "no", "OFF", "\toff\n"] {
            assert_eq!(parse_flag(Some(word)), Reading::Value(false), "{word}");
        }
    }

    #[test]
    fn parse_flag_is_unset_for_empty_and_invalid_for_garbage() {
        assert_eq!(parse_flag(None), Reading::Unset);
        assert_eq!(parse_flag(Some("")), Reading::Unset);
        assert_eq!(parse_flag(Some("   ")), Reading::Unset);
        assert_eq!(parse_flag(Some("maybe")), Reading::Invalid("maybe".into()));
    }

    #[test]
    fn parse_positive_rejects_zero_and_garbage() {
        assert_eq!(parse_positive(None), Reading::Unset);
        assert_eq!(parse_positive(Some("")), Reading::Unset);
        assert_eq!(parse_positive(Some("7")), Reading::Value(7));
        assert_eq!(parse_positive(Some("0")), Reading::Invalid("0".into()));
        assert_eq!(
            parse_positive(Some("nope")),
            Reading::Invalid("nope".into())
        );
        assert_eq!(parse_positive(Some("-1")), Reading::Invalid("-1".into()));
    }

    #[test]
    fn parse_u64_keeps_zero() {
        assert_eq!(parse_u64(None), Reading::Unset);
        assert_eq!(parse_u64(Some("")), Reading::Unset);
        assert_eq!(parse_u64(Some("0")), Reading::Value(0));
        assert_eq!(parse_u64(Some(" 25 ")), Reading::Value(25));
    }

    #[test]
    fn zero_disables_and_positive_parse() {
        assert_eq!(parse_u64(Some("0")), Reading::Value(0));
        assert_eq!(parse_u64(Some(" 500 ")), Reading::Value(500));
        assert_eq!(parse_u64(Some("abc")), Reading::Invalid("abc".into()));
        assert!(matches!(parse_positive(Some("0")), Reading::Invalid(_)));
        assert!(matches!(parse_positive(Some("abc")), Reading::Invalid(_)));
    }

    #[test]
    fn debug_log_path_is_unset_when_empty() {
        assert_eq!(parse_path(Some(OsStr::new(""))), None);
        assert_eq!(parse_path(None), None);
        assert_eq!(
            parse_path(Some(OsStr::new("/tmp/x.log"))),
            Some("/tmp/x.log".into())
        );
    }

    #[test]
    fn watch_worktree_override_ignores_unrecognised_values() {
        assert_eq!(parse_flag(Some("maybe")), Reading::Invalid("maybe".into()));
        assert_eq!(parse_flag(Some("off")), Reading::Value(false));
    }

    #[test]
    fn profile_is_off_for_anything_but_a_true_spelling() {
        assert_eq!(parse_flag(Some("0")), Reading::Value(false));
        assert_eq!(parse_flag(Some("")), Reading::Unset);
        assert_eq!(parse_flag(Some("debug")), Reading::Invalid("debug".into()));
        assert_eq!(parse_flag(Some("1")), Reading::Value(true));
    }

    #[test]
    fn warn_once_emits_each_name_once() {
        warn_once("GPY_TEST_WARN_ONCE_A", "first", false);
        warn_once("GPY_TEST_WARN_ONCE_A", "second", false);
        warn_once("GPY_TEST_WARN_ONCE_B", "only", false);
        let warned = WARNED.lock_or_recover();
        for name in ["GPY_TEST_WARN_ONCE_A", "GPY_TEST_WARN_ONCE_B"] {
            assert_eq!(
                warned.iter().filter(|seen| **seen == name).count(),
                1,
                "{name}"
            );
        }
    }

    #[test]
    fn registry_names_are_unique_gpy_variables() {
        let mut names: Vec<_> = AGENT_ENV_VARS.iter().map(|var| var.name).collect();
        assert!(names.iter().all(|name| name.starts_with("GPY_")));
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
        assert!(
            AGENT_ENV_VARS
                .iter()
                .any(|var| matches!(var.kind, EnvKind::ConfigRoot))
        );
    }

    /// Every row of the fixture the three shells also read, so the tables cannot drift.
    #[test]
    fn shell_env_flag_shared_vectors() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/shell_env_flag_vectors.tsv");
        let text = std::fs::read_to_string(&path).expect("read shared flag vectors");
        let mut rows = 0_u32;
        for line in text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (raw, state) = line.split_once('\t').expect("value<TAB>state row");
            let value = if raw == "\\e" { "" } else { raw };
            let reading = parse_flag(Some(value));
            match state {
                "on" => assert_eq!(reading, Reading::Value(true), "{raw:?} must be on"),
                "off" => assert_eq!(reading, Reading::Value(false), "{raw:?} must be off"),
                "unset" => assert!(
                    matches!(reading, Reading::Unset | Reading::Invalid(_)),
                    "{raw:?} must be unset"
                ),
                other => panic!("unknown state {other:?}"),
            }
            rows += 1;
        }
        assert!(rows > 0, "no vectors read from {}", path.display());
    }
}
