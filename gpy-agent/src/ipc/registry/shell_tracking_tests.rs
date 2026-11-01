#![allow(
    clippy::expect_used,
    reason = "cfg(all(test, unix)) is not recognized by clippy's in-tests config"
)]

use super::{
    MAX_RENUDGE_ATTEMPTS, ShellRenudger, announce_start_in, renudge_unregistered_shells_in,
    started_after_tracking,
};
use crate::ipc::ClientDirectory;

/// Seed `dir` with one tracking file per PID, as Fish does.
fn track_shells(dir: &std::path::Path, pids: &[u32]) {
    for pid in pids {
        std::fs::write(dir.join(pid.to_string()), pid.to_string()).expect("seed shell file");
    }
}

/// Tracked but unregistered shells are re-nudged.
#[test]
fn renudge_targets_live_unregistered_tracked_shells() {
    let dir = tempfile::tempdir().expect("create tempdir");
    track_shells(dir.path(), &[100_u32, 200_u32]);

    let mut renudger = ShellRenudger::default();
    let targets = renudger.pids_to_nudge(dir.path(), |pid| pid == 100_u32, |_, _| true);

    assert_eq!(
        targets,
        vec![200_u32],
        "only the tracked-but-unregistered shell should be nudged"
    );
}

/// Dead shells' tracking files are removed.
#[test]
fn renudge_removes_the_tracking_file_for_a_dead_shell() {
    let dir = tempfile::tempdir().expect("create tempdir");
    track_shells(dir.path(), &[300_u32]);

    let mut renudger = ShellRenudger::default();
    let targets = renudger.pids_to_nudge(dir.path(), |_| false, |_, _| false);

    assert!(targets.is_empty(), "a dead shell must not be signalled");
    assert!(
        !dir.path().join("300").exists(),
        "a dead shell's tracking file should be cleaned up"
    );
}

/// The restart nudge cleans up dead shells' tracking and flag files.
#[test]
fn restart_nudge_cleans_up_tracking_files_for_dead_shells() {
    let dir = tempfile::tempdir().expect("create tempdir");
    // PIDs far above any plausible live process.
    track_shells(dir.path(), &[999_997_u32, 999_998_u32]);
    for name in ["999997.reload", "999997.reregister", "999996.reload"] {
        std::fs::write(dir.path().join(name), "").expect("seed flag");
    }

    announce_start_in(dir.path(), || Ok(false));

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "the restart nudge must clean up tracking and flag files for exited shells: {leftovers:?}"
    );
}

/// The restart scan targets an alive tracked shell and keeps its file.
#[test]
fn nudge_all_tracked_shells_targets_and_preserves_an_alive_tracked_shell() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let pid = std::process::id();
    track_shells(dir.path(), &[pid]);

    let mut renudger = ShellRenudger::default();
    let targets = renudger.pids_to_nudge(
        dir.path(),
        |_| false,
        |tracked_pid, _| ClientDirectory::is_client_alive(tracked_pid),
    );

    assert_eq!(
        targets,
        vec![pid],
        "an alive tracked shell must be targeted for a nudge"
    );
    assert!(
        dir.path().join(pid.to_string()).exists(),
        "an alive shell's tracking file must survive (only dead ones are cleaned up)"
    );
}

/// A tracking file older than its PID's process means a recycled PID.
#[test]
fn nudge_skips_and_removes_tracking_file_older_than_its_process() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let pid = std::process::id();
    track_shells(dir.path(), &[pid]);
    let tracking = dir.path().join(pid.to_string());
    std::fs::File::options()
        .write(true)
        .open(&tracking)
        .expect("open tracking file")
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000))
        .expect("backdate tracking file");

    announce_start_in(dir.path(), || Ok(false));

    assert!(
        !dir.path().join(format!("{pid}.reregister")).exists(),
        "a recycled PID must not get a reregister flag"
    );
    assert!(
        !tracking.exists(),
        "a recycled PID's tracking file must be removed"
    );
}

