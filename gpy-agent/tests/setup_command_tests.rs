//! Integration tests for `gpy setup`: the per-shell install-and-wire step that
//! Homebrew, source builds and release archives share with the curl installer.
//!
//! Drives the real `gpy` binary against an isolated HOME with `--from` at the
//! repo's shell trees, so the shipped layout is under test.

#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, block_count, repo_root, setup};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

#[test]
fn zsh_setup_installs_files_and_wires_zshrc_once() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    let dir = home.path().join(".config/gpy/zsh");
    assert!(dir.join("gpy.zsh").is_file());
    assert!(dir.join("core/init.zsh").is_file());
    assert!(dir.join("completions/_gpy-dynamic").is_file());
    let structural = fs::read_to_string(dir.join("completions/_gpy")).unwrap();
    assert!(structural.contains("#compdef gpy"), "{structural}");

    let rc = home.path().join(".zshrc");
    assert_eq!(block_count(&rc), 1, "re-running must not stack blocks");
    let text = fs::read_to_string(&rc).unwrap();
    assert!(text.contains(&format!("source \"{}\"", dir.join("gpy.zsh").display())));
}

#[test]
fn bash_setup_wires_bashrc_and_a_login_loader() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    let dir = home.path().join(".config/gpy/bash");
    assert!(dir.join("gpy.bash").is_file());
    assert!(dir.join("completions/gpy.bash").is_file());
    assert_eq!(block_count(&home.path().join(".bashrc")), 1);
    let profile = fs::read_to_string(home.path().join(".profile")).unwrap();
    assert!(profile.contains(". \"$HOME/.bashrc\""), "{profile}");
}

/// A `.bashrc` that counts its runs, then stops (as most do) in a
/// non-interactive shell, before GPY's block.
const COUNTING_BASHRC: &str = "GPY_TEST_BASHRC_RUNS=$((${GPY_TEST_BASHRC_RUNS:-0} + 1))\ncase $- in *i*) ;; *) return ;; esac\n";

/// How many times a real Bash login shell in `home` ran `~/.bashrc`.
fn bashrc_runs_in_login_shell(home: &Path) -> String {
    let output = Command::new("bash")
        .args(["--login", "-c", "printf %s \"${GPY_TEST_BASHRC_RUNS:-0}\""])
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn bash");
    assert!(
        output.status.success(),
        "bash --login failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn bash_setup_wires_a_login_file_that_only_mentions_bashrc_in_comments() {
    for (name, text) in [
        (
            ".profile",
            "# .bashrc is intentionally not sourced here yet\n",
        ),
        (".bash_profile", "# [ -f ~/.bashrc ] && . ~/.bashrc\n"),
    ] {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".bashrc"), COUNTING_BASHRC).unwrap();
        let login = home.path().join(name);
        fs::write(&login, text).unwrap();

        assert_ok(&setup(home.path(), &["--shell", "bash"]));
        assert_ok(&setup(home.path(), &["--shell", "bash"]));

        assert!(fs::read_to_string(&login).unwrap().starts_with(text));
        assert_eq!(
            block_count(&login),
            1,
            "{name}: re-running must not stack loaders"
        );
        assert_eq!(bashrc_runs_in_login_shell(home.path()), "1", "{name}");
    }
}

