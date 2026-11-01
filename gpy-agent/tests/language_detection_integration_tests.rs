//! Language detection across supported languages via CLI and IPC formats.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

#[cfg(unix)]
use gpy_agent::config::defaults::DEFAULT_LANGUAGE_ICONS;
use gpy_agent::config::types::DetectionMode;
use gpy_agent::config::{Config, loader};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(unix)]
#[path = "common/child_guard.rs"]
mod child_guard;
#[path = "common/git.rs"]
mod git;
#[cfg(unix)]
#[path = "common/wait.rs"]
mod wait;

use git::git_init;

fn setup_node_project(path: &Path) {
    fs::write(path.join("package.json"), r#"{"name": "test"}"#)
        .expect("Failed to write package.json");
    fs::write(path.join("index.js"), "console.log('hello');").expect("Failed to write index.js");
}

fn setup_python_project(path: &Path) {
    fs::write(path.join("requirements.txt"), "requests==2.28.0")
        .expect("Failed to write requirements.txt");
    fs::write(path.join("main.py"), "print('hello')").expect("Failed to write main.py");
}

fn setup_rust_project(path: &Path) {
    fs::write(path.join("Cargo.toml"), r#"[package]\nname = "test""#)
        .expect("Failed to write Cargo.toml");
    fs::write(path.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
}

fn setup_go_project(path: &Path) {
    fs::write(path.join("go.mod"), "module test").expect("Failed to write go.mod");
    fs::write(path.join("main.go"), "package main\nfunc main() {}")
        .expect("Failed to write main.go");
}

fn setup_swift_project(path: &Path) {
    fs::write(path.join("Package.swift"), "// swift-tools-version:5.0")
        .expect("Failed to write Package.swift");
    fs::write(path.join("main.swift"), "print(\"hello\")").expect("Failed to write main.swift");
}

fn setup_elixir_project(path: &Path) {
    fs::write(path.join("mix.exs"), "defmodule Test.MixProject do\nend")
        .expect("Failed to write mix.exs");
    fs::create_dir_all(path.join("lib")).expect("Failed to create lib directory");
    fs::write(path.join("lib/test.ex"), "defmodule Test do\nend").expect("Failed to write test.ex");
}

fn setup_erlang_project(path: &Path) {
    fs::write(path.join("rebar.config"), "{erl_opts, []}.").expect("Failed to write rebar.config");
    fs::create_dir_all(path.join("src")).expect("Failed to create src directory");
    fs::write(path.join("src/test.erl"), "-module(test).").expect("Failed to write test.erl");
}

fn setup_ruby_project(path: &Path) {
    fs::write(path.join("Gemfile"), "source 'https://rubygems.org'")
        .expect("Failed to write Gemfile");
    fs::write(path.join("app.rb"), "puts 'hello'").expect("Failed to write app.rb");
}

fn setup_java_project(path: &Path) {
    fs::write(path.join("pom.xml"), "<project></project>").expect("Failed to write pom.xml");
    fs::write(path.join("Main.java"), "public class Main {}").expect("Failed to write Main.java");
}

fn setup_fish_project(path: &Path) {
    fs::create_dir_all(path.join("functions")).expect("Failed to create functions directory");
    fs::write(path.join("functions/test.fish"), "function test\nend")
        .expect("Failed to write test.fish");
}

fn create_language_test_dir(language: &str) -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path();

    // Mark it as a git repo so the bounded content-mode scan runs instead of
    // falling back to markers.
    fs::create_dir_all(path.join(".git")).expect("Failed to create fake .git marker");

    match language {
        "node" => setup_node_project(path),
        "python" => setup_python_project(path),
        "rust" => setup_rust_project(path),
        "go" => setup_go_project(path),
        "swift" => setup_swift_project(path),
        "elixir" => setup_elixir_project(path),
        "erlang" => setup_erlang_project(path),
        "ruby" => setup_ruby_project(path),
        "java" => setup_java_project(path),
        "fish" => setup_fish_project(path),
        _ => panic!("Unknown language: {language}"),
    }

    temp_dir
}

fn setup_test_env() -> (TempDir, PathBuf) {
    setup_test_env_with_detection(DetectionMode::Markers)
}

fn setup_test_env_with_detection(mode: DetectionMode) -> (TempDir, PathBuf) {
    let home = TempDir::new().expect("Failed to create temp home");
    let config_root = home.path().join(".config");
    let cache_root = home.path().join(".cache");
    let runtime_root = home.path().join(".runtime");
    let gpy_config_dir = config_root.join("gpy");
    fs::create_dir_all(&gpy_config_dir).expect("Failed to create config dir");
    fs::create_dir_all(cache_root.join("gpy")).expect("Failed to create cache dir");
    fs::create_dir_all(&runtime_root).expect("Failed to create runtime dir");

    let config_path = gpy_config_dir.join("config.toml");
    let default_config = Config::default();
    let mut updated = default_config.clone();
    updated.language.detection_mode = mode;
    loader::save_config(
        &config_path,
        &default_config,
        &updated,
        &["ui.show_icons", "language.detection_mode"],
    )
    .expect("write default config");

    (home, config_path)
}

fn configure_command_env(command: &mut std::process::Command, home: &TempDir, config_path: &Path) {
    let home_path = home.path();
    let config_root = home_path.join(".config");
    let cache_root = home_path.join(".cache");
    let runtime_root = home_path.join(".runtime");

    command.env("HOME", home_path);
    command.env("XDG_CONFIG_HOME", &config_root);
    command.env("XDG_CACHE_HOME", &cache_root);
    command.env("XDG_RUNTIME_DIR", &runtime_root);
    command.env("GPY_CONFIG_PATH", config_path);
    command.env("GPY_DISABLE_WATCHER", "1");
}

#[test]
fn test_all_languages_detected() {
    let languages = vec![
        "node", "python", "rust", "go", "swift", "elixir", "erlang", "ruby", "java", "fish",
    ];
    // The fish fixture keeps its evidence under `functions/`, which only the content scan reads.
    let (home_env, config_path) = setup_test_env_with_detection(DetectionMode::Content);

    for lang in languages {
        let test_dir = create_language_test_dir(lang);
        let path = test_dir.path().to_string_lossy().to_string();

        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path, "--format", "json"]);
        configure_command_env(&mut command, &home_env, &config_path);
        let output = command.output().expect("Failed to execute gpy-agent");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "Language detection failed for {lang}: {stderr:?}"
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("Failed to parse JSON for {lang}: {e} | Output: {stdout}"));

        let languages_array = json
            .get("languages")
            .and_then(|v| v.as_array())
            .expect("languages should be an array");

        let found = languages_array
            .iter()
            .any(|l| l.get("name").and_then(|n| n.as_str()).unwrap_or("") == lang);

        assert!(
            found,
            "Language '{lang}' not detected in test directory. Output: {stdout}"
        );
    }
}

