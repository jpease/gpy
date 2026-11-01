use crate::cli_harness::CliTestEnv;
use std::fs;

#[test]
fn test_enable_discovered_plugin_segment() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[agent]
enabled = true

[git]
enabled = true

[language]
enabled = true

[ui]
theme = "default"
enabled_segments = ["clock", "duration", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let plugin_dir = env.plugins_dir().join("demo");
    fs::create_dir_all(&plugin_dir).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "demo"
name = "Demo"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["k8s_tools"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let output = env
        .run_gpy(&["enable", "k8s_tools"])
        .expect("Failed to run gpy enable k8s_tools");

    assert!(
        output.exit_code == 0_i32,
        "Enable discovered plugin segment should succeed.\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("k8s_tools"),
        "Config should include plugin segment after enable"
    );
}

#[test]
fn plugin_list_marks_discovered_plugins_ready() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let plugin_dir = env.plugins_dir().join("demo");
    fs::create_dir_all(&plugin_dir).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "demo"
name = "Demo"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["k8s_tools"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let list = env
        .run_gpy(&["plugin", "list"])
        .expect("Failed to run gpy plugin list");
    list.assert_success("gpy plugin list");
    assert!(
        list.stdout.contains("\n- demo 0.1.0 [user] (ready)\n"),
        "Discovered plugin must be listed as ready.\nstdout:\n{}",
        list.stdout
    );
    assert!(
        list.stdout.contains(FISH_ONLY_LINE),
        "plugin list must say v1 plugin segments are Fish-only.\nstdout:\n{}",
        list.stdout
    );
}

#[test]
fn plugin_validate_says_plugin_segments_are_fish_only() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_demo_plugin(&env);
    let output = env
        .run_gpy(&["plugin", "validate", "demo"])
        .expect("Failed to run gpy plugin validate");
    output.assert_success("gpy plugin validate demo");
    assert!(
        output.stdout.contains(FISH_ONLY_LINE),
        "plugin validate must say v1 plugin segments are Fish-only.\nstdout:\n{}",
        output.stdout
    );
}

const FISH_ONLY_LINE: &str =
    "Plugin segments from v1 `.fish` files load in Fish only; Bash and Zsh prompts skip them.";

/// Plugin `demo` providing `k8s_tools`, with its segment file.
fn write_demo_plugin(env: &CliTestEnv) {
    let plugin_dir = env.plugins_dir().join("demo");
    fs::create_dir_all(plugin_dir.join("segments")).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "demo"
name = "Demo"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["k8s_tools"]
"#,
    )
    .expect("Failed to write plugin manifest");
    fs::write(plugin_dir.join("segments").join("k8s_tools.fish"), "")
        .expect("Failed to write segment file");
}

/// Script plugin `scripty` providing `ctx`; runs in every shell, so it is not Fish-only.
fn write_script_plugin(env: &CliTestEnv) {
    let plugin_dir = env.plugins_dir().join("scripty");
    fs::create_dir_all(plugin_dir.join("segments")).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "scripty"
name = "Scripty"
version = "0.1.0"
api_version = "v1"
entry_type = "script"
provided_segments = ["ctx"]
"#,
    )
    .expect("Failed to write plugin manifest");
    fs::write(
        plugin_dir.join("segments").join("ctx.toml"),
        "command = \"ctx.sh\"\n",
    )
    .expect("Failed to write script segment manifest");
    fs::write(plugin_dir.join("ctx.sh"), "echo ctx\n").expect("Failed to write script");
}

/// Mark an integration as installed the way `gpy setup` does: its rc file
/// (relative to `HOME`, or `XDG_CONFIG_HOME` for Fish) holds the GPY block.
fn install_integration(env: &CliTestEnv, rc_file: &str) {
    let path = if rc_file.starts_with("fish/") {
        env.xdg_config_home().join(rc_file)
    } else {
        env.root().join(rc_file)
    };
    fs::create_dir_all(path.parent().expect("rc file has a parent"))
        .expect("Failed to create rc file dir");
    fs::write(
        path,
        "# >>> gpy-init >>>\n# GPY shell integration\n# <<< gpy-init <<<\n",
    )
    .expect("Failed to write rc file");
}

fn enable_stdout(env: &CliTestEnv, segment: &str) -> String {
    let output = env
        .run_gpy(&["enable", segment])
        .expect("Failed to run gpy enable");
    output.assert_success("gpy enable");
    output.stdout
}

// Windows resolves the config home outside `XDG_CONFIG_HOME`, and has no Bash/Zsh integration.
#[cfg(unix)]
#[test]
fn enable_plugin_segment_notes_bash_and_zsh_skip_it() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_demo_plugin(&env);
    install_integration(&env, ".bashrc");
    install_integration(&env, "fish/config.fish");

    let stdout = enable_stdout(&env, "k8s_tools");
    assert!(
        stdout.contains("the bash prompt will skip it")
            && stdout.contains("gpy disable k8s_tools")
            && stdout.contains("use Fish"),
        "enable must warn that Bash skips a v1 plugin segment.\nstdout:\n{stdout}"
    );
    assert!(!stdout.contains("zsh"), "zsh is not installed:\n{stdout}");
}

#[test]
fn enable_plugin_segment_is_silent_without_bash_or_zsh() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_demo_plugin(&env);
    install_integration(&env, "fish/config.fish");

    let stdout = enable_stdout(&env, "k8s_tools");
    assert!(
        !stdout.contains("will skip it"),
        "Fish-only installs get no note.\nstdout:\n{stdout}"
    );
}

#[test]
fn enable_builtin_segment_is_silent_even_with_bash_and_zsh() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    install_integration(&env, ".bashrc");
    install_integration(&env, "gpy/zsh/gpy.zsh");

    let stdout = enable_stdout(&env, "clock");
    assert!(
        !stdout.contains("will skip it"),
        "built-in segments render in every shell.\nstdout:\n{stdout}"
    );
}

