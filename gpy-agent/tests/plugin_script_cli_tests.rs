//! `gpy plugin new/validate/list` and `gpy doctor` for script (G) plugins,
//! plus the example plugins under `docs/dev/examples/plugins`.

#![cfg(unix)]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::{CliCommandResult, CliTestEnv};

const ENABLED_DEMO_CONFIG: &str = r#"
[agent]
enabled = true

[ui]
theme = "default"
enabled_segments = ["clock", "demo"]
"#;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("docs/dev/examples/plugins")
}

/// Whether `shellcheck` or `dash` is on `PATH`, so `validate` can lint.
fn have_linter() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .any(|dir| dir.join("shellcheck").is_file() || dir.join("dash").is_file())
}

/// Install script plugin `demo-tools` providing segment `demo`.
fn install_plugin(env: &CliTestEnv, segment_toml: &str, script: &str) -> PathBuf {
    let root = env.plugins_dir().join("demo-tools");
    fs::create_dir_all(root.join("segments")).expect("segments dir");
    fs::create_dir_all(root.join("bin")).expect("bin dir");
    fs::write(
        root.join("plugin.toml"),
        "id = \"demo-tools\"\nname = \"Demo\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
         entry_type = \"script\"\nprovided_segments = [\"demo\"]\n",
    )
    .expect("plugin manifest");
    fs::write(root.join("segments/demo.toml"), segment_toml).expect("segment manifest");
    fs::write(root.join("bin/demo.sh"), script).expect("script");
    root
}

fn validate(env: &CliTestEnv, target: &Path) -> CliCommandResult {
    env.run_gpy(&["plugin", "validate", target.to_str().expect("utf8 path")])
        .expect("spawn gpy")
}

fn combined(result: &CliCommandResult) -> String {
    format!("{}{}", result.stdout, result.stderr)
}

fn doctor(env: &CliTestEnv) -> String {
    combined(&env.run_gpy(&["doctor"]).expect("spawn gpy"))
}

#[test]
fn new_scaffolds_a_script_plugin_that_validates_and_runs() {
    let env = CliTestEnv::new().expect("env");
    env.run_gpy(&["plugin", "new", "demo-tools", "--segment", "demo_status"])
        .expect("spawn gpy")
        .assert_success("gpy plugin new");

    let root = env.plugins_dir().join("demo-tools");
    let manifest = fs::read_to_string(root.join("plugin.toml")).expect("plugin.toml");
    assert!(manifest.contains("entry_type = \"script\""), "{manifest}");
    assert!(
        manifest.contains("provided_segments = [\"demo_status\"]"),
        "{manifest}"
    );
    assert!(
        !root.join("segments/demo_status.fish").exists(),
        "a G scaffold has no Fish file"
    );

    let segment = fs::read_to_string(root.join("segments/demo_status.toml")).expect("segment");
    for needle in [
        "command = \"bin/demo_status.sh\"",
        "inputs = []",
        "env = []",
        "ttl_secs = 5",
        "timeout_ms = 300",
    ] {
        assert!(segment.contains(needle), "{needle:?} missing:\n{segment}");
    }
    let script = fs::read_to_string(root.join("bin/demo_status.sh")).expect("script");
    assert!(script.starts_with("#!/bin/sh\n"), "{script}");
    assert!(script.contains("4 KiB"), "{script}");
    assert!(script.contains("no background jobs"), "{script}");

    let validated = validate(&env, &root);
    validated.assert_success("validate the scaffold");
    assert!(
        validated
            .stdout
            .contains("description: Describe what this segment shows"),
        "{}",
        validated.stdout
    );
    assert!(
        !validated.stdout.contains("uses $"),
        "the scaffold must validate without warnings:\n{}",
        validated.stdout
    );

    let run = Command::new("/bin/sh")
        .arg(root.join("bin/demo_status.sh"))
        .output()
        .expect("run scaffold");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "demo_status\n");
}

/// Run `gpy args...` through `sh` under `umask`, as a shell user would.
fn run_gpy_under_umask(env: &CliTestEnv, umask: &str, args: &[&str]) -> CliCommandResult {
    let mut command_line = vec![
        "-c",
        "umask \"$0\"; exec \"$@\"",
        umask,
        env!("CARGO_BIN_EXE_gpy"),
    ];
    command_line.extend_from_slice(args);
    let none: [(&str, &str); 0] = [];
    env.run_program_with_env(Path::new("/bin/sh"), &command_line, &none)
        .expect("spawn sh")
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    fs::metadata(path).expect("metadata").permissions().mode() & 0o7777
}