#[test]
fn test_version_detection_for_all_languages() {
    let languages = vec![
        "node", "python", "rust", "go", "swift", "elixir", "erlang", "ruby", "java", "fish",
    ];
    // The fish fixture keeps its evidence under `functions/`, which only the content scan reads.
    let (home_env, config_path) = setup_test_env_with_detection(DetectionMode::Content);

    for lang in languages {
        let test_dir = create_language_test_dir(lang);
        let path = test_dir.path().to_string_lossy().to_string();

        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path, "--format", "json"]);
        configure_command_env(&mut command, &home_env, &config_path);
        let output = command.output().expect("Failed to execute gpy-agent");

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("Failed to parse JSON");

        let languages_array = json.get("languages").and_then(|v| v.as_array()).unwrap();

        let lang_entry = languages_array
            .iter()
            .find(|l| l.get("name").and_then(|n| n.as_str()).unwrap_or("") == lang);

        let entry = lang_entry.unwrap_or_else(|| panic!("{lang} not detected: {stdout}"));
        // The version is null when the toolchain is not installed, never an empty string.
        let version = entry.get("version").expect("version key present");
        assert!(
            version.is_null() || version.as_str().is_some_and(|v| !v.is_empty()),
            "{lang} version must be null or a non-empty string, got {version}"
        );
    }
}

