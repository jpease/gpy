//! Integration tests for where `gpy setup` installs: the release layout
//! (`fish/`, `bash/`, `zsh/`, `plugins/`) under `$XDG_DATA_HOME/gpy`, found
//! again as a source by a bare `gpy setup`, plus the migration away from the
//! old locations under `$XDG_CONFIG_HOME`.

#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, block_count, repo_root, setup};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn data(home: &Path) -> PathBuf {
    home.join(".local/share/gpy")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A `gpy` copied into `<prefix>/bin`, so the build checkout is not one of
/// its source candidates.
fn installed_gpy(prefix: &Path) -> PathBuf {
    let bin = prefix.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let gpy = bin.join("gpy");
    fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).unwrap();
    gpy
}

/// `gpy setup --non-interactive` from `program` in an isolated HOME with
/// `extra_env` added.
fn run_setup(
    program: &Path,
    home: &Path,
    args: &[&std::ffi::OsStr],
    extra_env: &[(&str, &Path)],
) -> Output {
    let mut cmd = Command::new(program);
    cmd.args(["setup", "--non-interactive"])
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_NERD_FONT", "0");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("failed to spawn gpy setup")
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

/// A GPY block as an earlier release wrote it.
fn block(body: &str) -> String {
    format!("\n# >>> gpy-init >>>\n# GPY shell integration\n{body}\n# <<< gpy-init <<<\n")
}

fn guarded(gpy_bash: &Path) -> String {
    format!(
        "case $- in\n    *i*) source \"{}\" ;;\nesac",
        gpy_bash.display()
    )
}

/// An 0.8-style install of `shell`: the tree under `$XDG_CONFIG_HOME`, stamped.
fn old_tree(home: &Path, shell: &str) -> PathBuf {
    let dir = if shell == "fish" {
        home.join(".config/fish/gpy")
    } else {
        home.join(".config/gpy").join(shell)
    };
    copy_dir(&repo_root().join(shell), &dir);
    fs::write(dir.join(".gpy-version"), "0.8.0\n").unwrap();
    dir
}

fn old_fish_block(old: &Path) -> String {
    block(&format!(
        "if status is-interactive\n    source \"{}\"\nend",
        old.join("conf.d/gpy_init.fish").display()
    ))
}

fn files_named(dir: &Path, needle: &str) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.to_string_lossy().contains(needle))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn fresh_setup_uses_the_data_root() {
    let home = TempDir::new().unwrap();

    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    let root = data(home.path());
    for tree in ["fish", "bash", "zsh", "plugins"] {
        assert!(
            root.join(tree).is_dir(),
            "{tree} is installed in the data root"
        );
    }
    for old in ["bash", "zsh", "bundled-plugins"] {
        assert!(
            !home.path().join(".config/gpy").join(old).exists(),
            "nothing named {old} under the config root"
        );
    }
    assert!(!home.path().join(".config/fish/gpy").exists());
    assert!(
        !home
            .path()
            .join(".config/fish/completions/gpy.fish")
            .exists(),
        "no Fish completions for a Zsh setup"
    );
}

#[test]
fn custom_xdg_data_home_is_found_by_a_bare_setup() {
    let home = TempDir::new().unwrap();
    let prefix = TempDir::new().unwrap();
    let gpy = installed_gpy(prefix.path());
    let xdg_data = home.path().join("data");
    let env = [("XDG_DATA_HOME", xdg_data.as_path())];

    assert_ok(&run_setup(
        &gpy,
        home.path(),
        &[
            "--shell".as_ref(),
            "zsh".as_ref(),
            "--from".as_ref(),
            repo_root().as_os_str(),
        ],
        &env,
    ));
    assert!(xdg_data.join("gpy/zsh/gpy.zsh").is_file());
    assert!(!data(home.path()).exists());

    assert_ok(&run_setup(
        &gpy,
        home.path(),
        &["--shell".as_ref(), "bash".as_ref()],
        &env,
    ));
    assert!(xdg_data.join("gpy/bash/gpy.bash").is_file());
    assert!(xdg_data.join("gpy/bash/completions/gpy.bash").is_file());
}

#[test]
fn a_relative_xdg_data_home_is_ignored() {
    let home = TempDir::new().unwrap();
    let relative = Path::new("relative-data");
    let output = run_setup(
        Path::new(env!("CARGO_BIN_EXE_gpy")),
        home.path(),
        &[
            "--shell".as_ref(),
            "zsh".as_ref(),
            "--from".as_ref(),
            repo_root().as_os_str(),
        ],
        &[("XDG_DATA_HOME", relative)],
    );
    assert_ok(&output);
    assert!(data(home.path()).join("zsh/gpy.zsh").is_file());
    assert!(!home.path().join("relative-data").exists());
}