#[test]
fn new_scaffold_works_under_common_umasks() {
    for umask in ["0000", "0002", "0022", "0077"] {
        let env = CliTestEnv::new().expect("env");
        run_gpy_under_umask(
            &env,
            umask,
            &["plugin", "new", "demo-tools", "--segment", "demo"],
        )
        .assert_success("gpy plugin new");
        let root = env.plugins_dir().join("demo-tools");

        for path in [
            root.clone(),
            root.join("bin"),
            root.join("segments"),
            root.join("plugin.toml"),
            root.join("segments/demo.toml"),
            root.join("bin/demo.sh"),
        ] {
            let mode = mode_of(&path);
            assert_eq!(
                mode & 0o022,
                0,
                "umask {umask}: {} is {mode:o}",
                path.display()
            );
            assert_eq!(
                mode & 0o600,
                0o600,
                "umask {umask}: owner access lost on {}",
                path.display()
            );
        }

        validate(&env, &root).assert_success("validate by path");
        let by_id = env
            .run_gpy(&["plugin", "validate", "demo-tools"])
            .expect("spawn gpy");
        by_id.assert_success("validate by id");
        let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
        list.assert_success("plugin list");
        assert!(
            list.stdout.contains("segment demo:"),
            "umask {umask}: {}",
            list.stdout
        );
        env.run_gpy(&["enable", "demo"])
            .expect("spawn gpy")
            .assert_success("enable the scaffolded segment");
    }
}

#[test]
fn new_refuses_an_existing_target_without_touching_it() {
    let env = CliTestEnv::new().expect("env");
    let root = env.plugins_dir().join("demo-tools");
    fs::create_dir_all(&root).expect("existing root");
    fs::write(root.join("keep.txt"), "mine").expect("user file");
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o775)).expect("chmod");
    }
    let result = run_gpy_under_umask(&env, "0002", &["plugin", "new", "demo-tools"]);
    assert_ne!(result.exit_code, 0_i32, "{}", combined(&result));
    assert!(
        combined(&result).contains("already exists"),
        "{}",
        combined(&result)
    );
    assert_eq!(mode_of(&root), 0o775);
    assert_eq!(
        fs::read_to_string(root.join("keep.txt")).expect("kept"),
        "mine"
    );
    assert!(!root.join("plugin.toml").exists());
}

#[test]
fn validate_prints_each_segment_description() {
    let env = CliTestEnv::new().expect("env");
    let root = install_plugin(
        &env,
        "description = \"Shows the demo\"\ncommand = \"bin/demo.sh\"\n",
        "printf 'demo\\n'\n",
    );
    let result = validate(&env, &root);
    result.assert_success("validate");
    assert!(
        result.stdout.contains("segment demo:")
            && result.stdout.contains("description: Shows the demo"),
        "{}",
        result.stdout
    );
}

#[test]
fn validate_rejects_a_command_outside_the_plugin_root() {
    let env = CliTestEnv::new().expect("env");
    let root = install_plugin(&env, "command = \"../outside.sh\"\n", "true\n");
    let parent = validate(&env, &root);
    assert_ne!(parent.exit_code, 0_i32, "{parent:?}");

    // A symlink inside the root that points outside is refused as well.
    let outside = env.root().join("outside.sh");
    fs::write(&outside, "printf 'x\\n'\n").expect("outside script");
    fs::remove_file(root.join("bin/demo.sh")).expect("remove");
    std::os::unix::fs::symlink(&outside, root.join("bin/demo.sh")).expect("symlink");
    fs::write(
        root.join("segments/demo.toml"),
        "command = \"bin/demo.sh\"\n",
    )
    .expect("manifest");
    let linked = validate(&env, &root);
    assert_ne!(linked.exit_code, 0_i32, "{linked:?}");
    assert!(
        combined(&linked).contains("outside the plugin root"),
        "{}",
        combined(&linked)
    );

    fs::remove_file(root.join("bin/demo.sh")).expect("remove");
    fs::write(root.join("bin/demo.sh"), "true\n").expect("script");
    fs::write(
        root.join("segments/demo.toml"),
        "command = \"bin/demo.sh\"\nrequire_command = \"../x.sh\"\n",
    )
    .expect("manifest");
    let required = validate(&env, &root);
    assert_ne!(required.exit_code, 0_i32, "{required:?}");
}