/// An unknown start time or mtime never makes a PID look recycled.
#[test]
fn started_after_tracking_needs_more_than_a_second() {
    let written = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    assert!(!started_after_tracking(1000, written));
    assert!(!started_after_tracking(1001, written));
    assert!(started_after_tracking(1002, written));
}

/// A never-registering shell is nudged at most `MAX_RENUDGE_ATTEMPTS` times.
#[test]
fn renudge_stops_after_the_attempt_cap() {
    let dir = tempfile::tempdir().expect("create tempdir");
    track_shells(dir.path(), &[400_u32]);

    let mut renudger = ShellRenudger::default();
    let mut nudges = 0_u32;
    for _ in 0_u32..(MAX_RENUDGE_ATTEMPTS + 3_u32) {
        nudges += u32::try_from(
            renudger
                .pids_to_nudge(dir.path(), |_| false, |_, _| true)
                .len(),
        )
        .expect("nudge count fits in u32");
    }

    assert_eq!(
        nudges, MAX_RENUDGE_ATTEMPTS,
        "a permanently unregisterable shell should be nudged at most MAX_RENUDGE_ATTEMPTS times"
    );
}

/// Re-registering resets the shell's nudge budget.
#[test]
fn renudge_attempt_budget_resets_once_the_shell_registers() {
    let dir = tempfile::tempdir().expect("create tempdir");
    track_shells(dir.path(), &[500_u32]);

    let mut renudger = ShellRenudger::default();
    for _ in 0_u32..MAX_RENUDGE_ATTEMPTS {
        let _ = renudger.pids_to_nudge(dir.path(), |_| false, |_, _| true);
    }
    assert!(
        renudger
            .pids_to_nudge(dir.path(), |_| false, |_, _| true)
            .is_empty(),
        "budget should be exhausted before the recovery round"
    );

    // Recovered: the agent now knows this PID again.
    let _ = renudger.pids_to_nudge(dir.path(), |_| true, |_, _| true);

    assert_eq!(
        renudger.pids_to_nudge(dir.path(), |_| false, |_, _| true),
        vec![500_u32],
        "a shell that recovered and was stranded again should get a fresh budget"
    );
}

/// Spawn a stand-in shell that touches `mark` when SIGURG arrives while its
/// `<pid>.reregister` flag already exists in `dir`.
fn spawn_reregister_observer(dir: &std::path::Path, mark: &std::path::Path) -> std::process::Child {
    let script = format!(
        "trap '[ -e \"{dir}/$$.reregister\" ] && touch \"{mark}\"' URG; while :; do sleep 0.05; done",
        dir = dir.display(),
        mark = mark.display(),
    );
    let child = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .spawn()
        .expect("spawn stand-in shell");
    // Let sh install its trap before any doorbell rings.
    std::thread::sleep(std::time::Duration::from_millis(200));
    child
}