#[test]
fn setup_in_place_from_the_data_root_copies_nothing() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let root = data(home.path());
    let entry = root.join("zsh/gpy.zsh");
    let mut edited = fs::read_to_string(&entry).unwrap();
    edited.push_str("# edited in place\n");
    fs::write(&entry, &edited).unwrap();
    fs::remove_file(root.join("zsh/.gpy-version")).unwrap();
    fs::remove_file(root.join("zsh/completions/_gpy")).unwrap();

    let output = run_setup(
        Path::new(env!("CARGO_BIN_EXE_gpy")),
        home.path(),
        &[
            "--shell".as_ref(),
            "zsh".as_ref(),
            "--from".as_ref(),
            root.as_os_str(),
        ],
        &[],
    );

    assert_ok(&output);
    assert_eq!(
        fs::read_to_string(&entry).unwrap(),
        edited,
        "nothing was copied"
    );
    assert!(root.join("zsh/.gpy-version").is_file());
    assert!(root.join("zsh/completions/_gpy").is_file());
    assert_eq!(block_count(&home.path().join(".zshrc")), 1);
}

#[test]
fn checkout_build_prefers_the_checkout_over_the_data_root() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let entry = data(home.path()).join("zsh/gpy.zsh");
    fs::write(&entry, "# stale copy\n").unwrap();

    let output = run_setup(
        Path::new(env!("CARGO_BIN_EXE_gpy")),
        home.path(),
        &["--shell".as_ref(), "zsh".as_ref()],
        &[],
    );

    assert_ok(&output);
    assert_eq!(
        fs::read(&entry).unwrap(),
        fs::read(repo_root().join("zsh/gpy.zsh")).unwrap(),
        "the checkout's files replaced the stale data root copy"
    );
}

#[test]
fn setup_migrates_an_0_8_layout_without_deleting() {
    let home = TempDir::new().unwrap();
    let h = home.path();
    let old_fish = old_tree(h, "fish");
    let old_zsh = old_tree(h, "zsh");
    let old_bash = old_tree(h, "bash");
    fs::write(
        h.join(".zshrc"),
        block(&format!("source \"{}\"", old_zsh.join("gpy.zsh").display())),
    )
    .unwrap();
    fs::write(
        h.join(".bashrc"),
        block(&format!(
            "source \"{}\"",
            old_bash.join("gpy.bash").display()
        )),
    )
    .unwrap();
    fs::write(
        h.join(".config/fish/config.fish"),
        old_fish_block(&old_fish),
    )
    .unwrap();
    let functions = h.join(".config/fish/functions");
    fs::create_dir_all(&functions).unwrap();
    symlink(
        old_fish.join("functions/fish_prompt.fish"),
        functions.join("fish_prompt.fish"),
    )
    .unwrap();

    let output = setup(h, &["--shell", "zsh"]);

    assert_ok(&output);
    let root = data(h);
    let zshrc = fs::read_to_string(h.join(".zshrc")).unwrap();
    assert_eq!(block_count(&h.join(".zshrc")), 1);
    assert!(
        zshrc.contains(&root.join("zsh/gpy.zsh").display().to_string()),
        "{zshrc}"
    );
    let bashrc = fs::read_to_string(h.join(".bashrc")).unwrap();
    assert_eq!(block_count(&h.join(".bashrc")), 1);
    assert!(
        bashrc.contains(&guarded(&root.join("bash/gpy.bash"))),
        "{bashrc}"
    );
    let config_fish = fs::read_to_string(h.join(".config/fish/config.fish")).unwrap();
    assert_eq!(block_count(&h.join(".config/fish/config.fish")), 1);
    assert!(
        config_fish.contains(&root.join("fish/conf.d/gpy_init.fish").display().to_string()),
        "{config_fish}"
    );
    assert_eq!(
        fs::read_link(functions.join("fish_prompt.fish")).unwrap(),
        root.join("fish/functions/fish_prompt.fish")
    );
    assert!(
        files_named(&functions, ".backup.").is_empty(),
        "no backup of GPY's own link"
    );
    for old in [&old_fish, &old_zsh, &old_bash] {
        assert!(
            old.join(".gpy-version").is_file(),
            "{} is not deleted",
            old.display()
        );
        let hint = format!(
            "The GPY files in {dir} are no longer used; remove them after restarting your open shells: rm -rf '{dir}'",
            dir = old.display()
        );
        assert!(stdout(&output).contains(&hint), "{}", stdout(&output));
    }
}

