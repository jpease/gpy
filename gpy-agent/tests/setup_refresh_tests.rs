//! An upgrade that runs no `gpy setup` (Homebrew) leaves the shell trees of
//! the previous version installed. The next `gpy prompt` notices its shell's
//! stamp names another version and reinstalls the stale trees in the
//! background, leaving every rc file alone.

#![cfg(unix)]
#![allow(clippy::expect_used, reason = "test setup fails loudly")]
#![allow(clippy::unwrap_used, reason = "test setup fails loudly")]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, repo_root, setup};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// One Zsh prompt in `home`, with no agent.
fn prompt(home: &Path) {
    let runtime = home.join("run");
    fs::create_dir_all(&runtime).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["prompt", "zsh", "--status", "0", "--pid", "4242"])
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_AGENT_ENABLED", "0");
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        cmd.env("LLVM_PROFILE_FILE", profile);
    }
    let output = cmd.output().expect("failed to spawn gpy prompt");
    assert!(
        output.status.success(),
        "gpy prompt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stamp(dir: &Path) -> String {
    fs::read_to_string(dir.join(".gpy-version"))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[test]
fn a_prompt_refreshes_trees_an_upgrade_left_behind_without_touching_rc_files() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    assert_ok(&setup(home.path(), &["--shell", "bash"]));
    let root = home.path().join(".local/share/gpy");
    let (zsh_dir, bash_dir) = (root.join("zsh"), root.join("bash"));

    // What an older gpy left: its own scripts and stamps.
    fs::write(zsh_dir.join("gpy.zsh"), "# scripts from an older gpy\n").unwrap();
    fs::write(bash_dir.join("gpy.bash"), "# scripts from an older gpy\n").unwrap();
    fs::write(zsh_dir.join(".gpy-version"), "0.0.1\n").unwrap();
    fs::write(bash_dir.join(".gpy-version"), "0.0.1\n").unwrap();
    let zshrc = fs::read_to_string(home.path().join(".zshrc")).unwrap();
    let bashrc = fs::read_to_string(home.path().join(".bashrc")).unwrap();

    prompt(home.path());

    // The refresh runs detached; the prompt never waits for it.
    let deadline = Instant::now() + Duration::from_secs(20);
    while (stamp(&zsh_dir) != env!("CARGO_PKG_VERSION")
        || stamp(&bash_dir) != env!("CARGO_PKG_VERSION"))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    for (dir, file) in [(&zsh_dir, "zsh/gpy.zsh"), (&bash_dir, "bash/gpy.bash")] {
        assert_eq!(
            stamp(dir),
            env!("CARGO_PKG_VERSION"),
            "{} restamped",
            dir.display()
        );
        let name = Path::new(file).file_name().unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(name)).unwrap(),
            fs::read_to_string(repo_root().join(file)).unwrap(),
            "{file} refreshed"
        );
    }
    assert_eq!(
        fs::read_to_string(home.path().join(".zshrc")).unwrap(),
        zshrc
    );
    assert_eq!(
        fs::read_to_string(home.path().join(".bashrc")).unwrap(),
        bashrc
    );
}

#[test]
fn a_tree_without_a_stamp_is_left_alone() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let zsh_dir = home.path().join(".local/share/gpy/zsh");
    fs::remove_file(zsh_dir.join(".gpy-version")).unwrap();
    fs::write(zsh_dir.join("gpy.zsh"), "# hand-made\n").unwrap();

    prompt(home.path());
    std::thread::sleep(Duration::from_secs(2));

    assert_eq!(
        fs::read_to_string(zsh_dir.join("gpy.zsh")).unwrap(),
        "# hand-made\n",
        "only trees setup stamped are refreshed"
    );
}
