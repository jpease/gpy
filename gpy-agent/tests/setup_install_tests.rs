//! Integration tests for how `gpy setup` replaces installed files: a failure
//! while preparing the replacement must leave the previous install exactly as
//! it was, and an install location that aliases or overlaps the
//! source must be refused before anything is touched.
//!
//! Every test drives the real `gpy` binary against a disposable HOME (whose
//! `~/.local/share/gpy` is the data root), a copy of the repository's shell
//! trees as the source, and a temporary prefix holding the binary (so the
//! binary never sits in the build directory). The working checkout is never
//! an install or source location. Permission-based fixtures are skipped as root, which
//! bypasses file modes.

#![cfg(unix)]
#![allow(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure"
)]
#![allow(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure"
)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate lives inside the repository")
        .to_path_buf()
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

/// Whether file modes are ignored for this process.
fn running_as_root() -> bool {
    nix::unistd::geteuid().is_root()
}

/// Every regular file, directory and symlink below `dir` (relative path ->
/// bytes; a directory is `name/` with no bytes; a symlink holds its target),
/// so two snapshots compare a whole tree byte for byte. Empty when `dir` is
/// missing.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(rel, format!("-> {}", target.display()).into_bytes());
            } else if meta.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Sorted relative paths of the regular files below `dir`.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    snapshot(dir)
        .into_keys()
        .filter(|rel| !rel.ends_with('/'))
        .map(PathBuf::from)
        .filter(|rel| dir.join(rel).is_file() && !dir.join(rel).is_symlink())
        .collect()
}

/// Append `marker` to every regular file below `dir`: the "newer release".
fn bump_tree(dir: &Path, marker: &str) {
    for rel in files_under(dir) {
        let path = dir.join(rel);
        let mut text = fs::read(&path).unwrap();
        text.extend_from_slice(marker.as_bytes());
        fs::write(path, text).unwrap();
    }
}

/// A source file made unreadable until dropped.
struct Unreadable(PathBuf);

impl Unreadable {
    fn new(path: PathBuf) -> Self {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        Self(path)
    }
}

impl Drop for Unreadable {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o644));
    }
}

/// No hidden staging or backup directory is left beside the install.
fn assert_no_leftovers(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.contains(".gpy-staging-"),
            "staging left in {}: {name}",
            dir.display()
        );
        assert!(
            !name.contains(".gpy-old-"),
            "backup left in {}: {name}",
            dir.display()
        );
        assert!(
            !name.contains(".gpy-new-"),
            "temp file left in {}: {name}",
            dir.display()
        );
    }
}

/// A disposable HOME, a copy of the shell trees as the setup source, and a
/// prefix with the `gpy` binary (hard-linked, or copied across filesystems).
struct Fixture {
    home: TempDir,
    source: TempDir,
    prefix: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let source = TempDir::new().unwrap();
        for tree in ["fish", "zsh", "bash", "plugins"] {
            copy_dir(&repo_root().join(tree), &source.path().join(tree));
        }
        let prefix = TempDir::new().unwrap();
        let bin = prefix.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let built = env!("CARGO_BIN_EXE_gpy");
        if fs::hard_link(built, bin.join("gpy")).is_err() {
            fs::copy(built, bin.join("gpy")).unwrap();
        }
        Self {
            home: TempDir::new().unwrap(),
            source,
            prefix,
        }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn source(&self) -> &Path {
        self.source.path()
    }

    fn config(&self) -> PathBuf {
        self.home().join(".config")
    }

    /// The data root `gpy setup` installs into.
    fn data(&self) -> PathBuf {
        self.home().join(".local/share/gpy")
    }

    fn zsh_dir(&self) -> PathBuf {
        self.data().join("zsh")
    }

    fn bash_dir(&self) -> PathBuf {
        self.data().join("bash")
    }

    fn fish_dir(&self) -> PathBuf {
        self.data().join("fish")
    }

    /// `gpy setup --non-interactive` in the isolated environment.
    fn cmd(&self) -> Command {
        let mut cmd = Command::new(self.prefix.path().join("bin/gpy"));
        cmd.args(["setup", "--non-interactive"])
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("GPY_NERD_FONT", "0");
        cmd
    }