#[test]
fn bash_setup_wires_the_login_file_bash_reads_first() {
    let home = TempDir::new().unwrap();
    fs::write(home.path().join(".bashrc"), COUNTING_BASHRC).unwrap();
    let bash_login = home.path().join(".bash_login");
    fs::write(&bash_login, "# [ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();
    let profile = home.path().join(".profile");
    fs::write(&profile, "[ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();

    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    assert_eq!(block_count(&bash_login), 1);
    assert_eq!(
        fs::read_to_string(&profile).unwrap(),
        "[ -f ~/.bashrc ] && . ~/.bashrc\n"
    );
    assert_eq!(bashrc_runs_in_login_shell(home.path()), "1");
}

#[test]
fn bash_setup_leaves_a_login_file_that_already_loads_bashrc() {
    for loader in [
        "[ -r ~/.bashrc ] && . ~/.bashrc\n",
        "if [ -f \"$HOME/.bashrc\" ]; then\n    . \"$HOME/.bashrc\"\nfi\n",
        "# Load the interactive config.\ntest -f ~/.bashrc && source ~/.bashrc\n",
    ] {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".bashrc"), COUNTING_BASHRC).unwrap();
        let login = home.path().join(".bash_profile");
        fs::write(&login, loader).unwrap();

        assert_ok(&setup(home.path(), &["--shell", "bash"]));

        assert_eq!(fs::read_to_string(&login).unwrap(), loader);
        assert!(!home.path().join(".profile").exists());
        assert_eq!(bashrc_runs_in_login_shell(home.path()), "1", "{loader}");
    }
}

#[test]
fn fish_setup_links_the_prompt_and_backs_up_an_existing_one() {
    let home = TempDir::new().unwrap();
    let functions = home.path().join(".config/fish/functions");
    fs::create_dir_all(&functions).unwrap();
    fs::write(
        functions.join("fish_prompt.fish"),
        "function fish_prompt; end\n",
    )
    .unwrap();

    assert_ok(&setup(home.path(), &["--shell", "fish"]));
    assert_ok(&setup(home.path(), &["--shell", "fish"]));

    let dir = home.path().join(".config/fish/gpy");
    assert_eq!(
        fs::read_link(functions.join("fish_prompt.fish")).unwrap(),
        dir.join("functions/fish_prompt.fish")
    );
    let backups: Vec<_> = fs::read_dir(&functions)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".backup."))
        .collect();
    assert_eq!(backups.len(), 1, "only the foreign prompt is backed up");
    // The uninstallers restore only `.backup.YYYYMMDD_HHMMSS`.
    let name = backups
        .first()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let stamp = name.strip_prefix("fish_prompt.fish.backup.").unwrap();
    let (date, time) = stamp.split_once('_').unwrap();
    assert!(
        date.len() == 8 && time.len() == 6,
        "unexpected backup stamp in {name}"
    );
    assert!(stamp.chars().all(|c| c.is_ascii_digit() || c == '_'));

    assert!(
        !home.path().join(".config/fish/gpy/completions").exists(),
        "fish completions live in fish/completions, not the gpy tree"
    );
    let completions =
        fs::read_to_string(home.path().join(".config/fish/completions/gpy.fish")).unwrap();
    assert!(completions.contains("complete -c gpy"), "{completions}");
    assert_eq!(
        block_count(&home.path().join(".config/fish/config.fish")),
        1
    );
}

#[test]
fn shell_comes_from_the_login_shell_when_no_flag_is_given() {
    let home = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--non-interactive", "--from"])
        .arg(repo_root())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("SHELL", "/usr/local/bin/zsh")
        .env("GPY_NERD_FONT", "0")
        .output()
        .unwrap();
    assert_ok(&output);
    assert!(home.path().join(".config/gpy/zsh/gpy.zsh").is_file());
}

#[test]
fn unsupported_login_shell_fails_with_a_pointer_to_the_flag() {
    let home = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--non-interactive", "--from"])
        .arg(repo_root())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("SHELL", "/bin/tcsh")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--shell"));
    assert!(!home.path().join(".config").exists(), "nothing was written");
}

#[test]
fn missing_script_tree_fails_before_touching_any_rc_file() {
    let home = TempDir::new().unwrap();
    let empty = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--shell", "zsh", "--non-interactive", "--from"])
        .arg(empty.path())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!home.path().join(".zshrc").exists());
}

#[test]
fn re_running_replaces_files_a_package_manager_left_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let script = home.path().join(".config/gpy/zsh/gpy.zsh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o444)).unwrap();

    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
}

