//! One timed trial run of a script segment, for `gpy doctor`.
//!
//! Applies the runner's process rules from `docs/dev/plugins.md` ("Environment
//! and process rules"): `/bin/sh <script>` in a clean environment, stdin on
//! `/dev/null`, stderr discarded, its own process group, the segment's
//! `timeout_ms` (TERM, CONT, then KILL after a 50 ms grace) and the output
//! protocol. It does not evaluate the `require_*` triggers and does not cache.

use crate::config::plugin_segments::ResolvedScriptSegment;
use crate::plugin::{EnvName, InputName, ScriptSegmentManifest, VariantName};
use crate::{Error, Result};
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Output longer than this hides the segment.
pub(crate) const OUTPUT_CAP: usize = 4096;

/// Time between TERM and KILL.
const GRACE: Duration = Duration::from_millis(50);

/// Poll interval while the script runs.
const POLL: Duration = Duration::from_millis(1);

/// Variables copied from the invoking environment when set.
const BASE_ENV: [&str; 6] = ["HOME", "USER", "LOGNAME", "LANG", "TERM", "TZ"];

/// What the shell would have shown for one run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The segment would render `text`, styled by `variant` when given.
    Shown {
        text: String,
        variant: Option<String>,
    },
    /// The segment would be hidden, for the reason given.
    Hidden(String),
    /// The script outlived `timeout_ms` and was killed.
    TimedOut,
}

/// A run and how long it took.
#[derive(Debug)]
pub(crate) struct Trial {
    pub(crate) elapsed: Duration,
    pub(crate) outcome: Outcome,
}

/// `PATH` without empty, `.` or other relative entries.
fn sanitized_path(path: &std::ffi::OsStr) -> OsString {
    let kept: Vec<_> = std::env::split_paths(path)
        .filter(|entry| entry.is_absolute())
        .collect();
    std::env::join_paths(kept).unwrap_or_default()
}

/// The clean environment for a script: the base variables set in `parent`,
/// every set `LC_*`, the declared `env` names and the `GPY_*` values.
fn clean_env(
    parent: impl Iterator<Item = (OsString, OsString)>,
    declared: &[EnvName],
    runner: Vec<(String, String)>,
) -> Vec<(OsString, OsString)> {
    let mut vars: Vec<(OsString, OsString)> = Vec::new();
    for (key, value) in parent {
        let Some(name) = key.to_str() else { continue };
        if name == "PATH" {
            vars.push((key, sanitized_path(&value)));
        } else if BASE_ENV.contains(&name)
            || name == "TMPDIR"
            || name.starts_with("LC_")
            || declared.iter().any(|d| d.as_str() == name)
        {
            vars.push((key, value));
        }
    }
    vars.extend(
        runner
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    vars
}

#[cfg(unix)]
fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}

/// What `doctor` can say about the shell state an input stands for: the
/// working directory is the current one, there is no previous command.
fn runner_vars(
    manifest: &ScriptSegmentManifest,
    resolved: &ResolvedScriptSegment,
    cwd: &Path,
    show_icons: bool,
) -> Vec<(String, String)> {
    let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
    let mut vars: Vec<(String, String)> = manifest
        .inputs
        .iter()
        .map(|input| {
            let value = match input {
                InputName::Pwd => cwd.to_string_lossy().into_owned(),
                InputName::IsRoot => flag(is_root()),
                InputName::IsSudo => flag(std::env::var_os("SUDO_USER").is_some()),
                InputName::IsSsh => flag(
                    std::env::var_os("SSH_CONNECTION").is_some()
                        || std::env::var_os("SSH_TTY").is_some(),
                ),
                InputName::CmdDuration => String::new(),
                InputName::PromptIcons => if show_icons { "nerd" } else { "ascii" }.to_owned(),
            };
            (input.env_var().to_owned(), value)
        })
        .collect();
    vars.extend(
        resolved
            .settings
            .iter()
            .map(|(key, value)| (key.env_var(), value.to_string())),
    );
    vars
}

#[cfg(unix)]
fn signal_group(child: &Child, signal: nix::sys::signal::Signal) {
    let Ok(pid) = i32::try_from(child.id()) else {
        return;
    };
    // ESRCH: the group is already gone, which is the goal.
    let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), signal);
}

#[cfg(not(unix))]
fn signal_group(_child: &Child, _signal: ()) {}

/// TERM and CONT to the script's process group.
#[cfg(unix)]
fn terminate_group(child: &Child) {
    signal_group(child, nix::sys::signal::Signal::SIGTERM);
    signal_group(child, nix::sys::signal::Signal::SIGCONT);
}

#[cfg(not(unix))]
fn terminate_group(child: &Child) {
    signal_group(child, ());
}

#[cfg(unix)]
fn kill_group(child: &Child) {
    signal_group(child, nix::sys::signal::Signal::SIGKILL);
}

#[cfg(not(unix))]
fn kill_group(child: &Child) {
    signal_group(child, ());
}

/// Wait for `child` until `deadline`; `None` when it is still running then.
fn wait_until(child: &mut Child, deadline: Instant) -> Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait().map_err(Error::from)? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(POLL);
    }
}