    /// Set `shell` up from the fixture's source copy.
    fn setup(&self, shell: &str) -> Output {
        self.cmd()
            .args(["--shell", shell, "--from"])
            .arg(self.source())
            .output()
            .unwrap()
    }

    /// Set `shell` up from the data root, with no `--from`.
    fn bare_setup(&self, shell: &str) -> Output {
        self.cmd().args(["--shell", shell]).output().unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "gpy setup failed: {}{}",
        stdout(output),
        stderr(output)
    );
}

/// Setup refused: nonzero exit, both paths named, nothing reported as done.
fn assert_refused(output: &Output, source: &Path, install: &Path) {
    assert!(
        !output.status.success(),
        "setup must refuse: {}{}",
        stdout(output),
        stderr(output)
    );
    let err = stderr(output);
    assert!(
        err.contains(&source.display().to_string()),
        "error must name the source {}: {err}",
        source.display()
    );
    assert!(
        err.contains(&install.display().to_string()),
        "error must name the install {}: {err}",
        install.display()
    );
    assert!(!stdout(output).contains("Installed"), "{}", stdout(output));
}

fn zsh_binary() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("zsh"))
        .find(|path| path.is_file())
}

/// Start a real zsh that sources the installed `gpy.zsh` (no agent on PATH)
/// and report whether the integration loaded: its stderr and whether a core
/// function exists afterwards.
fn zsh_startup(zsh: &Path, install: &Path, home: &Path) -> (String, bool) {
    let runtime = home.join("rt");
    fs::create_dir_all(&runtime).unwrap();
    let output = Command::new(zsh)
        .args([
            "-f",
            "-c",
            "source \"$1/gpy.zsh\"; echo \"loaded=$(whence -w __gpy_g_reload)\"",
            "zsh",
        ])
        .arg(install)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("GPY_NERD_FONT", "0")
        .output()
        .unwrap();
    (
        stderr(&output),
        stdout(&output).contains("loaded=__gpy_g_reload: function"),
    )
}

// ---- aliased and overlapping install locations ----------------------

const UNCOMMITTED: &str = "# uncommitted local change\n";

#[test]
fn zsh_install_dir_symlinked_to_the_source_tree_is_refused_with_every_source_byte_kept() {
    let fx = Fixture::new();
    bump_tree(&fx.source().join("zsh"), UNCOMMITTED);
    fs::create_dir_all(fx.data()).unwrap();
    std::os::unix::fs::symlink(fx.source().join("zsh"), fx.zsh_dir()).unwrap();
    let before = snapshot(fx.source());

    let output = fx.setup("zsh");

    assert_refused(&output, fx.source(), &fx.zsh_dir());
    assert_eq!(
        snapshot(fx.source()),
        before,
        "the source must be untouched"
    );
    assert!(!fx.home().join(".zshrc").exists(), "no rc file on refusal");
    assert!(!fx.source().join("zsh/.gpy-version").exists());
}

#[test]
fn fish_install_dir_symlinked_to_the_source_tree_is_refused_with_every_source_byte_kept() {
    let fx = Fixture::new();
    bump_tree(&fx.source().join("fish"), UNCOMMITTED);
    fs::create_dir_all(fx.data()).unwrap();
    std::os::unix::fs::symlink(fx.source().join("fish"), fx.fish_dir()).unwrap();
    let before = snapshot(fx.source());

    let output = fx.setup("fish");

    assert_refused(&output, fx.source(), &fx.fish_dir());
    assert_eq!(
        snapshot(fx.source()),
        before,
        "the source must be untouched"
    );
    assert!(!fx.config().join("fish/config.fish").exists());
    assert!(!fx.config().join("fish/completions/gpy.fish").exists());
}