#[test]
fn script_plugin_gets_no_fish_only_note() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_script_plugin(&env);
    install_integration(&env, ".bashrc");

    let validate = env
        .run_gpy(&["plugin", "validate", "scripty"])
        .expect("Failed to run gpy plugin validate");
    validate.assert_success("gpy plugin validate scripty");
    let list = env
        .run_gpy(&["plugin", "list"])
        .expect("Failed to run gpy plugin list");
    list.assert_success("gpy plugin list");
    let enable = enable_stdout(&env, "ctx");

    for (command, stdout) in [
        ("validate", &validate.stdout),
        ("list", &list.stdout),
        ("enable", &enable),
    ] {
        assert!(
            !stdout.contains("Fish only") && !stdout.contains("will skip it"),
            "script plugins run in every shell; `{command}` must not say otherwise.\nstdout:\n{stdout}"
        );
    }
}

#[test]
fn test_plugin_new_creates_scaffold() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["plugin", "new", "demo-tools", "--segment", "demo_status"])
        .expect("Failed to run gpy plugin new");
    output.assert_success("gpy plugin new demo-tools --segment demo_status");

    let plugin_root = env.plugins_dir().join("demo-tools");
    let manifest_path = plugin_root.join("plugin.toml");
    let segment_manifest_path = plugin_root.join("segments").join("demo_status.toml");
    let script_path = plugin_root.join("bin").join("demo_status.sh");

    for path in [&manifest_path, &segment_manifest_path, &script_path] {
        assert!(path.is_file(), "{} should be created", path.display());
    }

    let manifest = fs::read_to_string(&manifest_path).expect("Failed to read plugin manifest");
    assert!(
        manifest.contains("id = \"demo-tools\""),
        "Manifest should include plugin id"
    );
    assert!(
        manifest.contains("entry_type = \"script\""),
        "Scaffold should be a script (G) plugin"
    );
    assert!(
        manifest.contains("provided_segments = [\"demo_status\"]"),
        "Manifest should include requested segment"
    );
    assert!(
        !output.stdout.contains("Fish"),
        "a script plugin runs in every shell.\nstdout:\n{}",
        output.stdout
    );
}

#[test]
fn test_plugin_validate_reports_missing_segment_file() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let plugin_dir = env.plugins_dir().join("broken");
    fs::create_dir_all(&plugin_dir).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "broken"
name = "Broken"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["broken_seg"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let output = env
        .run_gpy(&["plugin", "validate", &plugin_dir.display().to_string()])
        .expect("Failed to run gpy plugin validate");

    assert_ne!(
        output.exit_code, 0_i32,
        "Validation should fail when segment file is missing"
    );
    assert!(
        output.stderr.contains("missing segment file")
            || output.stdout.contains("missing segment file"),
        "Output should mention the missing segment file.\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn test_plugin_validate_accepts_bare_manifest_filename() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let plugin_dir = env.plugins_dir().join("bare");
    fs::create_dir_all(plugin_dir.join("segments")).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "bare"
name = "Bare"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["bare_seg"]
"#,
    )
    .expect("Failed to write plugin manifest");
    fs::write(plugin_dir.join("segments").join("bare_seg.fish"), "# seg\n")
        .expect("Failed to write segment file");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["plugin", "validate", "plugin.toml"])
        .current_dir(&plugin_dir)
        .env("HOME", env.root())
        .env("XDG_CONFIG_HOME", env.xdg_config_home())
        .env("XDG_RUNTIME_DIR", env.xdg_runtime_dir())
        .env("XDG_CACHE_HOME", env.xdg_cache_home())
        .output()
        .expect("Failed to run gpy plugin validate");

    assert!(
        output.status.success(),
        "Bare plugin.toml should validate from inside the plugin dir.\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_plugin_validate_rejects_misspelled_provided_segments() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let plugin_dir = env.plugins_dir().join("weather");
    fs::create_dir_all(plugin_dir.join("segments")).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "weather"
name = "Weather"
version = "1.0.0"
api_version = "v1"
entry_type = "file"
provided_segment = ["weather"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let output = env
        .run_gpy(&["plugin", "validate", &plugin_dir.display().to_string()])
        .expect("Failed to run gpy plugin validate");

    assert_ne!(
        output.exit_code, 0_i32,
        "Validation should fail when provided_segments is missing"
    );
    assert!(
        output.stderr.contains("provided_segments") || output.stdout.contains("provided_segments"),
        "Output should name provided_segments.\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );

    let list = env
        .run_gpy(&["plugin", "list"])
        .expect("Failed to run gpy plugin list");
    assert!(
        !list.stdout.contains("weather 1.0.0 [user] (ready)"),
        "Plugin without provided_segments must not be listed as ready.\nstdout:\n{}",
        list.stdout
    );
}

#[test]
fn test_plugin_new_twice_keeps_the_existing_scaffold() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    env.run_gpy(&["plugin", "new", "twice-tools"])
        .expect("Failed to run gpy plugin new")
        .assert_success("gpy plugin new twice-tools");
    let manifest = env.plugins_dir().join("twice-tools/plugin.toml");
    fs::write(&manifest, "# hand edited\n").expect("Failed to edit manifest");

    let second = env
        .run_gpy(&["plugin", "new", "twice-tools"])
        .expect("Failed to run gpy plugin new");

    assert_ne!(second.exit_code, 0_i32, "{second:?}");
    assert!(second.stderr.contains("already exists"), "{second:?}");
    assert_eq!(
        fs::read_to_string(&manifest).expect("Failed to read manifest"),
        "# hand edited\n",
        "an existing plugin must not be overwritten"
    );
}