#[test]
fn test_format_parameter_honored() {
    let test_dir = create_language_test_dir("rust");
    let path = test_dir.path().to_string_lossy().to_string();
    let (home_env, config_path) = setup_test_env();

    let json_output = {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path, "--format", "json"]);
        configure_command_env(&mut command, &home_env, &config_path);
        command.output().expect("Failed to execute gpy-agent")
    };

    let json_stdout = String::from_utf8_lossy(&json_output.stdout);
    assert!(
        json_stdout.contains('{') && json_stdout.contains('}'),
        "JSON format should contain braces. Got: {json_stdout}"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&json_stdout).expect("JSON format should be valid JSON");
    assert!(parsed.is_object(), "JSON stdout should be an object");

    let fish_rendered_output = {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path, "--format", "ansi"]);
        configure_command_env(&mut command, &home_env, &config_path);
        command.output().expect("Failed to execute gpy-agent")
    };

    let fish_rendered_stdout = String::from_utf8_lossy(&fish_rendered_output.stdout);

    assert!(
        fish_rendered_stdout.contains("\x1b["),
        "Fish-rendered format should contain ANSI escape codes. Got: {fish_rendered_stdout}"
    );
    assert!(
        !fish_rendered_stdout.contains('{'),
        "Fish-rendered format should NOT be JSON. Got: {fish_rendered_stdout}"
    );
}

#[test]
fn test_ansi_format_ansi_codes() {
    let test_dir = create_language_test_dir("python");
    let path = test_dir.path().to_string_lossy().to_string();

    let (home_env, config_path) = setup_test_env();
    let output = {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path, "--format", "ansi"]);
        configure_command_env(&mut command, &home_env, &config_path);
        command.output().expect("Failed to execute gpy-agent")
    };

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.trim().is_empty(),
        "python project must render a segment"
    );
    assert!(stdout.contains("\x1b["), "Should contain ANSI escape codes");

    let has_color_codes = stdout.contains("30m")
        || stdout.contains("30;4") // combined fg black + bg color, e.g. "\x1b[30;44m"
        || stdout.contains("40m")
        || stdout.contains("43m")
        || stdout.contains("37m");
    assert!(has_color_codes, "Should contain ANSI color codes");

    assert!(
        stdout.contains("\x1b[0m") || stdout.contains("\x1b[m"),
        "Should contain reset codes"
    );

    assert!(!stdout.contains('{'), "Should not be JSON format");
    assert!(
        !stdout.contains("\"name\""),
        "Should not contain JSON fields"
    );
}

/// A daemon on a private socket, stopped on drop.
#[cfg(unix)]
struct RunningAgent {
    socket_path: PathBuf,
    // Field order is drop order: stop the agent before removing its socket dir.
    _stop: child_guard::AgentStopGuard,
    _socket_dir: TempDir,
}

#[cfg(unix)]
impl RunningAgent {
    fn start(home: &TempDir, config_path: &Path) -> Self {
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let socket_dir = TempDir::new().expect("Failed to create socket dir");
        let socket_path = socket_dir.path().join("gpy.sock");

        // Registered before `start` so the daemon stops even if start fails.
        let mut stop_command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        configure_command_env(&mut stop_command, home, config_path);
        let stop = child_guard::AgentStopGuard::new(stop_command, socket_path.clone());

        let mut start = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        start.args(["start", "--socket"]).arg(&socket_path);
        configure_command_env(&mut start, home, config_path);
        let started = start.output().expect("Failed to execute gpy-agent start");
        assert!(
            started.status.success(),
            "agent failed to start: {}",
            String::from_utf8_lossy(&started.stderr)
        );

        let accepted = wait::wait_until(Duration::from_secs(10), Duration::from_millis(25), || {
            UnixStream::connect(&socket_path).is_ok()
        });
        assert!(
            accepted,
            "agent never accepted a connection on {}",
            socket_path.display()
        );
        Self {
            socket_path,
            _stop: stop,
            _socket_dir: socket_dir,
        }
    }
}