#[test]
fn re_running_removes_scripts_a_newer_release_no_longer_ships() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let stale = home.path().join(".config/gpy/zsh/segments/retired.zsh");
    fs::write(&stale, "retired\n").unwrap();

    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    assert!(!stale.exists(), "a dropped script must not survive setup");
    assert!(home.path().join(".config/gpy/zsh/segments").is_dir());
}

/// `<home>/.config/gpy/themes`, created.
fn themes_dir(home: &Path) -> PathBuf {
    let dir = home.join(".config/gpy/themes");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The bytes of the builtin theme `name` this build embeds.
fn current_builtin(name: &str) -> Vec<u8> {
    fs::read(repo_root().join(format!("config/themes/{name}.toml"))).unwrap()
}

/// Stale copies of builtin themes (from an older release or the current one)
/// shadow every later fix, so setup moves them aside and says so; an edited
/// copy is the user's and stays. A second run has nothing left to move.
#[test]
fn setup_retires_unedited_builtin_theme_copies_and_keeps_edited_ones() {
    let home = TempDir::new().unwrap();
    let themes = themes_dir(home.path());
    let old_text = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/released_themes/text-v0.6.0.toml"),
    )
    .unwrap();
    assert_ne!(
        old_text,
        current_builtin("text"),
        "fixture must be an older release"
    );
    fs::write(themes.join("text.toml"), &old_text).unwrap();
    fs::write(themes.join("starship.toml"), current_builtin("starship")).unwrap();
    let mut edited = current_builtin("default");
    edited.extend_from_slice(b"# mine\n");
    fs::write(themes.join("default.toml"), &edited).unwrap();

    let first = setup(home.path(), &["--shell", "zsh"]);
    assert_ok(&first);
    let stdout = String::from_utf8_lossy(&first.stdout);
    for name in ["text", "starship"] {
        assert!(
            !themes.join(format!("{name}.toml")).exists(),
            "{name}.toml should be retired"
        );
        assert!(
            stdout.contains(&format!(
                "Retired {}",
                themes.join(format!("{name}.toml")).display()
            )),
            "setup must report the move of {name}.toml: {stdout}"
        );
    }
    assert_eq!(fs::read(themes.join("text.toml.bak")).unwrap(), old_text);
    assert_eq!(
        fs::read(themes.join("starship.toml.bak")).unwrap(),
        current_builtin("starship")
    );
    assert_eq!(fs::read(themes.join("default.toml")).unwrap(), edited);
    assert!(
        stdout.contains("Kept your theme files that override builtins")
            && stdout.contains("default.toml"),
        "setup must mention the kept copy: {stdout}"
    );

    let second = setup(home.path(), &["--shell", "zsh"]);
    assert_ok(&second);
    let rerun_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        !rerun_stdout.contains("Retired"),
        "nothing is left to retire: {rerun_stdout}"
    );
    assert!(!themes.join("text.toml.bak.1").exists());
    assert_eq!(fs::read(themes.join("text.toml.bak")).unwrap(), old_text);
    assert_eq!(fs::read(themes.join("default.toml")).unwrap(), edited);
}

