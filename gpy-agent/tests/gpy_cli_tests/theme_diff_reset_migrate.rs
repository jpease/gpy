use crate::cli_harness::{self, CliTestEnv};
#[cfg(unix)]
use crate::fixtures;
use std::fs;

// `gpy theme diff|reset|migrate`: user copies of builtin themes.

const BUILTIN_STARSHIP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/starship.toml"
));

/// The current starship theme with the keys 0.6 accepted on `username` and
/// `hostname` (since removed) put back, as an old `gpy theme save` left it.
fn stale_starship_copy() -> String {
    let stale = BUILTIN_STARSHIP
        .replacen(
            "[segments.username]\n",
            "[segments.username]\nformat = \"[$username](bold red) in \"\nuser_format = \"[$username](bold yellow) in \"\nshow_always = true\n",
            1,
        )
        .replacen(
            "[segments.hostname]\n",
            "[segments.hostname]\nformat = \"[$symbol$hostname](bold dimmed green) in \"\nshow_always = true\ntrim_at = \"-\"\nicon = \"🌐 \"\n",
            1,
        )
        .replacen("ssh_icon = \"🌐 \"\n", "", 1);
    for removed_key in ["user_format = ", "trim_at = \"-\"", "\nicon = \"🌐 \""] {
        assert!(stale.contains(removed_key), "fixture lost {removed_key}");
    }
    assert!(!stale.contains("ssh_icon"), "fixture kept ssh_icon");
    stale
}

fn write_theme_file(env: &CliTestEnv, name: &str, content: &str) -> std::path::PathBuf {
    fs::create_dir_all(env.themes_dir()).expect("Failed to create themes dir");
    let path = env.themes_dir().join(format!("{name}.toml"));
    fs::write(&path, content).expect("Failed to write theme file");
    path
}

#[test]
fn test_theme_diff_shows_how_a_stale_copy_differs_from_the_builtin() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let path = write_theme_file(&env, "starship", &stale_starship_copy());

    let output = env
        .run_gpy(&["theme", "diff", "starship"])
        .expect("Failed to run gpy theme diff");

    output.assert_success("gpy theme diff starship");
    let stdout = output.stdout;
    assert!(
        stdout.contains("--- builtin:starship (embedded)")
            && stdout.contains(&format!("+++ {}", path.display())),
        "{stdout}"
    );
    for expected in [
        "\n+show_always = true\n",
        "\n+trim_at = \"-\"\n",
        "\n+icon = \"🌐 \"\n",
        "\n-ssh_icon = \"🌐 \"\n",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in {stdout}"
        );
    }
}

#[test]
fn test_theme_diff_reports_an_identical_copy() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_theme_file(&env, "starship", BUILTIN_STARSHIP);

    let output = env
        .run_gpy(&["theme", "diff", "starship"])
        .expect("Failed to run gpy theme diff");

    output.assert_success("gpy theme diff starship");
    assert!(
        output.stdout.contains("identical to the builtin"),
        "{output:?}"
    );
    assert!(!output.stdout.contains("@@"), "{output:?}");
}

#[test]
fn test_theme_diff_without_a_copy_says_so() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["theme", "diff", "starship"])
        .expect("Failed to run gpy theme diff");

    output.assert_success("gpy theme diff starship");
    assert!(
        output.stdout.contains("No user copy of 'starship'"),
        "{output:?}"
    );
}

#[test]
fn test_theme_diff_and_reset_reject_non_builtin_themes() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let custom = write_theme_file(&env, "my-own", &stale_starship_copy());

    for verb in ["diff", "reset"] {
        let output = env
            .run_gpy(&["theme", verb, "my-own"])
            .expect("Failed to run gpy theme");
        assert_ne!(output.exit_code, 0_i32, "{verb}: {output:?}");
        assert!(
            output.stderr.contains("not a builtin theme") && output.stderr.contains("starship"),
            "{verb}: {output:?}"
        );
    }
    assert_eq!(
        fs::read_to_string(&custom).expect("Failed to read custom theme"),
        stale_starship_copy(),
        "a non-builtin theme must never be moved aside"
    );
    assert!(!custom.with_extension("toml.bak").exists());
}