#[cfg(unix)]
#[test]
fn test_ipc_format_parameter() {
    let (home_env, config_path) = setup_test_env();
    let daemon = RunningAgent::start(&home_env, &config_path);

    let test_dir = create_language_test_dir("rust");
    let path = test_dir.path().to_string_lossy().to_string();

    let request = format!(
        r#"{{"op":"lang","cwd":"{}","format":"ansi"}}"#,
        path.replace('"', "\\\"")
    );
    let response = ipc_request(&daemon.socket_path, &request);

    assert!(
        response.contains("\x1b["),
        "IPC ansi response should carry ANSI codes. Got: {response:?}"
    );
    assert!(
        !response.contains(r#""languages""#),
        "IPC should return ansi format, not JSON. Got: {response:?}"
    );
    // The default config renders the built-in Rust icon instead of the language name.
    let rust_icon = DEFAULT_LANGUAGE_ICONS
        .iter()
        .find_map(|(key, glyph)| (*key == "rust").then_some(*glyph))
        .expect("rust has a built-in icon");
    assert!(
        response.contains(rust_icon),
        "the rendered segment should show the detected language's icon. Got: {response:?}"
    );
}

#[test]
fn test_empty_directory() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().to_string_lossy().to_string();

    let (home_env, config_path) = setup_test_env();
    let output = {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path, "--format", "json"]);
        configure_command_env(&mut command, &home_env, &config_path);
        command.output().expect("Failed to execute gpy-agent")
    };

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Failed to parse JSON");

    let languages = json.get("languages").and_then(|v| v.as_array()).unwrap();
    assert_eq!(
        languages.len(),
        0,
        "Empty directory should have no languages"
    );
}

#[test]
fn test_polyglot_project() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path();

    fs::write(path.join("package.json"), r#"{"name": "test"}"#).ok();
    fs::write(path.join("index.js"), "console.log('hello');").ok();
    fs::write(path.join("requirements.txt"), "requests").ok();
    fs::write(path.join("main.py"), "print('hello')").ok();
    fs::write(path.join("Gemfile"), "source 'https://rubygems.org'").ok();
    fs::write(path.join("app.rb"), "puts 'hello'").ok();

    let path_str = path.to_string_lossy().to_string();
    let (home_env, config_path) = setup_test_env();

    let output = {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd", &path_str, "--format", "json"]);
        configure_command_env(&mut command, &home_env, &config_path);
        command.output().expect("Failed to execute gpy-agent")
    };

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Failed to parse JSON");

    let languages = json.get("languages").and_then(|v| v.as_array()).unwrap();

    let detected_names: Vec<&str> = languages
        .iter()
        .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(
        detected_names.contains(&"node") || detected_names.contains(&"javascript"),
        "Should detect Node/JavaScript"
    );
    assert!(detected_names.contains(&"python"), "Should detect Python");
    assert!(detected_names.contains(&"ruby"), "Should detect Ruby");
}

// `oneshot lang` in a subdirectory resolves the git root first, so a stray
// `docs/conf.py` in a Rust repo doesn't report `python`.
#[test]
fn test_oneshot_lang_uses_git_root_in_subdirectory() {
    let (home_env, config_path) = setup_test_env();
    let repo = TempDir::new().expect("Failed to create temp dir");
    let root = repo.path();

    git_init(root);

    fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n")
        .expect("Failed to write Cargo.toml");
    fs::create_dir_all(root.join("src")).expect("Failed to create src");
    fs::write(root.join("src/main.rs"), "fn main() {}").expect("Failed to write main.rs");
    fs::create_dir_all(root.join("docs")).expect("Failed to create docs");
    fs::write(root.join("docs/conf.py"), "x = 1\n").expect("Failed to write conf.py");

    let names_for = |cwd: &Path| -> Vec<String> {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
        command.args(["oneshot", "lang", "--cwd"]);
        command.arg(cwd);
        command.args(["--format", "json"]);
        configure_command_env(&mut command, &home_env, &config_path);
        let output = command.output().expect("Failed to execute gpy-agent");
        assert!(output.status.success(), "oneshot lang failed");
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("Failed to parse JSON");
        json.get("languages")
            .and_then(|v| v.as_array())
            .expect("languages should be an array")
            .iter()
            .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_owned))
            .collect()
    };

    let at_root = names_for(root);
    let at_sub = names_for(&root.join("docs"));

    assert!(
        at_sub.iter().any(|n| n == "rust"),
        "rust must be detected from a subdirectory: {at_sub:?}"
    );
    assert!(
        !at_sub.iter().any(|n| n == "python"),
        "subdirectory-only python must not leak into the repo-level answer: {at_sub:?}"
    );
    assert_eq!(at_sub, at_root, "subdirectory must match the repo root");
}

