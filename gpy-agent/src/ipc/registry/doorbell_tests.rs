#![allow(
    clippy::expect_used,
    reason = "cfg(all(test, unix)) is not recognized by clippy's in-tests config"
)]

use super::{ClientDirectory, ShellFlag};
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

/// Wait up to ~5s for `path` to exist.
fn wait_for(path: &Path) -> bool {
    for _ in 0_u32..250_u32 {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    path.exists()
}

fn kill_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A registered process with no handler (e.g. a just-`exec`ed shell) survives every notification.
#[test]
fn unhandled_registered_process_survives_repaint_and_reload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut child = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();

    let registry = ClientDirectory::with_shell_dir(dir.path().to_path_buf());
    registry.register(pid, None);
    registry.notify_repaint_force(None);
    registry.notify_repaint(None);
    registry.notify_reload();
    // Negative proof: give a default-terminate disposition time to kill the child.
    std::thread::sleep(Duration::from_millis(200));

    assert!(
        child.try_wait().expect("poll child").is_none(),
        "an unhandled process must survive the doorbell"
    );
    assert!(registry.is_registered(pid));
    kill_child(child);
}

/// The URG trap only records delivery if the reload flag already exists.
#[test]
fn notify_reload_writes_flag_before_signalling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mark = dir.path().join("delivered");
    let ready = dir.path().join("ready");
    let flag_glob = dir.path().join("$$.reload");
    let script = format!(
        "trap '[ -e \"{flag}\" ] && touch \"{mark}\"' URG; touch \"{ready}\"; while :; do sleep 0.05; done",
        flag = flag_glob.display(),
        mark = mark.display(),
        ready = ready.display(),
    );
    let child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .spawn()
        .expect("spawn sh");
    let pid = child.id();
    assert!(
        wait_for(&ready),
        "sh must install its trap before the doorbell rings"
    );

    let registry = ClientDirectory::with_shell_dir(dir.path().to_path_buf());
    registry.register(pid, None);
    registry.notify_reload();

    assert!(
        ShellFlag::Reload.path(dir.path(), pid).exists(),
        "reload flag must be written"
    );
    assert!(
        wait_for(&mark),
        "doorbell must arrive with the reload flag already present"
    );
    kill_child(child);
}