/// Stands in for an agent that accepts connections but never answers, so an
/// attempted reload prints `Agent not reloaded:` and a skipped one prints
/// nothing. Native Windows never runs an agent, so there it only runs the
/// command.
#[cfg(unix)]
struct ReloadProbe {
    _dir: tempfile::TempDir,
    _listener: std::os::unix::net::UnixListener,
    socket: std::path::PathBuf,
}

#[cfg(unix)]
impl ReloadProbe {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("probe-")
            .tempdir_in(fixtures::gpy_test_root().expect("GPY test root"))
            .expect("socket tempdir under the test root");
        let socket = dir.path().join("gpy.sock");
        let listener = fixtures::bind_silent_socket(&socket);
        Self {
            _dir: dir,
            _listener: listener,
            socket,
        }
    }

    fn run_gpy(&self, env: &CliTestEnv, args: &[&str]) -> cli_harness::CliCommandResult {
        env.run_gpy_with_env(args, &[("GPY_AGENT_SOCKET_PATH", self.socket.as_os_str())])
            .expect("Failed to run gpy")
    }

    #[expect(clippy::unused_self, reason = "mirrors the non-Unix probe")]
    fn assert_reload_attempted(
        &self,
        output: &cli_harness::CliCommandResult,
        attempted: bool,
        why: &str,
    ) {
        assert_eq!(
            output.stdout.contains("Agent not reloaded:"),
            attempted,
            "{why}: {output:?}"
        );
    }
}

#[cfg(not(unix))]
struct ReloadProbe;

#[cfg(not(unix))]
impl ReloadProbe {
    const fn new() -> Self {
        Self
    }

    #[expect(clippy::unused_self, reason = "mirrors the Unix probe")]
    fn run_gpy(&self, env: &CliTestEnv, args: &[&str]) -> cli_harness::CliCommandResult {
        env.run_gpy(args).expect("Failed to run gpy")
    }

    #[expect(clippy::unused_self, reason = "mirrors the Unix probe")]
    const fn assert_reload_attempted(
        &self,
        _output: &cli_harness::CliCommandResult,
        _attempted: bool,
        _why: &str,
    ) {
    }
}

#[test]
fn test_theme_reset_moves_the_copy_aside_so_the_builtin_takes_over() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let stale = stale_starship_copy();
    let path = write_theme_file(&env, "starship", &stale);

    let probe = ReloadProbe::new();
    let output = probe.run_gpy(&env, &["theme", "reset", "starship"]);

    output.assert_success("gpy theme reset starship");
    let backup = env.themes_dir().join("starship.toml.bak");
    assert!(!path.exists(), "the copy must be moved, not left in place");
    assert_eq!(
        fs::read_to_string(&backup).expect("Failed to read backup"),
        stale
    );
    assert!(
        output.stdout.contains(&backup.display().to_string()),
        "{output:?}"
    );
    let diff = env
        .run_gpy(&["theme", "diff", "starship"])
        .expect("Failed to run gpy theme diff");
    assert!(diff.stdout.contains("No user copy"), "{diff:?}");
    probe.assert_reload_attempted(
        &output,
        false,
        "starship is not the active theme, so no reload is attempted",
    );
}

#[test]
fn test_theme_reset_keeps_older_backups() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_theme_file(&env, "text", "# first\n");
    env.run_gpy(&["theme", "reset", "text"])
        .expect("Failed to run gpy theme reset")
        .assert_success("first reset");
    write_theme_file(&env, "text", "# second\n");

    env.run_gpy(&["theme", "reset", "text"])
        .expect("Failed to run gpy theme reset")
        .assert_success("second reset");

    let read = |file: &str| fs::read_to_string(env.themes_dir().join(file)).expect("backup exists");
    assert_eq!(read("text.toml.bak"), "# first\n");
    assert_eq!(read("text.toml.bak.1"), "# second\n");
}

#[test]
fn test_theme_reset_of_the_active_theme_attempts_a_reload() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_theme_file(&env, "default", "# stale\n");

    let probe = ReloadProbe::new();
    let output = probe.run_gpy(&env, &["theme", "reset", "default"]);

    output.assert_success("gpy theme reset default");
    probe.assert_reload_attempted(
        &output,
        true,
        "the active theme changed, so a reload is attempted",
    );
    assert!(env.themes_dir().join("default.toml.bak").is_file());
}