// Outside git, a source-less subdirectory resolves the nearest marker
// ancestor as project root.
#[test]
fn test_nongit_subdirectory_inherits_project_language() {
    let (home_env, config_path) = setup_test_env();
    // Under the isolated HOME, so no enclosing git repo is ever found.
    let project = home_env.path().join("nongit");
    fs::create_dir_all(project.join("src")).expect("Failed to create src");
    fs::create_dir_all(project.join("docs")).expect("Failed to create docs");
    fs::write(project.join("package.json"), "{}").expect("Failed to write package.json");
    fs::write(project.join("src/index.js"), "x\n").expect("Failed to write index.js");
    fs::write(project.join("docs/README.md"), "# d\n").expect("Failed to write README.md");

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
    command.args(["oneshot", "lang", "--cwd"]);
    command.arg(project.join("docs"));
    command.args(["--format", "json"]);
    configure_command_env(&mut command, &home_env, &config_path);
    let output = command.output().expect("Failed to execute gpy-agent");
    assert!(output.status.success(), "oneshot lang failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Failed to parse JSON");
    let found = json
        .get("languages")
        .and_then(|v| v.as_array())
        .expect("languages should be an array")
        .iter()
        .any(|l| l.get("name").and_then(|n| n.as_str()) == Some("node"));
    assert!(
        found,
        "a non-git docs/ subdirectory must inherit the project's node: {stdout}"
    );
}

/// The default `agent.timeout_seconds`: the budget a language request gets.
#[cfg(unix)]
const OPERATION_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// Run `oneshot lang --format json` at `cwd` and return the parsed output,
/// failing if it outlives [`OPERATION_BUDGET`].
#[cfg(unix)]
fn oneshot_lang_json(cwd: &Path, home: &TempDir, config_path: &Path) -> serde_json::Value {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
    command.args(["oneshot", "lang", "--cwd"]);
    command.arg(cwd);
    command.args(["--format", "json"]);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    configure_command_env(&mut command, home, config_path);
    let mut child = command.spawn().expect("Failed to execute gpy-agent");
    let deadline = std::time::Instant::now()
        .checked_add(OPERATION_BUDGET)
        .expect("deadline fits in Instant");
    while child.try_wait().expect("poll gpy-agent").is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("oneshot lang at {} outlived its budget", cwd.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("collect gpy-agent output");
    assert!(
        output.status.success(),
        "oneshot lang failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Failed to parse JSON")
}

/// The `python` entry of a `lang` JSON reply.
#[cfg(unix)]
fn python_entry(json: &serde_json::Value) -> &serde_json::Value {
    json.get("languages")
        .and_then(|v| v.as_array())
        .expect("languages should be an array")
        .iter()
        .find(|l| l.get("name").and_then(|n| n.as_str()) == Some("python"))
        .unwrap_or_else(|| panic!("python must be detected: {json}"))
}

// A project `.venv` is repository content: rendering reads its `pyvenv.cfg`
// but never runs its interpreter, and without a usable version shows none.
#[cfg(unix)]
#[test]
fn test_discovered_venv_interpreter_is_never_executed() {
    use std::os::unix::fs::PermissionsExt;

    let (home_env, config_path) = setup_test_env();
    let cfgs: [Option<&str>; 3] = [
        None,
        Some("home = /x\nversion_info = final\n"),
        Some("home = /x\ninclude-system-site-packages = false\n"),
    ];
    for (index, cfg) in cfgs.into_iter().enumerate() {
        let project = home_env.path().join(format!("venv-project-{index}"));
        let bin = project.join(".venv/bin");
        fs::create_dir_all(&bin).expect("Failed to create .venv/bin");
        setup_python_project(&project);
        fs::write(project.join("pyproject.toml"), "[project]\nname = \"x\"\n")
            .expect("Failed to write pyproject.toml");
        let marker = project.join("executed");
        let python = bin.join("python");
        fs::write(
            &python,
            format!(
                "#!/bin/sh\nprintf executed > '{}'\nprintf 'Python 3.11.9\\n'\n",
                marker.display()
            ),
        )
        .expect("Failed to write fixture python");
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755))
            .expect("Failed to chmod fixture python");
        if let Some(contents) = cfg {
            fs::write(project.join(".venv/pyvenv.cfg"), contents).expect("Failed to write cfg");
        }

        let json = oneshot_lang_json(&project, &home_env, &config_path);

        assert!(
            !marker.exists(),
            "the project interpreter must not run (pyvenv.cfg {cfg:?})"
        );
        assert!(
            python_entry(&json)
                .get("version")
                .is_none_or(serde_json::Value::is_null),
            "no usable pyvenv.cfg version means no version (pyvenv.cfg {cfg:?}): {json}"
        );
    }
}