#[test]
fn bash_migration_accepts_the_guarded_and_unguarded_bodies() {
    for guarded_body in [false, true] {
        let home = TempDir::new().unwrap();
        let h = home.path();
        let old_bash = old_tree(h, "bash");
        let entry = old_bash.join("gpy.bash");
        let body = if guarded_body {
            guarded(&entry)
        } else {
            format!("source \"{}\"", entry.display())
        };
        fs::write(h.join(".bashrc"), format!("export A=1\n{}", block(&body))).unwrap();

        assert_ok(&setup(h, &["--shell", "bash"]));

        let bashrc = fs::read_to_string(h.join(".bashrc")).unwrap();
        assert_eq!(block_count(&h.join(".bashrc")), 1, "{bashrc}");
        assert!(bashrc.starts_with("export A=1\n"), "{bashrc}");
        assert!(
            bashrc.contains(&guarded(&data(h).join("bash/gpy.bash"))),
            "{bashrc}"
        );
        assert!(!bashrc.contains(&entry.display().to_string()), "{bashrc}");
    }
}

#[test]
fn fish_prompt_link_into_old_tree_is_replaced_without_backup() {
    let home = TempDir::new().unwrap();
    let h = home.path();
    let old_fish = old_tree(h, "fish");
    let functions = h.join(".config/fish/functions");
    fs::create_dir_all(&functions).unwrap();
    symlink(
        old_fish.join("functions/fish_prompt.fish"),
        functions.join("fish_prompt.fish"),
    )
    .unwrap();

    // The chosen shell is not Fish: the link is still re-pointed.
    assert_ok(&setup(h, &["--shell", "zsh"]));

    assert_eq!(
        fs::read_link(functions.join("fish_prompt.fish")).unwrap(),
        data(h).join("fish/functions/fish_prompt.fish")
    );
    assert_eq!(files_named(&functions, ".backup.").len(), 0_usize);
    assert!(old_fish.join("functions/fish_prompt.fish").is_file());
}

#[test]
fn edited_rc_block_is_kept_and_reported() {
    let home = TempDir::new().unwrap();
    let h = home.path();
    let old_zsh = old_tree(h, "zsh");
    let edited = format!("source \"{}\" # mine", old_zsh.join("gpy.zsh").display());
    fs::write(h.join(".zshrc"), block(&edited)).unwrap();

    let output = setup(h, &["--shell", "zsh"]);

    assert_ok(&output);
    assert!(
        fs::read_to_string(h.join(".zshrc"))
            .unwrap()
            .contains(&edited)
    );
    let out = stdout(&output);
    assert!(out.contains("Your edited GPY block in"), "{out}");
    assert!(
        out.contains(&format!("still loads {}", old_zsh.display())),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "change it to {}",
            data(h).join("zsh/gpy.zsh").display()
        )),
        "{out}"
    );
    assert!(old_zsh.join("gpy.zsh").is_file());
}

#[test]
fn completions_only_for_wired_shells() {
    let home = TempDir::new().unwrap();
    let h = home.path();
    assert_ok(&setup(h, &["--shell", "zsh"]));
    let root = data(h);
    assert!(root.join("zsh/completions/_gpy").is_file());
    assert!(!root.join("bash/completions/gpy.bash").exists());
    assert!(!h.join(".config/fish/completions/gpy.fish").exists());

    assert_ok(&setup(h, &["--shell", "bash"]));
    fs::remove_file(root.join("bash/completions/gpy.bash")).unwrap();
    assert_ok(&setup(h, &["--shell", "zsh"]));

    assert!(
        root.join("bash/completions/gpy.bash").is_file(),
        "Bash is wired now, so a Zsh run refreshes its completions"
    );
    assert!(!h.join(".config/fish/completions/gpy.fish").exists());
}

#[test]
fn doctor_lists_shells_by_rc_block() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .arg("doctor")
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn gpy doctor");

    let report = stdout(&output);
    assert!(report.contains("Checking zsh integration"), "{report}");
    assert!(!report.contains("Checking bash integration"), "{report}");
    assert!(!report.contains("Checking fish integration"), "{report}");
}

#[test]
fn bundled_root_falls_back_to_the_old_location() {
    let home = TempDir::new().unwrap();
    let old_root = home.path().join(".config/gpy/bundled-plugins");
    copy_dir(&repo_root().join("plugins"), &old_root);

    let list = |dir: &Path| {
        let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
            .arg("segments")
            .current_dir(dir)
            .env_clear()
            .env("HOME", dir)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .output()
            .expect("failed to spawn gpy segments");
        stdout(&output)
    };

    assert!(
        list(home.path()).contains("username"),
        "the old root still serves username"
    );

    // Once the data root has its own `plugins`, the old root is ignored.
    fs::create_dir_all(data(home.path()).join("plugins")).unwrap();
    assert!(!list(home.path()).contains("username"));
}