#[test]
fn test_theme_reset_without_a_copy_is_an_error() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["theme", "reset", "starship"])
        .expect("Failed to run gpy theme reset");

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    assert!(
        output
            .stderr
            .contains("No user copy of builtin theme 'starship'"),
        "{output:?}"
    );
}

fn plugin_setting(env: &CliTestEnv, segment: &str, key: &str) -> Option<toml::Value> {
    let text = fs::read_to_string(env.config_path()).expect("Failed to read config.toml");
    let config: toml::Table = toml::from_str(&text).expect("config.toml parses");
    config.get("plugin")?.get(segment)?.get(key).cloned()
}

#[test]
fn test_theme_migrate_turns_a_0_6_copy_into_one_that_loads() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let stale = stale_starship_copy();
    let path = write_theme_file(&env, "starship", &stale);
    let config_before = fs::read_to_string(env.config_path()).expect("Failed to read config");
    let broken = env
        .run_gpy(&["theme", "validate", "starship"])
        .expect("Failed to run gpy theme validate");
    assert_ne!(
        broken.exit_code, 0_i32,
        "the 0.6-era copy must fail before migrating: {broken:?}"
    );

    let output = env
        .run_gpy(&["theme", "migrate", "starship"])
        .expect("Failed to run gpy theme migrate");

    output.assert_success("gpy theme migrate starship");
    let migrated = fs::read_to_string(&path).expect("Failed to read migrated theme");
    let segments: toml::Table = toml::from_str::<toml::Table>(&migrated)
        .expect("migrated theme parses")
        .remove("segments")
        .and_then(|value| value.try_into().ok())
        .expect("migrated theme has [segments]");
    let keys = |segment: &str| -> Vec<String> {
        let mut keys: Vec<String> = segments
            .get(segment)
            .and_then(toml::Value::as_table)
            .map(|table| table.keys().cloned().collect())
            .expect("segment table present");
        keys.sort();
        keys
    };
    assert_eq!(
        keys("username"),
        ["bg_color", "root_bg_color", "root_text_color", "text_color"]
    );
    assert_eq!(keys("hostname"), ["bg_color", "ssh_icon", "text_color"]);
    assert!(migrated.contains("ssh_icon = \"🌐 \""), "{migrated}");
    assert_eq!(
        plugin_setting(&env, "hostname", "show_always"),
        Some(toml::Value::Boolean(true))
    );
    assert_eq!(
        plugin_setting(&env, "hostname", "trim_at"),
        Some(toml::Value::String("-".to_owned()))
    );
    assert_eq!(
        plugin_setting(&env, "username", "show_always"),
        Some(toml::Value::Boolean(true))
    );
    assert_eq!(
        fs::read_to_string(env.themes_dir().join("starship.toml.bak")).expect("theme backup"),
        stale
    );
    assert_eq!(
        fs::read_to_string(env.config_dir().join("config.toml.bak")).expect("config backup"),
        config_before
    );
    for rewrite in [
        "removed `format`",
        "moved `trim_at`",
        "renamed `icon` to `ssh_icon`",
    ] {
        assert!(
            output.stdout.contains(rewrite),
            "missing {rewrite:?}: {output:?}"
        );
    }
    env.run_gpy(&["theme", "validate", "starship"])
        .expect("Failed to run gpy theme validate")
        .assert_success("the migrated theme loads");

    let again = env
        .run_gpy(&["theme", "migrate", "starship"])
        .expect("Failed to run gpy theme migrate");
    again.assert_success("second gpy theme migrate starship");
    assert!(again.stdout.contains("Nothing to migrate"), "{again:?}");
    assert_eq!(
        fs::read_to_string(&path).expect("Failed to read theme"),
        migrated
    );
    assert!(!env.themes_dir().join("starship.toml.bak.1").exists());
}