#[test]
fn setup_keeps_unedited_theme_copies_when_retirement_is_off() {
    let home = TempDir::new().unwrap();
    let themes = themes_dir(home.path());
    fs::write(
        home.path().join(".config/gpy/config.toml"),
        "[ui]\nretire_unmodified_themes = false\n",
    )
    .unwrap();
    fs::write(themes.join("starship.toml"), current_builtin("starship")).unwrap();

    let output = setup(home.path(), &["--shell", "zsh"]);
    assert_ok(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Retired"), "{stdout}");
    assert_eq!(
        fs::read(themes.join("starship.toml")).unwrap(),
        current_builtin("starship")
    );
    assert!(!themes.join("starship.toml.bak").exists());
}

/// A wizard-style overlay of a builtin, with a stale key that fails to load.
const BROKEN_OVERLAY: &str = "extends = \"starship\"\n\n[segments.username]\nformat = \"$user\"\n";

/// `setup` keeps an overlay of a builtin but lists it apart from a full copy.
#[test]
fn setup_lists_kept_overlays_apart_from_full_copies() {
    let home = TempDir::new().unwrap();
    let themes = themes_dir(home.path());
    fs::write(
        themes.join("starship.toml"),
        "extends = \"starship\"\n\n[segments.clock]\ntime_format = \"24\"\n",
    )
    .unwrap();
    let mut edited = current_builtin("default");
    edited.extend_from_slice(b"# mine\n");
    fs::write(themes.join("default.toml"), &edited).unwrap();

    let output = setup(home.path(), &["--shell", "zsh"]);
    assert_ok(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let full_line = stdout
        .lines()
        .find(|line| line.starts_with("Kept your theme files that override builtins"))
        .unwrap_or_else(|| panic!("full copies are listed: {stdout}"));
    let overlay_line = stdout
        .lines()
        .find(|line| line.starts_with("Kept your theme overlays of builtins"))
        .unwrap_or_else(|| panic!("overlays are listed: {stdout}"));
    assert!(
        full_line.contains("default.toml") && !full_line.contains("starship.toml"),
        "{full_line}"
    );
    assert!(
        overlay_line.contains("starship.toml") && !overlay_line.contains("default.toml"),
        "{overlay_line}"
    );
    assert!(themes.join("starship.toml").exists());
}

/// Doctor lists overlays apart from full copies, suggests `rebase` only for a
/// full copy, and gives a failing self-named overlay its chain error, not the
/// "older GPY" fork hint.
#[test]
fn doctor_separates_overlays_from_full_copies_and_never_calls_an_overlay_a_fork() {
    let home = TempDir::new().unwrap();
    let themes = themes_dir(home.path());
    fs::write(
        home.path().join(".config/gpy/config.toml"),
        "[ui]\ntheme = \"text\"\n",
    )
    .unwrap();
    fs::write(themes.join("starship.toml"), BROKEN_OVERLAY).unwrap();
    let mut edited = current_builtin("default");
    edited.extend_from_slice(b"# mine\n");
    fs::write(themes.join("default.toml"), &edited).unwrap();

    let report = doctor(home.path());

    let overlays = report.find("Overlays (").expect("overlay heading");
    let copies = report.find("Full copies (").expect("full copy heading");
    let starship = report
        .find("Checking override 'starship'")
        .expect("starship");
    let default = report.find("Checking override 'default'").expect("default");
    assert!(
        overlays < starship && starship < copies && copies < default,
        "{report}"
    );
    assert!(report.contains("gpy theme rebase default"), "{report}");
    assert!(!report.contains("gpy theme rebase starship"), "{report}");
    assert!(
        report.contains("segments.username.format") && report.contains("chain:"),
        "the overlay's chain error is reported: {report}"
    );
    let overlay_section = &report[starship..copies];
    assert!(!overlay_section.contains("older GPY"), "{overlay_section}");
}

fn doctor(home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .arg("doctor")
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn gpy doctor");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// An upgrade runs setup for one shell. A shell set up earlier must get the
/// new scripts too, without its rc file being touched, and `gpy doctor` must
/// name a shell an older gpy left behind.
#[test]
fn setup_refreshes_shells_set_up_earlier_and_doctor_flags_stale_ones() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    // What an older gpy left: different scripts and its own version stamp.
    let bash_dir = home.path().join(".config/gpy/bash");
    let core = bash_dir.join("core/ipc.bash");
    fs::write(&core, "# scripts from an older gpy\n").unwrap();
    fs::write(bash_dir.join(".gpy-version"), "0.0.1\n").unwrap();
    let bashrc = fs::read_to_string(home.path().join(".bashrc")).unwrap();
    let report = doctor(home.path());
    assert!(
        report.contains("Checking bash integration... ")
            && report.contains("installed by gpy 0.0.1"),
        "{report}"
    );
    assert!(report.contains("gpy setup --shell bash"), "{report}");

    let output = setup(home.path(), &["--shell", "zsh"]);
    assert_ok(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Refreshed bash files"), "{stdout}");
    assert_eq!(
        fs::read_to_string(&core).unwrap(),
        fs::read_to_string(repo_root().join("bash/core/ipc.bash")).unwrap(),
        "bash scripts refreshed"
    );
    assert_eq!(
        fs::read_to_string(bash_dir.join(".gpy-version"))
            .unwrap()
            .trim(),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        fs::read_to_string(home.path().join(".bashrc")).unwrap(),
        bashrc,
        "the other shell's rc file is left alone"
    );
    assert!(
        !home.path().join(".config/fish").exists(),
        "a shell never set up is not installed"
    );
    let refreshed_report = doctor(home.path());
    assert!(
        !refreshed_report.contains("installed by"),
        "{refreshed_report}"
    );
}

/// The installers run setup with a temporary `--from`. The scripts must be
/// kept beside the binary, so a later bare `gpy setup` finds them.
#[test]
fn installer_style_setup_keeps_the_scripts_for_a_bare_rerun() {
    let home = TempDir::new().unwrap();
    let prefix = TempDir::new().unwrap();
    let bin = prefix.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let gpy = bin.join("gpy");
    fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).unwrap();
    let run = |extra: &[&std::ffi::OsStr]| {
        Command::new(&gpy)
            .args(["setup", "--non-interactive", "--shell", "zsh"])
            .args(extra)
            .current_dir(home.path())
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("GPY_NERD_FONT", "0")
            .output()
            .expect("failed to spawn gpy setup")
    };

    assert_ok(&run(&["--from".as_ref(), repo_root().as_os_str()]));
    assert!(prefix.path().join("share/gpy/zsh/gpy.zsh").is_file());
    assert!(
        prefix
            .path()
            .join("share/gpy/fish/conf.d/gpy_init.fish")
            .is_file()
    );
    assert_ok(&run(&[]));
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for dirent in fs::read_dir(from).unwrap() {
        let entry = dirent.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn fish_setup_fails_when_the_dynamic_completion_glue_is_missing() {
    let home = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    copy_dir(&repo_root().join("fish"), &source.path().join("fish"));
    fs::remove_file(source.path().join("fish/completions/gpy-dynamic.fish")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--shell", "fish", "--non-interactive", "--from"])
        .arg(source.path())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_NERD_FONT", "0")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("gpy-dynamic.fish"), "{stderr}");
    assert!(
        !home
            .path()
            .join(".config/fish/completions/gpy.fish")
            .exists(),
        "no completions without the glue"
    );
}

#[test]
fn doctor_inspects_a_plugin_segment_and_ignores_comments() {
    let home = TempDir::new().unwrap();
    let config = home.path().join(".config/gpy");
    let plugin = config.join("plugins/demo-tools");
    fs::create_dir_all(plugin.join("segments")).unwrap();
    fs::write(
        plugin.join("plugin.toml"),
        "id = \"demo-tools\"\nname = \"Demo\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
         entry_type = \"file\"\nprovided_segments = [\"demo_status\"]\n",
    )
    .unwrap();
    fs::write(
        plugin.join("segments/demo_status.fish"),
        "# no network here: curl, wget\nset node_count 1\n",
    )
    .unwrap();
    fs::write(
        config.join("config.toml"),
        "[ui]\nenabled_segments = [\"demo_status\"]\n",
    )
    .unwrap();

    let report = doctor(home.path());

    assert!(
        report.contains("- demo_status (plugin: demo-tools"),
        "{report}"
    );
    assert!(
        report.contains("heuristic: no obvious slow patterns detected"),
        "{report}"
    );
}