fn wait_for(path: &std::path::Path) -> bool {
    for _ in 0_u32..250_u32 {
        if path.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    path.exists()
}

/// Only the unregistered shell gets its `.reregister` flag and doorbell.
#[test]
fn renudge_signals_only_the_unregistered_tracked_shell() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let registered_mark = dir.path().join("registered.mark");
    let stranded_mark = dir.path().join("stranded.mark");

    let mut registered_child = spawn_reregister_observer(dir.path(), &registered_mark);
    let mut stranded_child = spawn_reregister_observer(dir.path(), &stranded_mark);

    let registered_pid = registered_child.id();
    let stranded_pid = stranded_child.id();
    track_shells(dir.path(), &[registered_pid, stranded_pid]);

    let registry = ClientDirectory::new();
    registry.register(registered_pid, None);

    let mut renudger = ShellRenudger::default();
    renudge_unregistered_shells_in(dir.path(), &mut renudger, &registry);

    assert!(
        wait_for(&stranded_mark),
        "the unregistered tracked shell should get the doorbell with its reregister flag present"
    );
    assert!(
        !dir.path()
            .join(format!("{registered_pid}.reregister"))
            .exists(),
        "the registered shell must not be asked to re-register"
    );
    assert!(!registered_mark.exists());

    for child in [&mut registered_child, &mut stranded_child] {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The restart nudge writes `.reregister`, and a shell without a handler survives.
#[test]
fn restart_nudge_writes_reregister_flag_and_spares_unhandled_shell() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn unhandled stand-in shell");
    let pid = child.id();
    track_shells(dir.path(), &[pid]);

    announce_start_in(dir.path(), || Ok(false));
    // Give an unhandled SIGURG time to kill the child, if it would.
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(
        dir.path().join(format!("{pid}.reregister")).exists(),
        "the restart nudge must write the reregister flag"
    );
    assert!(
        child.try_wait().expect("poll child").is_none(),
        "an unhandled process must survive the restart nudge"
    );
    let _ = child.kill();
    let _ = child.wait();
}

/// The startup warm-up raises `.reload` only when the export changed.
#[test]
fn warm_theme_export_flags_reload_only_when_the_export_changed() {
    let shell_dir = tempfile::tempdir().expect("create shell dir");
    let cache = tempfile::tempdir().expect("create cache dir");
    let config = crate::config::Config::default();
    let default_theme = crate::theme::ThemeManager::builtin("default").expect("default theme");
    let text_theme = crate::theme::ThemeManager::builtin("text").expect("text theme");
    // The previous agent's export, as the shell sourced it.
    crate::cache::theme_export::write_theme_export_to_dir(
        cache.path(),
        &default_theme,
        &config,
        &crate::template::Palette::default(),
    )
    .expect("seed export");

    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn unhandled stand-in shell");
    let pid = child.id();
    track_shells(shell_dir.path(), &[pid]);
    let reload_flag = shell_dir.path().join(format!("{pid}.reload"));

    announce_start_in(shell_dir.path(), || {
        crate::cache::theme_export::write_theme_export_to_dir(
            cache.path(),
            &default_theme,
            &config,
            &crate::template::Palette::default(),
        )
    });
    let flagged_on_unchanged = reload_flag.exists();

    announce_start_in(shell_dir.path(), || {
        crate::cache::theme_export::write_theme_export_to_dir(
            cache.path(),
            &text_theme,
            &config,
            &crate::template::Palette::default(),
        )
    });
    let flagged_on_changed = reload_flag.exists();

    let _ = child.kill();
    let _ = child.wait();
    assert!(
        !flagged_on_unchanged,
        "an unchanged export must not ask shells to reload"
    );
    assert!(
        flagged_on_changed,
        "a changed export must write the tracked shell's reload flag"
    );
}

/// A start that changed the export rings each shell once, with `.reload`
/// and `.reregister` both already written: Zsh drops a second ring that
/// lands while its trap for the first is running.
#[test]
fn announce_start_rings_once_for_reload_and_reregister() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let mark = dir.path().join("rings.mark");
    let script = format!(
        "trap 'if [ -e \"{dir}/$$.reload\" ] && [ -e \"{dir}/$$.reregister\" ]; then echo both >>\"{mark}\"; else echo partial >>\"{mark}\"; fi' URG; while :; do sleep 0.05; done",
        dir = dir.path().display(),
        mark = mark.display(),
    );
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .spawn()
        .expect("spawn stand-in shell");
    // Let sh install its trap before the doorbell rings.
    std::thread::sleep(std::time::Duration::from_millis(200));
    track_shells(dir.path(), &[child.id()]);

    announce_start_in(dir.path(), || Ok(true));
    let rang = wait_for(&mark);
    // Room for a second ring to show up, if one were sent.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = child.kill();
    let _ = child.wait();

    assert!(rang, "the shell must be rung");
    assert_eq!(
        std::fs::read_to_string(&mark).expect("read mark"),
        "both\n",
        "one ring, after both flags were written"
    );
}