#[test]
fn validate_rejects_a_manifest_that_breaks_the_schema() {
    let env = CliTestEnv::new().expect("env");
    for (manifest, needle) in [
        (
            "command = \"bin/demo.sh\"\nttl_secs = 99999999999\n",
            "ttl_secs",
        ),
        (
            "command = \"bin/demo.sh\"\ntimeout_ms = 900\n",
            "timeout_ms",
        ),
        ("command = \"bin/demo.sh\"\nunknown = 1\n", "unknown"),
        (
            "command = \"bin/demo.sh\"\nenv = [\"GPY_PWD\"]\n",
            "reserved",
        ),
    ] {
        let root = install_plugin(&env, manifest, "true\n");
        let result = validate(&env, &root);
        assert_ne!(result.exit_code, 0_i32, "{manifest}: {result:?}");
        assert!(
            combined(&result).contains(needle),
            "{needle}: {}",
            combined(&result)
        );
        fs::remove_dir_all(&root).expect("reset");
    }
}

#[test]
fn validate_lints_the_script_as_posix_sh() {
    if !have_linter() {
        eprintln!("skipped: neither shellcheck nor dash is installed");
        return;
    }
    let env = CliTestEnv::new().expect("env");
    // `[[ ]]` and arrays are bashisms that macOS `sh -n` (Bash 3.2) accepts.
    let root = install_plugin(
        &env,
        "command = \"bin/demo.sh\"\n",
        "#!/bin/sh\nitems=(a b)\nif [[ -n ${items[0]} ]]; then printf 'x\\n'; fi\n",
    );
    let result = validate(&env, &root);
    assert_ne!(
        result.exit_code, 0_i32,
        "a bashism must fail the lint: {result:?}"
    );
    assert!(
        combined(&result).contains("failed"),
        "{}",
        combined(&result)
    );

    fs::write(root.join("bin/demo.sh"), "#!/bin/sh\nprintf 'ok\\n'\n").expect("fix script");
    let fixed = validate(&env, &root);
    fixed.assert_success("a clean script passes");
    assert!(
        fixed.stdout.contains("found no problems"),
        "the linter used is named:\n{}",
        fixed.stdout
    );
}

#[test]
fn validate_warns_about_undeclared_variables() {
    let env = CliTestEnv::new().expect("env");
    let root = install_plugin(
        &env,
        "command = \"bin/demo.sh\"\nenv = [\"KUBECONFIG\"]\n",
        "#!/bin/sh\nprintf '%s %s %s\\n' \"$KUBECONFIG\" \"$GPY_PWD\" \"$AWS_PROFILE\"\n",
    );
    let result = validate(&env, &root);
    result.assert_success("undeclared variables only warn");
    assert!(
        result.stdout.contains("uses $GPY_PWD") && result.stdout.contains("\"pwd\""),
        "{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("uses $AWS_PROFILE"),
        "{}",
        result.stdout
    );
    assert!(
        !result.stdout.contains("uses $KUBECONFIG"),
        "{}",
        result.stdout
    );
}

#[test]
fn list_shows_script_segments_without_a_fish_note() {
    let env = CliTestEnv::new().expect("env");
    install_plugin(
        &env,
        "description = \"Shows the demo\"\ncommand = \"bin/demo.sh\"\ninputs = [\"pwd\"]\nttl_secs = 9\n",
        "printf 'demo\\n'\n",
    );
    let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
    list.assert_success("plugin list");
    assert!(
        list.stdout.contains("- demo-tools 0.1.0"),
        "{}",
        list.stdout
    );
    assert!(
        list.stdout
            .contains("segment demo: folder scope, ttl_secs 9, timeout_ms 500 - Shows the demo"),
        "{}",
        list.stdout
    );
    assert!(!list.stdout.contains("Fish"), "{}", list.stdout);
}

#[test]
fn doctor_times_an_enabled_script_segment() {
    let env = CliTestEnv::with_config(ENABLED_DEMO_CONFIG).expect("env");
    install_plugin(
        &env,
        "command = \"bin/demo.sh\"\ntimeout_ms = 400\n",
        "printf 'hello\\nwarn\\n'\n",
    );
    let report = doctor(&env);
    assert!(report.contains("Plugin Performance"), "{report}");
    assert!(
        report.contains("trial: ran in") && report.contains("(timeout_ms 400)"),
        "{report}"
    );
    assert!(
        report.contains("shows \"hello\" with variant warn"),
        "{report}"
    );
}

#[test]
fn doctor_kills_a_script_after_timeout_ms() {
    let env = CliTestEnv::with_config(ENABLED_DEMO_CONFIG).expect("env");
    install_plugin(
        &env,
        "command = \"bin/demo.sh\"\ntimeout_ms = 100\n",
        "sleep 30\nprintf 'late\\n'\n",
    );
    let started = std::time::Instant::now();
    let report = doctor(&env);
    assert!(
        report.contains("trial: killed after timeout_ms 100"),
        "{report}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "doctor must not wait for the script"
    );
}