/// Create a FIFO at `path`; no writer ever opens it.
#[cfg(unix)]
fn make_fifo(path: &Path) {
    nix::unistd::mkfifo(path, nix::sys::stat::Mode::S_IRWXU).expect("Failed to create FIFO");
}

/// A Python project whose `.venv/pyvenv.cfg` is a writer-less FIFO, beside a
/// regular non-executable `.venv/bin/python`.
#[cfg(unix)]
fn write_fifo_venv_project(project: &Path) {
    fs::create_dir_all(project.join(".venv/bin")).expect("Failed to create .venv/bin");
    setup_python_project(project);
    fs::write(project.join("pyproject.toml"), "[project]\nname = \"x\"\n")
        .expect("Failed to write pyproject.toml");
    fs::write(project.join(".venv/bin/python"), "").expect("Failed to write python");
    make_fifo(&project.join(".venv/pyvenv.cfg"));
}

/// A git repository whose only Python evidence is an extensionless shebang
/// script, so detection must read it, beside an untracked extensionless FIFO.
#[cfg(unix)]
fn write_fifo_git_repo(root: &Path) {
    fs::create_dir_all(root).expect("Failed to create repo dir");
    git_init(root);
    fs::write(root.join("tool"), "#!/usr/bin/env python3\nprint('hi')\n")
        .expect("Failed to write tool");
    make_fifo(&root.join("untracked-pipe"));
}

// A writer-less FIFO in repository metadata or content must not stall
// detection: the reads fail fast and the regular files still count.
#[cfg(unix)]
#[test]
fn test_fifo_repository_files_do_not_hang_oneshot() {
    let (home_env, config_path) = setup_test_env_with_detection(DetectionMode::Content);

    let control = home_env.path().join("regular-venv");
    write_fifo_venv_project(&control);
    fs::remove_file(control.join(".venv/pyvenv.cfg")).expect("Failed to remove FIFO");
    fs::write(control.join(".venv/pyvenv.cfg"), "version = 3.11.9\n").expect("write cfg");
    let json = oneshot_lang_json(&control, &home_env, &config_path);
    assert_eq!(
        python_entry(&json).get("version").and_then(|v| v.as_str()),
        Some("3.11.9"),
        "a regular pyvenv.cfg still supplies the version: {json}"
    );

    let fifo_venv = home_env.path().join("fifo-venv");
    write_fifo_venv_project(&fifo_venv);
    oneshot_lang_json(&fifo_venv, &home_env, &config_path);

    let fifo_repo = home_env.path().join("fifo-repo");
    write_fifo_git_repo(&fifo_repo);
    let fifo_repo_json = oneshot_lang_json(&fifo_repo, &home_env, &config_path);
    python_entry(&fifo_repo_json);
}

/// Send one newline-framed request and return the reply line.
#[cfg(unix)]
fn ipc_request(socket_path: &Path, request: &str) -> String {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).expect("connect to agent");
    stream
        .set_read_timeout(Some(
            OPERATION_BUDGET
                .checked_mul(2)
                .expect("read timeout fits in Duration"),
        ))
        .expect("set read timeout");
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .expect("send request");
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .expect("read response");
    response
}

// The daemon answers for FIFO-bearing repositories within its budget (a
// stuck detection job would surface as a timeout error) and keeps answering.
#[cfg(unix)]
#[test]
fn test_daemon_answers_for_fifo_repositories() {
    let (home_env, config_path) = setup_test_env_with_detection(DetectionMode::Content);
    let daemon = RunningAgent::start(&home_env, &config_path);

    let repo = home_env.path().join("fifo-repo");
    write_fifo_git_repo(&repo);
    write_fifo_venv_project(&repo);
    let request = format!(
        r#"{{"op":"lang","cwd":"{}"}}"#,
        repo.display().to_string().replace('"', "\\\"")
    );
    for attempt in ["cold", "cached"] {
        let response = ipc_request(&daemon.socket_path, &request);
        let json: serde_json::Value = serde_json::from_str(&response)
            .unwrap_or_else(|e| panic!("{attempt} reply is not JSON ({e}): {response:?}"));
        python_entry(&json);
    }
}
