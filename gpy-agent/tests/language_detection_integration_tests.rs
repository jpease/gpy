//! Language detection across supported languages via CLI and IPC formats.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures

use gpy_agent::config::{Config, loader};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(unix)]
#[path = "common/child_guard.rs"]
mod child_guard;
#[cfg(unix)]
#[path = "common/wait.rs"]
mod wait;

fn setup_node_project(path: &Path) {
    fs::write(path.join("package.json"), r#"{"name": "test"}"#)
        .expect("Failed to write package.json");
    fs::write(path.join("index.js"), "console.log('hello');").expect("Failed to write index.js");
}

#[allow(clippy::missing_panics_doc)]
fn setup_python_project(path: &Path) {
    fs::write(path.join("requirements.txt"), "requests==2.28.0")
        .expect("Failed to write requirements.txt");
    fs::write(path.join("main.py"), "print('hello')").expect("Failed to write main.py");
}

#[allow(clippy::missing_panics_doc)]
fn setup_rust_project(path: &Path) {
    fs::write(path.join("Cargo.toml"), r#"[package]\nname = "test""#)
        .expect("Failed to write Cargo.toml");
    fs::write(path.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
}

#[allow(clippy::missing_panics_doc)]
fn setup_go_project(path: &Path) {
    fs::write(path.join("go.mod"), "module test").expect("Failed to write go.mod");
    fs::write(path.join("main.go"), "package main\nfunc main() {}")
        .expect("Failed to write main.go");
}

#[allow(clippy::missing_panics_doc)]
fn setup_swift_project(path: &Path) {
    fs::write(path.join("Package.swift"), "// swift-tools-version:5.0")
        .expect("Failed to write Package.swift");
    fs::write(path.join("main.swift"), "print(\"hello\")").expect("Failed to write main.swift");
}

#[allow(clippy::missing_panics_doc)]
fn setup_elixir_project(path: &Path) {
    fs::write(path.join("mix.exs"), "defmodule Test.MixProject do\nend")
        .expect("Failed to write mix.exs");
    fs::create_dir_all(path.join("lib")).expect("Failed to create lib directory");
    fs::write(path.join("lib/test.ex"), "defmodule Test do\nend").expect("Failed to write test.ex");
}

#[allow(clippy::missing_panics_doc)]
fn setup_erlang_project(path: &Path) {
    fs::write(path.join("rebar.config"), "{erl_opts, []}.").expect("Failed to write rebar.config");
    fs::create_dir_all(path.join("src")).expect("Failed to create src directory");
    fs::write(path.join("src/test.erl"), "-module(test).").expect("Failed to write test.erl");
}

#[allow(clippy::missing_panics_doc)]
fn setup_ruby_project(path: &Path) {
    fs::write(path.join("Gemfile"), "source 'https://rubygems.org'")
        .expect("Failed to write Gemfile");
    fs::write(path.join("app.rb"), "puts 'hello'").expect("Failed to write app.rb");
}

#[allow(clippy::missing_panics_doc)]
fn setup_java_project(path: &Path) {
    fs::write(path.join("pom.xml"), "<project></project>").expect("Failed to write pom.xml");
    fs::write(path.join("Main.java"), "public class Main {}").expect("Failed to write Main.java");
}

#[allow(clippy::missing_panics_doc)]
fn setup_fish_project(path: &Path) {
    fs::create_dir_all(path.join("functions")).expect("Failed to create functions directory");
    fs::write(path.join("functions/test.fish"), "function test\nend")
        .expect("Failed to write test.fish");
}

#[allow(clippy::missing_panics_doc)]
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
    loader::save_config(
        config_path.to_str().expect("config path utf-8"),
        &default_config,
        &default_config,
        &["ui.show_icons"],
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
    let (home_env, config_path) = setup_test_env();

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
    let (home_env, config_path) = setup_test_env();

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
    assert!(
        serde_json::from_str::<serde_json::Value>(&json_stdout).is_ok(),
        "JSON format should be valid JSON. Got: {json_stdout}"
    );

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

#[cfg(unix)]
#[test]
fn test_ipc_format_parameter() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let (home_env, config_path) = setup_test_env();
    let socket_dir = TempDir::new().expect("Failed to create socket dir");
    let socket_path = socket_dir.path().join("gpy.sock");

    // Registered before `start` so the daemon stops even if start fails.
    let mut stop_command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
    configure_command_env(&mut stop_command, &home_env, &config_path);
    let _daemon = child_guard::AgentStopGuard::new(stop_command, socket_path.clone());

    let mut start = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
    start.args(["start", "--socket"]).arg(&socket_path);
    configure_command_env(&mut start, &home_env, &config_path);
    let started = start.output().expect("Failed to execute gpy-agent start");
    assert!(
        started.status.success(),
        "agent failed to start: {}",
        String::from_utf8_lossy(&started.stderr)
    );

    assert!(
        wait::wait_until(Duration::from_secs(10), Duration::from_millis(25), || {
            UnixStream::connect(&socket_path).is_ok()
        }),
        "agent never accepted a connection on {}",
        socket_path.display()
    );

    let test_dir = create_language_test_dir("rust");
    let path = test_dir.path().to_string_lossy().to_string();

    let mut stream = UnixStream::connect(&socket_path).expect("connect to agent");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let request = format!(
        r#"{{"op":"lang","cwd":"{}","format":"ansi"}}"#,
        path.replace('"', "\\\"")
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .expect("send lang request");

    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .expect("read lang response");

    assert!(
        response.contains("\x1b["),
        "IPC ansi response should carry ANSI codes. Got: {response:?}"
    );
    assert!(
        !response.contains(r#""languages""#),
        "IPC should return ansi format, not JSON. Got: {response:?}"
    );
    assert!(
        response.contains("rust"),
        "the rendered segment should name the detected language. Got: {response:?}"
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

    let git_init = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("Failed to run git init");
    assert!(git_init.success(), "git init failed");

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

/// Run `oneshot lang --format json` at `cwd` and return the parsed output.
#[cfg(unix)]
fn oneshot_lang_json(cwd: &Path, home: &TempDir, config_path: &Path) -> serde_json::Value {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpy-agent"));
    command.args(["oneshot", "lang", "--cwd"]);
    command.arg(cwd);
    command.args(["--format", "json"]);
    configure_command_env(&mut command, home, config_path);
    let output = command.output().expect("Failed to execute gpy-agent");
    assert!(
        output.status.success(),
        "oneshot lang failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Failed to parse JSON")
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
        let languages = json
            .get("languages")
            .and_then(|v| v.as_array())
            .expect("languages should be an array");
        let python_entry = languages
            .iter()
            .find(|l| l.get("name").and_then(|n| n.as_str()) == Some("python"))
            .unwrap_or_else(|| panic!("python must still be detected: {json}"));
        assert!(
            python_entry
                .get("version")
                .is_none_or(serde_json::Value::is_null),
            "no usable pyvenv.cfg version means no version (pyvenv.cfg {cfg:?}): {json}"
        );
    }
}
