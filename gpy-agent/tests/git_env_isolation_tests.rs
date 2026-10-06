//! GIT-ENV class test (#714).
//!
//! Invariant: the agent's git status for repo B, computed with a hostile
//! inherited git environment, equals the status computed with a clean
//! environment, and the agent never writes to any repository config it was
//! not asked to. The hostile environment is passed only to the `gpy-agent`
//! child process, never to the test process or to the fixture `git` calls.
//!
//! To add a case (e.g. #715's `core.untrackedCache` overwrite), append a row
//! to [`hostile_rows`] and, if it needs a new observable, extend
//! [`Snapshot`].

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::CliTestEnv;

/// One hostile inherited-environment row: a label and the variables to set.
type Row = (&'static str, Vec<(&'static str, String)>);

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=T"])
        .args(["-c", "user.email=t@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Repo A on `featureA` (one extra tracked file), repo B on `main` with a
/// modified tracked file and one untracked file. Returns `(a, b)`.
fn make_repos(root: &Path) -> (PathBuf, PathBuf) {
    let repo_a = root.join("repoA");
    let repo_b = root.join("repoB");
    for (repo, branch) in [(&repo_a, "featureA"), (&repo_b, "main")] {
        fs::create_dir_all(repo).expect("mkdir");
        git(repo, &["init", "-q", "-b", branch]);
        fs::write(repo.join("f"), "x\n").expect("write");
        if repo == &repo_a {
            // Makes A's index differ from B's, so GIT_INDEX_FILE is visible.
            fs::write(repo.join("only_a"), "a\n").expect("write");
        }
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "init"]);
    }
    fs::write(repo_b.join("f"), "changed\n").expect("write");
    fs::write(repo_b.join("untracked.txt"), "z\n").expect("write");
    (repo_a, repo_b)
}

/// Rows mirror `git rev-parse --local-env-vars` plus `GIT_NAMESPACE`.
fn hostile_rows(repo_a: &Path) -> Vec<Row> {
    let a_git = repo_a.join(".git");
    let to_str = |p: &Path| p.to_string_lossy().into_owned();
    // A hostile excludes file that would hide B's untracked file.
    let excludes_file = repo_a.join("hostile_excludes");
    fs::write(&excludes_file, "untracked.txt\n").expect("write excludes");
    let excludes = to_str(&excludes_file);
    vec![
        ("GIT_DIR", vec![("GIT_DIR", to_str(&a_git))]),
        (
            "GIT_DIR+GIT_WORK_TREE",
            vec![
                ("GIT_DIR", to_str(&a_git)),
                ("GIT_WORK_TREE", to_str(repo_a)),
            ],
        ),
        (
            "GIT_INDEX_FILE",
            vec![("GIT_INDEX_FILE", to_str(&a_git.join("index")))],
        ),
        (
            "GIT_CONFIG_PARAMETERS",
            vec![(
                "GIT_CONFIG_PARAMETERS",
                format!("'core.excludesFile'='{excludes}'"),
            )],
        ),
        (
            "GIT_CONFIG_COUNT",
            vec![
                ("GIT_CONFIG_COUNT", "1".to_owned()),
                ("GIT_CONFIG_KEY_0", "core.excludesFile".to_owned()),
                ("GIT_CONFIG_VALUE_0", excludes),
            ],
        ),
        (
            "GIT_OBJECT_DIRECTORY",
            vec![("GIT_OBJECT_DIRECTORY", to_str(&a_git.join("objects")))],
        ),
        ("GIT_NAMESPACE", vec![("GIT_NAMESPACE", "other".to_owned())]),
        ("GIT_COMMON_DIR", vec![("GIT_COMMON_DIR", to_str(&a_git))]),
    ]
}

/// Everything the agent must not change unasked: repo configs and repo A's
/// index.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    a_config: Vec<u8>,
    a_index: Vec<u8>,
    b_config: Vec<u8>,
}

fn snapshot(repo_a: &Path, repo_b: &Path) -> Snapshot {
    Snapshot {
        a_config: fs::read(repo_a.join(".git/config")).unwrap_or_default(),
        a_index: fs::read(repo_a.join(".git/index")).unwrap_or_default(),
        b_config: fs::read(repo_b.join(".git/config")).unwrap_or_default(),
    }
}

