//! Static checks of a script segment's `sh` files for `gpy plugin validate`.
//!
//! The script is linted with `shellcheck -s sh`, else `dash -n`. `sh -n` is
//! deliberately not used: on macOS `/bin/sh` is Bash 3.2 in POSIX mode and
//! accepts bashisms that a real `sh` (the Debian/Ubuntu `dash`, BusyBox) rejects.
//! The linters only read the file; they never run it.

use super::utils::find_in_path;
use crate::plugin::{InputName, ScriptSegmentManifest};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// Result of linting one script.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Lint {
    /// The tool accepted the script.
    Clean(&'static str),
    /// The tool rejected the script; its report follows.
    Failed { tool: &'static str, report: String },
    /// Neither `shellcheck` nor `dash` is on `PATH`.
    Unavailable,
}

/// Lint the script at `script`.
pub(crate) fn lint_script(script: &Path) -> Lint {
    for (tool, flags) in [("shellcheck", &["-s", "sh"][..]), ("dash", &["-n"][..])] {
        let Some(program) = find_in_path(tool) else {
            continue;
        };
        let result = Command::new(program)
            .args(flags)
            .arg(script)
            .stdin(Stdio::null())
            .output();
        return match result {
            Ok(output) if output.status.success() => Lint::Clean(tool),
            Ok(output) => {
                let mut report = String::from_utf8_lossy(&output.stdout).into_owned();
                report.push_str(&String::from_utf8_lossy(&output.stderr));
                Lint::Failed {
                    tool,
                    report: report.trim_end().to_owned(),
                }
            }
            Err(error) => Lint::Failed {
                tool,
                report: format!("could not run {tool}: {error}"),
            },
        };
    }
    Lint::Unavailable
}

/// Names the runner provides or the shell itself defines, so a script may use them undeclared.
const PROVIDED: [&str; 17] = [
    "PATH", "HOME", "USER", "LOGNAME", "LANG", "TERM", "TZ", "TMPDIR", "PWD", "OLDPWD", "SHLVL",
    "IFS", "OPTIND", "OPTARG", "LINENO", "PPID", "_",
];

/// A `$NAME` use the manifest does not provide.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Undeclared {
    pub(crate) name: String,
    pub(crate) fix: String,
}

const fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn name_at(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|(index, c)| {
            !(c.is_ascii_alphanumeric() || *c == '_') || (*index == 0 && !is_name_start(*c))
        })
        .map_or(text.len(), |(index, _)| index);
    text.get(..end).unwrap_or_default()
}

/// Variable names the script reads (`$NAME`, `${NAME...}`), outside comments
/// and single quotes.
fn used_names(script: &str) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut prev_space = true;
    let mut chars = script.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let space_before = prev_space;
        prev_space = c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')');
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && space_before => {
                for (_, rest) in chars.by_ref() {
                    if rest == '\n' {
                        break;
                    }
                }
                prev_space = true;
            }
            '$' if !in_single => {
                let tail = script.get(index.saturating_add(1)..).unwrap_or_default();
                let after = tail.strip_prefix('{').unwrap_or(tail);
                let name = name_at(after);
                if !name.is_empty() {
                    used.insert(name.to_owned());
                }
            }
            _ => {}
        }
    }
    used
}

/// Names the script assigns itself: `NAME=`, `for NAME in`, `read NAME`,
/// `getopts spec NAME`.
fn assigned_names(script: &str) -> BTreeSet<String> {
    let mut assigned = BTreeSet::new();
    for line in script.lines() {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')'))
            .filter(|word| !word.is_empty())
            .collect();
        for (index, word) in words.iter().enumerate() {
            let name = name_at(word);
            if !name.is_empty() && word.get(name.len()..).is_some_and(|r| r.starts_with('=')) {
                assigned.insert(name.to_owned());
            }
            let argument = |offset: usize| words.get(index.saturating_add(offset)).copied();
            match *word {
                "for" => assigned.extend(argument(1).map(name_at).map(str::to_owned)),
                "getopts" => assigned.extend(argument(2).map(name_at).map(str::to_owned)),
                "read" | "local" => {
                    assigned.extend(
                        words
                            .iter()
                            .skip(index.saturating_add(1))
                            .filter(|w| !w.starts_with('-'))
                            .map(|w| name_at(w).to_owned()),
                    );
                }
                _ => {}
            }
        }
    }
    assigned.remove("");
    assigned
}

/// `$VAR` uses in `script` that `manifest` does not declare or the runner
/// does not provide.
///
/// This is a heuristic scan (quoting is tracked, `eval` and indirection are
/// not), so the findings are warnings.
pub(crate) fn undeclared_vars(script: &str, manifest: &ScriptSegmentManifest) -> Vec<Undeclared> {
    let assigned = assigned_names(script);
    let mut found: Vec<Undeclared> = used_names(script)
        .into_iter()
        .filter(|name| {
            !(PROVIDED.contains(&name.as_str())
                || name.starts_with("LC_")
                || assigned.contains(name)
                || manifest
                    .env
                    .iter()
                    .any(|declared| declared.as_str() == name)
                || manifest.inputs.iter().any(|input| input.env_var() == name)
                || manifest.settings.keys().any(|key| &key.env_var() == name))
        })
        .map(|name| {
            let fix = InputName::ALL
                .iter()
                .find(|input| input.env_var() == name)
                .map_or_else(
                    || {
                        if name.starts_with("GPY_OPT_") {
                            "declare it under `[settings.<key>]`".to_owned()
                        } else if name.starts_with("GPY_") {
                            "it is not a variable the runner sets".to_owned()
                        } else {
                            format!("add \"{name}\" to `env` if the user's value is meant")
                        }
                    },
                    |input| format!("add \"{}\" to `inputs`", input.as_str()),
                );
            Undeclared { name, fix }
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(extra: &str) -> ScriptSegmentManifest {
        ScriptSegmentManifest::parse(&format!("command = \"a.sh\"\n{extra}")).expect("manifest")
    }

    fn names(script: &str, extra: &str) -> Vec<String> {
        undeclared_vars(script, &manifest(extra))
            .into_iter()
            .map(|u| u.name)
            .collect()
    }

    #[test]
    fn flags_only_undeclared_uses() {
        let script = "#!/bin/sh\n# $COMMENTED is ignored\n\
            x=1\nfor f in a; do echo \"$f $x ${KUBECONFIG:-} $HOME $GPY_PWD\"; echo '$SINGLE'; done\n\
            echo \"$GPY_IS_SSH $OTHER $GPY_OPT_style $1 $?\"\n";
        let found = names(
            script,
            "inputs = [\"pwd\"]\nenv = [\"KUBECONFIG\"]\n[settings.style]\ntype = \"string\"\ndefault = \"a\"\n",
        );
        assert_eq!(found, ["GPY_IS_SSH", "OTHER"]);
    }

    #[test]
    fn explains_the_fix_for_an_undeclared_input() {
        let found = undeclared_vars("echo \"$GPY_PWD\"\n", &manifest(""));
        assert_eq!(found.len(), 1);
        let first = found.first().expect("one finding");
        assert!(first.fix.contains("\"pwd\""), "{found:?}");
    }
}