#[test]
fn doctor_runs_the_script_in_a_clean_environment() {
    let env = CliTestEnv::with_config(ENABLED_DEMO_CONFIG).expect("env");
    install_plugin(
        &env,
        "command = \"bin/demo.sh\"\nenv = [\"DECLARED_VAR\"]\n",
        "printf '%s|%s|%s\\n' \"${SECRET_TOKEN:-unset}\" \"${DECLARED_VAR:-unset}\" \"${GPY_OPT_x:-unset}\"\n",
    );
    let result = env
        .run_gpy_with_env(
            &["doctor"],
            &[
                ("SECRET_TOKEN", "leak"),
                ("DECLARED_VAR", "kept"),
                ("GPY_OPT_x", "leak"),
            ],
        )
        .expect("spawn gpy");
    let report = combined(&result);
    assert!(report.contains("shows \"unset|kept|unset\""), "{report}");
}

#[test]
fn doctor_uses_the_plugin_root_unless_pwd_is_declared() {
    let env = CliTestEnv::with_config(ENABLED_DEMO_CONFIG).expect("env");
    let script = "printf '%s\\n' \"$(basename \"$(pwd)\")\"\n";
    install_plugin(&env, "command = \"bin/demo.sh\"\n", script);
    let root_report = doctor(&env);
    assert!(
        root_report.contains("shows \"demo-tools\""),
        "{root_report}"
    );

    install_plugin(
        &env,
        "command = \"bin/demo.sh\"\ninputs = [\"pwd\"]\n",
        script,
    );
    let cwd_report = doctor(&env);
    let cwd = std::env::current_dir().expect("cwd");
    let expected = cwd
        .file_name()
        .expect("cwd name")
        .to_string_lossy()
        .into_owned();
    assert!(
        cwd_report.contains(&format!("shows \"{expected}\"")),
        "{cwd_report}"
    );
}

#[test]
fn doctor_reports_a_hidden_segment() {
    let env = CliTestEnv::with_config(ENABLED_DEMO_CONFIG).expect("env");
    install_plugin(&env, "command = \"bin/demo.sh\"\n", "exit 3\n");
    let report = doctor(&env);
    assert!(report.contains("segment hidden: non-zero exit"), "{report}");
}

#[test]
fn example_plugins_validate_and_stay_short_lived() {
    let env = CliTestEnv::new().expect("env");
    for (dir, segment, description) in [
        ("k8s-context", "k8s", "Current kubectl context"),
        ("username-example", "whoami-user", "Login name"),
        ("hostname-example", "host-name", "Host name"),
    ] {
        let root = examples_dir().join(dir);
        let result = validate(&env, &root);
        result.assert_success(&format!("validate example {dir}"));
        assert!(
            result.stdout.contains(&format!("segment {segment}:"))
                && result
                    .stdout
                    .contains(&format!("description: {description}")),
            "{dir}:\n{}",
            result.stdout
        );
        assert!(
            !result.stdout.contains("uses $"),
            "{dir} must not warn about undeclared variables:\n{}",
            result.stdout
        );
        let manifest = fs::read_to_string(root.join(format!("segments/{segment}.toml")))
            .expect("segment manifest");
        let ttl = manifest
            .lines()
            .find_map(|line| line.strip_prefix("ttl_secs = "))
            .expect("every example sets ttl_secs")
            .trim()
            .parse::<u32>()
            .expect("ttl_secs is a number");
        assert!(
            (1..=30).contains(&ttl),
            "{dir}: ttl_secs {ttl} is not short"
        );
    }
}

#[test]
fn k8s_example_declares_kubeconfig_and_no_pwd() {
    let manifest = fs::read_to_string(examples_dir().join("k8s-context/segments/k8s.toml"))
        .expect("k8s manifest");
    assert!(manifest.contains("env = [\"KUBECONFIG\"]"), "{manifest}");
    assert!(
        !manifest.contains("inputs"),
        "no pwd: session scope teaches the cache: {manifest}"
    );
}

#[test]
fn k8s_example_reads_the_current_context() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let kubeconfig = temp.path().join("config");
    fs::write(
        &kubeconfig,
        "apiVersion: v1\ncurrent-context: \"prod-eu\"\nkind: Config\n",
    )
    .expect("kubeconfig");
    let script = examples_dir().join("k8s-context/bin/k8s.sh");
    let run = |config: &Path| {
        Command::new("/bin/sh")
            .arg(&script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("KUBECONFIG", config)
            .output()
            .expect("run k8s example")
    };
    assert_eq!(
        String::from_utf8_lossy(&run(&kubeconfig).stdout),
        "prod-eu\nprod\n"
    );
    assert_eq!(run(&temp.path().join("missing")).stdout, b"");
}