/// Apply the output protocol to the bytes a script wrote.
pub(crate) fn classify(bytes: &[u8], exit_ok: bool) -> Outcome {
    if !exit_ok {
        return Outcome::Hidden("non-zero exit".to_owned());
    }
    if bytes.len() > OUTPUT_CAP {
        return Outcome::Hidden(format!("output is over {OUTPUT_CAP} bytes"));
    }
    // Output after the first NUL is undefined: ignore it.
    let visible = bytes.split(|b| *b == 0).next().unwrap_or_default();
    let text = String::from_utf8_lossy(visible);
    if text.chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c)) {
        return Outcome::Hidden("output holds a C1 control character".to_owned());
    }
    let strip = |line: &str| -> String {
        line.chars()
            .filter(|c| !c.is_ascii_control())
            .collect::<String>()
    };
    let mut lines = text.split('\n');
    let first = strip(lines.next().unwrap_or_default());
    if first.is_empty() {
        return Outcome::Hidden("empty output".to_owned());
    }
    let variant = lines
        .next()
        .map(strip)
        .and_then(|name| VariantName::new(&name).ok())
        .map(|name| name.as_str().to_owned());
    Outcome::Shown {
        text: first,
        variant,
    }
}

/// Run `script` once the way the shell runner would.
///
/// `root` is the plugin root (the working directory unless `pwd` is declared,
/// when it is `cwd`).
///
/// # Errors
/// Returns an error if the script cannot be started or waited on.
pub(crate) fn run_trial(
    script: &Path,
    root: &Path,
    manifest: &ScriptSegmentManifest,
    resolved: &ResolvedScriptSegment,
    cwd: &Path,
    show_icons: bool,
) -> Result<Trial> {
    let dir = if manifest.inputs.contains(&InputName::Pwd) {
        cwd
    } else {
        root
    };
    let vars = clean_env(
        std::env::vars_os(),
        &manifest.env,
        runner_vars(manifest, resolved, cwd, show_icons),
    );
    let mut command = Command::new("/bin/sh");
    command
        .arg(script)
        .env_clear()
        .envs(vars)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = command.spawn().map_err(Error::from)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::invalid("script stdout was not captured"))?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let limit = u64::try_from(OUTPUT_CAP)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let _ = (&mut stdout).take(limit).read_to_end(&mut captured);
        let _ = sender.send(captured);
        // Keep draining so a chatty script ends by the timeout, not on a full pipe.
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
    });

    let timeout = Duration::from_millis(u64::from(resolved.timeout_ms.get()));
    let finished = wait_until(&mut child, started.checked_add(timeout).unwrap_or(started))?;
    let elapsed = started.elapsed();
    terminate_group(&child);
    let Some(status) = finished else {
        if wait_until(
            &mut child,
            Instant::now()
                .checked_add(GRACE)
                .unwrap_or_else(Instant::now),
        )?
        .is_none()
        {
            kill_group(&child);
            child.wait().map_err(Error::from)?;
        }
        kill_group(&child);
        return Ok(Trial {
            elapsed,
            outcome: Outcome::TimedOut,
        });
    };
    // Leftovers of a normal exit got TERM above; give the group the grace, then KILL.
    let bytes = receiver.recv_timeout(GRACE).unwrap_or_default();
    kill_group(&child);
    Ok(Trial {
        elapsed,
        outcome: classify(&bytes, status.success()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_applies_the_output_protocol() {
        assert_eq!(
            classify(b"text\nwarn\n", true),
            Outcome::Shown {
                text: "text".into(),
                variant: Some("warn".into())
            }
        );
        assert_eq!(
            classify(b"a\x1b[31mb\nBad Variant\n", true),
            Outcome::Shown {
                text: "a[31mb".into(),
                variant: None
            }
        );
        for (bytes, ok) in [
            (&b""[..], true),
            (b"x", false),
            ("a\u{9b}b".as_bytes(), true),
        ] {
            assert!(
                matches!(classify(bytes, ok), Outcome::Hidden(_)),
                "{bytes:?}"
            );
        }
        assert!(matches!(
            classify(&vec![b'a'; OUTPUT_CAP + 1], true),
            Outcome::Hidden(_)
        ));
        assert_eq!(
            classify(b"a\0junk", true),
            Outcome::Shown {
                text: "a".into(),
                variant: None
            }
        );
    }

    // Unix-only: the fixture PATH uses `:` separators and `/`-rooted entries,
    // which `split_paths`/`is_absolute` treat differently on Windows.
    #[cfg(unix)]
    #[test]
    fn clean_env_keeps_only_documented_names() {
        let parent = [
            ("PATH", "/usr/bin::.:rel:/bin"),
            ("HOME", "/h"),
            ("LC_ALL", "C"),
            ("SECRET_TOKEN", "x"),
            ("KUBECONFIG", "/k"),
            ("GPY_OPT_style", "user"),
        ]
        .into_iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let declared = [EnvName::new("KUBECONFIG").expect("name")];
        let vars = clean_env(parent, &declared, vec![("GPY_IS_SSH".into(), "0".into())]);
        let find = |name: &str| {
            vars.iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert_eq!(find("PATH").as_deref(), Some("/usr/bin:/bin"));
        assert_eq!(find("HOME").as_deref(), Some("/h"));
        assert_eq!(find("LC_ALL").as_deref(), Some("C"));
        assert_eq!(find("KUBECONFIG").as_deref(), Some("/k"));
        assert_eq!(find("GPY_IS_SSH").as_deref(), Some("0"));
        assert_eq!(find("SECRET_TOKEN"), None);
        assert_eq!(find("GPY_OPT_style"), None);
    }
}
