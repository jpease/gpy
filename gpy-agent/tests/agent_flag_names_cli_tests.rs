//! Renamed `gpy-agent` subcommands and flags.
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::panic, reason = "test code may panic on failure")]

mod common;

use common::cli_harness::CliTestEnv;

/// `gpy-agent <args>` in an isolated environment, which must exit with `code`.
fn run(env: &CliTestEnv, args: &[&str], code: i32) -> common::cli_harness::CliCommandResult {
    let result = env.run_gpy_agent(args).expect("spawn gpy-agent");
    assert_eq!(
        result.exit_code,
        code,
        "gpy-agent {}: {result:?}",
        args.join(" ")
    );
    result
}

#[test]
fn oneshot_exit_status_renders_the_pill() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let tail = ["--exit-code", "1", "--format", "ansi"];
    let result = run(
        &env,
        &[&["oneshot", "exit-status"][..], &tail].concat(),
        0_i32,
    );

    assert!(!result.stdout.is_empty(), "the pill renders: {result:?}");
    assert!(result.stderr.is_empty(), "silent on stderr: {result:?}");
    run(&env, &[&["oneshot", "status"][..], &tail].concat(), 2_i32);
}

#[test]
fn theme_export_shell_selects_the_shell() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let result = run(&env, &["theme", "export", "--shell", "bash"], 0_i32);

    assert!(!result.stdout.is_empty(), "the export prints: {result:?}");
    assert!(result.stderr.is_empty(), "silent on stderr: {result:?}");
    run(&env, &["theme", "export", "--format", "bash"], 2_i32);
}

/// The text after the flag column of the `--socket` line in `gpy-agent <args>
/// --help`.
fn socket_help(env: &CliTestEnv, args: &[&str]) -> String {
    let help = run(env, &[args, &["--help"]].concat(), 0_i32).stdout;
    let line = help
        .lines()
        .find(|line| line.contains("--socket"))
        .unwrap_or_else(|| {
            panic!(
                "`gpy-agent {} --help` lists --socket:\n{help}",
                args.join(" ")
            )
        });
    line.split_once("<PATH>")
        .unwrap_or_else(|| panic!("the --socket line names <PATH>: {line}"))
        .1
        .trim()
        .to_owned()
}

#[test]
fn help_surface_matches_the_decisions() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let socket_lines: Vec<String> = [
        &["start"][..],
        &["stop"][..],
        &["status"][..],
        &["restart"][..],
        &["config", "reload"][..],
    ]
    .iter()
    .map(|args| socket_help(&env, args))
    .collect();
    let first = socket_lines.first().expect("five commands were checked");
    assert!(
        socket_lines.iter().all(|line| line == first),
        "every --socket help is identical: {socket_lines:?}"
    );
    assert!(first.contains("GPY_AGENT_SOCKET_PATH"), "{socket_lines:?}");

    let git_help = run(&env, &["oneshot", "git", "--help"], 0_i32).stdout;
    assert!(!git_help.contains("--benchmark-mode"), "{git_help}");
    assert!(git_help.contains("<DIR>"), "{git_help}");
    let cwd_line = git_help
        .lines()
        .find(|line| line.contains("--cwd"))
        .expect("the --cwd line");
    assert_eq!(
        cwd_line.matches("default").count(),
        1,
        "the default is stated once: {cwd_line}"
    );

    let oneshot_help = run(&env, &["oneshot", "--help"], 0_i32).stdout;
    assert!(oneshot_help.contains("exit-status"), "{oneshot_help}");
    assert!(
        !oneshot_help
            .lines()
            .any(|line| line.trim_start().starts_with("status ")),
        "the deprecated `status` is hidden: {oneshot_help}"
    );

    let top_help = run(&env, &["--help"], 0_i32).stdout;
    assert!(
        top_help
            .lines()
            .any(|line| line.trim_start().starts_with("status ")),
        "the health check `status` stays listed: {top_help}"
    );
    assert!(!top_help.contains("exit-status"), "{top_help}");
}
