//! `gpy-agent oneshot` honours `ui.enabled_segments` as the only segment
//! switch: an unlisted `git` or `language` does no work.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use common::cli_harness::CliTestEnv;
use common::git::git_init;

#[test]
fn oneshot_git_with_git_unlisted_reports_disabled() {
    let env = CliTestEnv::with_config("[ui]\nenabled_segments = [\"clock\", \"directory\"]\n")
        .expect("create isolated CLI test env");
    let repo = env.root().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    git_init(&repo);
    let cwd = repo.to_string_lossy();

    let result = env
        .run_gpy_agent(&["oneshot", "git", "--cwd", &cwd, "--format", "json"])
        .expect("run gpy-agent");

    assert!(
        result.stdout.contains("Git segment disabled via config"),
        "{result:?}"
    );
}

#[test]
fn oneshot_lang_with_language_unlisted_reports_nothing() {
    let env = CliTestEnv::with_config("[ui]\nenabled_segments = [\"clock\", \"directory\"]\n")
        .expect("create isolated CLI test env");
    let project = env.root().join("project");
    std::fs::create_dir_all(&project).expect("create project dir");
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("marker");
    let cwd = project.to_string_lossy();

    let result = env
        .run_gpy_agent(&["oneshot", "lang", "--cwd", &cwd])
        .expect("run gpy-agent");

    result.assert_success("gpy-agent oneshot lang");
    let json: serde_json::Value = serde_json::from_str(&result.stdout).expect("JSON output");
    let languages = json
        .get("languages")
        .and_then(serde_json::Value::as_array)
        .expect("languages array");
    assert!(languages.is_empty(), "{result:?}");
}