#[test]
fn an_alias_through_a_symlinked_parent_and_a_symlinked_source_path_is_refused() {
    let fx = Fixture::new();
    // The install dir is `~/.local/share/gpy/zsh`, reached through a symlinked
    // `~/.local/share/gpy`, and `--from` is spelled through another symlink:
    // no lexical match with the source tree anywhere.
    let dotfiles = TempDir::new().unwrap();
    std::os::unix::fs::symlink(fx.source().join("zsh"), dotfiles.path().join("zsh")).unwrap();
    fs::create_dir_all(fx.data().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(dotfiles.path(), fx.data()).unwrap();
    let alias = TempDir::new().unwrap();
    let from = alias.path().join("checkout");
    std::os::unix::fs::symlink(fx.source(), &from).unwrap();
    let before = snapshot(fx.source());

    let output = fx
        .cmd()
        .args(["--shell", "zsh", "--from"])
        .arg(&from)
        .output()
        .unwrap();

    assert_refused(&output, &from, &fx.zsh_dir());
    assert_eq!(snapshot(fx.source()), before);
    assert!(!fx.source().join("zsh/.gpy-version").exists());
}

#[test]
fn an_install_dir_inside_the_source_is_refused() {
    let fx = Fixture::new();
    let data_home = fx.source().join("data");
    let before = snapshot(fx.source());

    let output = fx
        .cmd()
        .args(["--shell", "zsh", "--from"])
        .arg(fx.source())
        .env("XDG_DATA_HOME", &data_home)
        .output()
        .unwrap();

    assert_refused(&output, fx.source(), &data_home.join("gpy/zsh"));
    assert_eq!(
        snapshot(fx.source()),
        before,
        "nothing may be created in the source"
    );
}

#[test]
fn a_source_inside_the_install_dir_is_refused() {
    let fx = Fixture::new();
    let checkout = fx.zsh_dir().join("checkout");
    for tree in ["fish", "zsh", "bash", "plugins"] {
        copy_dir(&repo_root().join(tree), &checkout.join(tree));
    }
    let before = snapshot(&fx.zsh_dir());

    let output = fx
        .cmd()
        .args(["--shell", "zsh", "--from"])
        .arg(&checkout)
        .output()
        .unwrap();

    assert_refused(&output, &checkout, &fx.zsh_dir());
    assert_eq!(
        snapshot(&fx.zsh_dir()),
        before,
        "the checkout must be untouched"
    );
}

#[test]
fn installing_an_other_shell_that_aliases_the_source_cannot_bypass_the_guard() {
    let fx = Fixture::new();
    assert_ok(&fx.setup("bash"));
    // Bash was set up earlier; its install dir now aliases the source tree.
    fs::remove_dir_all(fx.bash_dir()).unwrap();
    std::os::unix::fs::symlink(fx.source().join("bash"), fx.bash_dir()).unwrap();
    bump_tree(&fx.source().join("bash"), UNCOMMITTED);
    let before = snapshot(fx.source());

    let output = fx.setup("zsh");

    assert_ok(&output);
    assert!(
        stdout(&output).contains("Could not install the bash files"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        snapshot(fx.source()),
        before,
        "bash source bytes must survive"
    );
    assert!(
        fx.zsh_dir().join("gpy.zsh").is_file(),
        "zsh itself was set up"
    );
    assert_eq!(
        fs::read_link(fx.bash_dir()).unwrap(),
        fx.source().join("bash"),
        "the user's symlink stays"
    );
}

#[test]
fn a_bundled_plugins_symlink_is_refused_before_any_file_is_replaced() {
    let fx = Fixture::new();
    let mine = TempDir::new().unwrap();
    fs::write(mine.path().join("mine.txt"), "mine\n").unwrap();
    fs::create_dir_all(fx.data()).unwrap();
    let link = fx.data().join("plugins");
    std::os::unix::fs::symlink(mine.path(), &link).unwrap();

    let output = fx.setup("zsh");

    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(stderr(&output).contains("plugins"), "{}", stderr(&output));
    assert_eq!(fs::read_link(&link).unwrap(), mine.path());
    assert_eq!(
        fs::read_to_string(mine.path().join("mine.txt")).unwrap(),
        "mine\n"
    );
    assert!(!fx.zsh_dir().exists(), "no shell file before the refusal");
}

#[test]
fn a_symlinked_install_dir_to_a_separate_tree_is_updated_in_place() {
    let fx = Fixture::new();
    let elsewhere = TempDir::new().unwrap();
    let real = elsewhere.path().join("zsh");
    fs::create_dir_all(real.join("segments")).unwrap();
    fs::write(real.join("segments/old.zsh"), "retired\n").unwrap();
    fs::write(real.join("keep.txt"), "mine\n").unwrap();
    fs::create_dir_all(fx.data()).unwrap();
    std::os::unix::fs::symlink(&real, fx.zsh_dir()).unwrap();

    assert_ok(&fx.setup("zsh"));

    assert_eq!(
        fs::read_link(fx.zsh_dir()).unwrap(),
        real,
        "still the user's symlink"
    );
    assert!(real.join("gpy.zsh").is_file());
    assert!(real.join(".gpy-version").is_file());
    assert!(!real.join("segments/old.zsh").exists());
    assert_eq!(fs::read_to_string(real.join("keep.txt")).unwrap(), "mine\n");
    let zshrc = fs::read_to_string(fx.home().join(".zshrc")).unwrap();
    assert!(
        zshrc.contains(&fx.zsh_dir().join("gpy.zsh").display().to_string()),
        "the rc block sources the path the user set up: {zshrc}"
    );
}

// ---- a failed setup must keep the previous install ------------------

#[test]
fn a_successful_rerun_removes_retired_scripts_and_keeps_unrelated_files() {
    let fx = Fixture::new();
    assert_ok(&fx.setup("zsh"));
    assert_ok(&fx.setup("fish"));
    let zsh = fx.zsh_dir();
    for stale in ["segments/username.zsh", "segments/hostname.zsh"] {
        fs::write(zsh.join(stale), "retired\n").unwrap();
    }
    fs::write(zsh.join("notes.txt"), "mine\n").unwrap();
    fs::create_dir_all(zsh.join("custom")).unwrap();
    fs::write(zsh.join("custom/extra.zsh"), "mine\n").unwrap();
    let fish = fx.fish_dir();
    fs::write(fish.join("functions/retired.fish"), "retired\n").unwrap();
    fs::write(fish.join("fisher.json"), "{}\n").unwrap();

    assert_ok(&fx.setup("zsh"));
    assert_ok(&fx.setup("fish"));

    for gone in [
        zsh.join("segments/username.zsh"),
        zsh.join("segments/hostname.zsh"),
        fish.join("functions/retired.fish"),
    ] {
        assert!(!gone.exists(), "{gone:?} must be removed");
    }
    assert_eq!(fs::read_to_string(zsh.join("notes.txt")).unwrap(), "mine\n");
    assert_eq!(
        fs::read_to_string(zsh.join("custom/extra.zsh")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        fs::read_to_string(fish.join("fisher.json")).unwrap(),
        "{}\n"
    );
    assert!(zsh.join("segments/git.zsh").is_file());
    assert_no_leftovers(&fx.data());
    assert_no_leftovers(&fx.config().join("fish"));
}

#[test]
fn a_failed_copy_at_any_read_position_leaves_the_previous_zsh_install_unchanged() {
    if running_as_root() {
        eprintln!("skipped: root bypasses the unreadable-file fixture");
        return;
    }
    let fx = Fixture::new();
    assert_ok(&fx.setup("zsh"));
    // A newer release: every shipped file differs, so a partly applied copy
    // could not match the old install.
    bump_tree(&fx.source().join("zsh"), "# v2\n");
    let before = snapshot(&fx.zsh_dir());
    let zshrc = fs::read(fx.home().join(".zshrc")).unwrap();
    assert!(before.contains_key(".gpy-version"));
    let zsh = zsh_binary();
    if zsh.is_none() {
        eprintln!("note: zsh not found; the shell startup check is skipped");
    }

    let files = files_under(&fx.source().join("zsh"));
    assert!(files.len() >= 10, "{files:?}");
    let (middle, last) = (files.len() / 2, files.len().saturating_sub(1));
    for (position, rel) in files.iter().enumerate() {
        let victim = fx.source().join("zsh").join(rel);
        let guard = Unreadable::new(victim.clone());
        let output = fx.setup("zsh");
        drop(guard);

        assert!(
            !output.status.success(),
            "{rel:?}: setup must fail: {}",
            stdout(&output)
        );
        assert!(
            stderr(&output).contains(&victim.display().to_string()),
            "{rel:?}: the error must name the source file: {}",
            stderr(&output)
        );
        assert_eq!(snapshot(&fx.zsh_dir()), before, "{rel:?}: install changed");
        assert_eq!(fs::read(fx.home().join(".zshrc")).unwrap(), zshrc);
        assert_no_leftovers(&fx.data());
        let spot_check = [0, middle, last].contains(&position);
        if let (Some(zsh_path), true) = (&zsh, spot_check) {
            let (diagnostics, loaded) = zsh_startup(zsh_path, &fx.zsh_dir(), fx.home());
            assert!(
                loaded,
                "{rel:?}: the previous integration must still load: {diagnostics}"
            );
            assert!(
                !diagnostics.contains("GPY disabled"),
                "{rel:?}: {diagnostics}"
            );
        }
    }

    // With the fault gone the same command installs the new release.
    assert_ok(&fx.setup("zsh"));
    let updated = fs::read_to_string(fx.zsh_dir().join("gpy.zsh")).unwrap();
    assert!(
        updated.ends_with("# v2\n"),
        "the rerun installed the new files"
    );
}

#[test]
fn a_failed_other_shell_install_leaves_that_shell_installed_while_setup_succeeds() {
    if running_as_root() {
        eprintln!("skipped: root bypasses the unreadable-file fixture");
        return;
    }
    let fx = Fixture::new();
    assert_ok(&fx.setup("bash"));
    assert_ok(&fx.setup("zsh"));
    bump_tree(&fx.source().join("bash"), "# v2\n");
    bump_tree(&fx.source().join("zsh"), "# v2\n");
    let bash_before = snapshot(&fx.bash_dir());
    let _locked = Unreadable::new(fx.source().join("bash/core/ipc.bash"));

    let output = fx.setup("zsh");

    assert_ok(&output);
    let out = stdout(&output);
    assert!(out.contains("Could not install the bash files"), "{out}");
    assert!(
        out.contains("ipc.bash"),
        "the warning names the source file: {out}"
    );
    assert_eq!(
        snapshot(&fx.bash_dir()),
        bash_before,
        "bash install must be intact"
    );
    let zsh_entry = fs::read_to_string(fx.zsh_dir().join("gpy.zsh")).unwrap();
    assert!(
        zsh_entry.ends_with("# v2\n"),
        "the requested shell was updated"
    );
    assert_no_leftovers(&fx.data());
}

#[test]
fn fish_without_the_completion_glue_keeps_the_installed_tree_and_completions() {
    let fx = Fixture::new();
    assert_ok(&fx.setup("fish"));
    bump_tree(&fx.source().join("fish"), "# v2\n");
    let glue = fx.source().join("fish/completions/gpy-dynamic.fish");
    fs::remove_file(&glue).unwrap();
    let tree = snapshot(&fx.fish_dir());
    let completions = snapshot(&fx.config().join("fish/completions"));
    assert!(completions.contains_key("gpy.fish"));

    let output = fx.setup("fish");

    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("gpy-dynamic.fish"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        snapshot(&fx.fish_dir()),
        tree,
        "the fish tree must be untouched"
    );
    assert_eq!(snapshot(&fx.config().join("fish/completions")), completions);
    assert_no_leftovers(&fx.config().join("fish"));
    assert_no_leftovers(&fx.config().join("fish/completions"));
}

#[test]
fn fish_with_an_unreadable_completion_glue_keeps_the_installed_tree_and_completions() {
    if running_as_root() {
        eprintln!("skipped: root bypasses the unreadable-file fixture");
        return;
    }
    let fx = Fixture::new();
    assert_ok(&fx.setup("fish"));
    bump_tree(&fx.source().join("fish"), "# v2\n");
    let tree = snapshot(&fx.fish_dir());
    let completions = snapshot(&fx.config().join("fish/completions"));
    let glue = fx.source().join("fish/completions/gpy-dynamic.fish");
    let _locked = Unreadable::new(glue.clone());

    let output = fx.setup("fish");

    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains(&glue.display().to_string()),
        "{}",
        stderr(&output)
    );
    assert_eq!(snapshot(&fx.fish_dir()), tree);
    assert_eq!(snapshot(&fx.config().join("fish/completions")), completions);
}

#[test]
fn a_bare_setup_after_an_installer_style_run_finds_the_data_root() {
    let fx = Fixture::new();
    assert_ok(&fx.setup("zsh"));
    assert!(
        fx.bash_dir().join("gpy.bash").is_file(),
        "the first run installed every tree"
    );

    assert_ok(&fx.bare_setup("bash"));

    assert!(fx.bash_dir().join("core/ipc.bash").is_file());
    assert!(fx.bash_dir().join("completions/gpy.bash").is_file());
    assert!(fx.bash_dir().join(".gpy-version").is_file());
    assert_no_leftovers(&fx.data());
}