#[test]
fn test_theme_migrate_keeps_a_setting_already_in_config_toml() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config = fs::read_to_string(env.config_path()).expect("Failed to read config");
    env.write_config(&format!(
        "# keep me\n{config}\n[plugin.hostname]\nshow_always = false\n"
    ))
    .expect("Failed to write config");
    write_theme_file(&env, "starship", &stale_starship_copy());

    let output = env
        .run_gpy(&["theme", "migrate", "starship"])
        .expect("Failed to run gpy theme migrate");

    output.assert_success("gpy theme migrate starship");
    assert_eq!(
        plugin_setting(&env, "hostname", "show_always"),
        Some(toml::Value::Boolean(false)),
        "an existing config.toml value must win over the theme's"
    );
    assert_eq!(
        plugin_setting(&env, "hostname", "trim_at"),
        Some(toml::Value::String("-".to_owned()))
    );
    assert!(
        output
            .stdout
            .contains("kept plugin.hostname.show_always = false"),
        "{output:?}"
    );
    let config_after = fs::read_to_string(env.config_path()).expect("Failed to read config");
    assert!(config_after.starts_with("# keep me\n"), "{config_after}");
}

#[test]
fn test_theme_migrate_reports_a_theme_that_still_fails_and_keeps_the_backup() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let theme = "[ui]\nprompt_icon = \"❯\"\nprompt_color = \"not-a-color\"\n\n[segments.hostname]\nshow_always = true\n";
    write_theme_file(&env, "custom", theme);

    let output = env
        .run_gpy(&["theme", "migrate", "custom"])
        .expect("Failed to run gpy theme migrate");

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    let combined = format!("{}{}", output.stdout, output.stderr);
    assert!(
        combined.contains("Invalid color") && combined.contains("still does not validate"),
        "{combined}"
    );
    assert_eq!(
        fs::read_to_string(env.themes_dir().join("custom.toml.bak")).expect("backup"),
        theme
    );
}

#[test]
fn test_theme_migrate_without_a_theme_file_is_an_error() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["theme", "migrate", "starship"])
        .expect("Failed to run gpy theme migrate");

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    assert!(
        output.stderr.contains("No user theme file for 'starship'"),
        "{output:?}"
    );
}

// `gpy theme diff|reset|migrate` on overlays (ADR-0012).

const OVERLAY_DIRECTORY_FORMAT: &str =
    "extends = \"starship\"\n\n[segments.directory]\nformat = \"[OVL $path](bold cyan) \"\n";

fn combined(output: &cli_harness::CliCommandResult) -> String {
    format!("{}{}", output.stdout, output.stderr)
}

#[test]
fn test_theme_diff_of_an_overlay_shows_only_the_changed_keys() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let path = write_theme_file(&env, "mine", OVERLAY_DIRECTORY_FORMAT);

    let output = env
        .run_gpy(&["theme", "diff", "mine"])
        .expect("Failed to run gpy theme diff");

    output.assert_success("gpy theme diff mine");
    let stdout = output.stdout;
    assert!(
        stdout.contains("--- starship (builtin) (merged)")
            && stdout.contains(&format!("+++ {} (merged)", path.display())),
        "{stdout}"
    );
    assert!(
        stdout.contains("\n+segments.directory.format = \"[OVL $path](bold cyan) \""),
        "{stdout}"
    );
    let changes: Vec<&str> = stdout
        .lines()
        .filter(|line| {
            (line.starts_with('+') || line.starts_with('-'))
                && !line.starts_with("+++")
                && !line.starts_with("---")
        })
        .collect();
    assert!(!changes.is_empty(), "{stdout}");
    assert!(
        changes
            .iter()
            .all(|line| line.contains("segments.directory.format")),
        "only the changed key may appear: {stdout}"
    );
}

#[test]
fn test_theme_reset_of_an_overlay_reports_what_the_name_resolves_to() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let mine = write_theme_file(&env, "mine", OVERLAY_DIRECTORY_FORMAT);
    let starship = write_theme_file(&env, "starship", OVERLAY_DIRECTORY_FORMAT);

    let first = env
        .run_gpy(&["theme", "reset", "mine"])
        .expect("Failed to run gpy theme reset");
    first.assert_success("gpy theme reset mine");
    assert!(
        first.stdout.contains("No theme 'mine' remains"),
        "{first:?}"
    );
    assert!(!mine.exists() && env.themes_dir().join("mine.toml.bak").is_file());

    let second = env
        .run_gpy(&["theme", "reset", "starship"])
        .expect("Failed to run gpy theme reset");
    second.assert_success("gpy theme reset starship");
    assert!(
        second
            .stdout
            .contains("The builtin theme 'starship' is in use again"),
        "{second:?}"
    );
    assert!(!starship.exists());
}