/// Status JSON, or a description of the failure (a hostile env may break the
/// agent outright; that is a leak too and must not hide the other rows).
fn oneshot_status(
    env: &CliTestEnv,
    repo_b: &Path,
    hostile: &[(&str, String)],
) -> Result<serde_json::Value, String> {
    let result = env
        .run_gpy_agent_with_env(
            &["oneshot", "git", "--cwd", &repo_b.to_string_lossy()],
            hostile,
        )
        .map_err(|err| format!("spawn gpy-agent: {err}"))?;
    if result.exit_code != 0_i32 {
        return Err(format!("exit {}: {}", result.exit_code, result.stderr));
    }
    serde_json::from_str(&result.stdout)
        .map_err(|err| format!("invalid JSON ({err}): {}", result.stdout))
}

#[test]
fn test_git_status_is_independent_of_inherited_git_env() {
    let env = CliTestEnv::new().expect("env");
    let (repo_a, repo_b) = make_repos(env.root());

    let no_env: &[(&str, String)] = &[];
    let clean = oneshot_status(&env, &repo_b, no_env).expect("clean run");
    assert_eq!(
        clean.get("branch"),
        Some(&serde_json::json!("main")),
        "clean baseline: {clean}"
    );
    assert_eq!(
        clean.get("untracked"),
        Some(&serde_json::json!(1_i32)),
        "clean baseline: {clean}"
    );
    let after_clean = snapshot(&repo_a, &repo_b);

    let mut failures = Vec::new();
    for (label, vars) in hostile_rows(&repo_a) {
        match oneshot_status(&env, &repo_b, &vars) {
            Ok(hostile) if hostile == clean => {}
            Ok(hostile) => failures.push(format!("{label}: status {hostile} != clean {clean}")),
            Err(err) => failures.push(format!("{label}: agent failed: {err}")),
        }
        let after = snapshot(&repo_a, &repo_b);
        if after != after_clean {
            failures.push(format!(
                "{label}: agent wrote to a config/index it was not asked to"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "hostile env leaked:\n{}",
        failures.join("\n")
    );
}

/// (label, local setting, `GIT_CONFIG_GLOBAL` for the agent, expected local values)
type CacheRow = (
    &'static str,
    Option<&'static str>,
    String,
    Vec<&'static str>,
);

/// `git config --local --get-all core.untrackedCache` values, one per entry.
fn local_untracked_cache(repo: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["config", "--local", "--get-all", "core.untrackedCache"])
        .current_dir(repo)
        .output()
        .expect("spawn git");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// #715: gpy must never override a `core.untrackedCache` the user set at any
/// scope, but still enables it when it is unset everywhere.
#[test]
fn test_agent_respects_user_untracked_cache_setting() {
    let env = CliTestEnv::new().expect("env");
    let global_false = env.root().join("global_false.gitconfig");
    fs::write(&global_false, "[core]\n\tuntrackedCache = false\n").expect("write gitconfig");
    let global_empty = env.root().join("global_empty.gitconfig");
    fs::write(&global_empty, "").expect("write gitconfig");
    let global_false_str = global_false.to_string_lossy().into_owned();
    let global_empty_str = global_empty.to_string_lossy().into_owned();

    let rows: Vec<CacheRow> = vec![
        (
            "local false",
            Some("false"),
            global_empty_str.clone(),
            vec!["false"],
        ),
        ("global false", None, global_false_str, vec![]),
        ("unset everywhere", None, global_empty_str, vec!["true"]),
    ];
    let mut failures = Vec::new();
    for (index, (label, local, global, expected)) in rows.into_iter().enumerate() {
        let repo = env.root().join(format!("uc{index}"));
        fs::create_dir_all(&repo).expect("mkdir");
        git(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("f"), "x\n").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        fs::write(repo.join("untracked.txt"), "z\n").expect("write");
        if let Some(value) = local {
            git(&repo, &["config", "core.untrackedCache", value]);
        }
        let vars = [("GIT_CONFIG_GLOBAL", global)];
        match oneshot_status(&env, &repo, &vars) {
            Ok(status) if status.get("untracked") == Some(&serde_json::json!(1_i32)) => {}
            Ok(status) => failures.push(format!("{label}: wrong status {status}")),
            Err(err) => failures.push(format!("{label}: agent failed: {err}")),
        }
        let actual = local_untracked_cache(&repo);
        if actual != expected {
            failures.push(format!(
                "{label}: local core.untrackedCache {actual:?}, expected {expected:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "untrackedCache policy violated:\n{}",
        failures.join("\n")
    );
}