#[test]
fn test_theme_reset_refuses_a_file_in_the_active_chain() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    env.write_config("[ui]\ntheme = \"mine2\"\n")
        .expect("Failed to write config");
    let mine = write_theme_file(&env, "mine", OVERLAY_DIRECTORY_FORMAT);
    write_theme_file(&env, "mine2", "extends = \"mine\"\n");

    let output = env
        .run_gpy(&["theme", "reset", "mine"])
        .expect("Failed to run gpy theme reset");

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    let text = combined(&output);
    assert!(
        text.contains("active theme 'mine2' would not load") && text.contains("gpy theme use"),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(&mine).expect("the overlay stays in place"),
        OVERLAY_DIRECTORY_FORMAT
    );
    assert!(!env.themes_dir().join("mine.toml.bak").exists());
}

#[test]
fn test_theme_reset_moves_a_chain_file_when_the_active_theme_still_loads() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    env.write_config("[ui]\ntheme = \"starship\"\n")
        .expect("Failed to write config");
    let path = write_theme_file(&env, "starship", OVERLAY_DIRECTORY_FORMAT);

    let probe = ReloadProbe::new();
    let output = probe.run_gpy(&env, &["theme", "reset", "starship"]);

    output.assert_success("gpy theme reset starship");
    assert!(!path.exists());
    probe.assert_reload_attempted(
        &output,
        true,
        "the file was in the active chain, so a reload is attempted",
    );
}

#[test]
fn test_theme_reset_does_not_refuse_when_the_active_theme_was_already_broken() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    env.write_config("[ui]\ntheme = \"broken\"\n")
        .expect("Failed to write config");
    write_theme_file(&env, "broken", "this is [not toml\n");
    let mine = write_theme_file(&env, "mine", OVERLAY_DIRECTORY_FORMAT);

    let probe = ReloadProbe::new();
    let output = probe.run_gpy(&env, &["theme", "reset", "mine"]);

    output.assert_success("gpy theme reset mine");
    assert!(!mine.exists());
    probe.assert_reload_attempted(
        &output,
        false,
        "the moved file is outside the active theme's chain",
    );
}

#[test]
fn test_theme_reset_warns_when_other_overlays_extend_the_moved_one() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    write_theme_file(&env, "mine", OVERLAY_DIRECTORY_FORMAT);
    write_theme_file(&env, "child", "extends = \"mine\"\n");

    let output = env
        .run_gpy(&["theme", "reset", "mine"])
        .expect("Failed to run gpy theme reset");

    output.assert_success("gpy theme reset mine");
    let text = combined(&output);
    assert!(text.contains("extend 'mine': child"), "{text}");
}

#[test]
fn test_theme_migrate_rewrites_and_drops_unset_paths_of_an_overlay() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let path = write_theme_file(
        &env,
        "mine",
        "extends = \"starship\"\nunset = [\"segments.hostname.icon\", \"segments.username.format\", \"segments.directory.format\"]\n",
    );

    let output = env
        .run_gpy(&["theme", "migrate", "mine"])
        .expect("Failed to run gpy theme migrate");

    output.assert_success("gpy theme migrate mine");
    let migrated: toml::Table =
        toml::from_str(&fs::read_to_string(&path).expect("Failed to read migrated overlay"))
            .expect("migrated overlay parses");
    let unset: Vec<&str> = migrated
        .get("unset")
        .and_then(toml::Value::as_array)
        .expect("unset is kept")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    assert_eq!(
        unset,
        ["segments.hostname.ssh_icon", "segments.directory.format"]
    );
    assert!(
        output.stdout.contains("renamed `segments.hostname.icon`")
            && output.stdout.contains("dropped `segments.username.format`"),
        "{output:?}"
    );
}

#[test]
fn test_theme_migrate_removes_an_unset_list_it_emptied() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let path = write_theme_file(
        &env,
        "mine",
        "extends = \"starship\"\nunset = [\"segments.username.format\"]\n",
    );

    env.run_gpy(&["theme", "migrate", "mine"])
        .expect("Failed to run gpy theme migrate")
        .assert_success("gpy theme migrate mine");

    let migrated = fs::read_to_string(&path).expect("Failed to read migrated overlay");
    assert!(!migrated.contains("unset"), "{migrated}");
    assert!(migrated.contains("extends = \"starship\""), "{migrated}");
}
