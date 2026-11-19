//! The late window: an agent that connects but never answers costs one
//! budget, then every request skips it for 5 s; a reply in time, or a
//! `.reload`/`.reregister` flag the agent wrote, closes the window.

use crate::harness::{Answer, ScriptedAgent, Shell, ack};
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The reply budget in these tests: large enough that a prompt that skips
/// the agent is clearly faster than one that waited.
const BUDGET: Duration = Duration::from_secs(1);
const BUDGET_ENV: (&str, &str) = ("GPY_IPC_TIMEOUT_MS", "1000");

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
}

/// A shell drawing only the directory (its only request is `register`),
/// a directory to draw it in, and an agent that holds every connection
/// open until `answering` is set, then acks.
fn wedged() -> (Shell, PathBuf, ScriptedAgent, Arc<AtomicBool>) {
    let shell = Shell::new(r#""directory""#);
    let dir = shell.root().join("a");
    std::fs::create_dir_all(&dir).expect("mkdir a");
    let answering = Arc::new(AtomicBool::new(false));
    let agent = {
        let answers = Arc::clone(&answering);
        ScriptedAgent::spawn(shell.socket(), move |_| {
            if answers.load(Ordering::Relaxed) {
                ack()
            } else {
                Answer::Hold
            }
        })
    };
    (shell, dir, agent, answering)
}

/// Three prompts against a wedged agent cost one budget and one request;
/// the window lasts 5 s for this socket; once it has expired the next
/// prompt asks again, and that reply in time removes the stamp.
#[test]
#[serial]
fn wedged_agent_costs_one_budget_per_window() {
    let (shell, dir, agent, answering) = wedged();
    let first = shell.prompt(&dir, &[BUDGET_ENV]);
    assert!(
        first.elapsed >= BUDGET,
        "the first prompt waited: {:?}",
        first.elapsed
    );
    let stamp = std::fs::read_to_string(shell.late_window()).expect("the late window opened");
    let (until_text, socket) = stamp.split_once(' ').expect("`<until> <socket>`");
    let until: u64 = until_text.parse().expect("epoch seconds");
    let now = now_secs();
    assert!(
        (now.saturating_add(3)..=now.saturating_add(5)).contains(&until),
        "open about 5 s: {stamp}"
    );
    assert_eq!(socket, shell.socket().to_string_lossy(), "for this socket");

    let second = shell.prompt(&dir, &[BUDGET_ENV]);
    let third = shell.prompt(&dir, &[BUDGET_ENV]);
    assert!(
        second.elapsed.saturating_add(third.elapsed) < BUDGET,
        "inside the window the agent is skipped: {:?} + {:?}",
        second.elapsed,
        third.elapsed
    );
    assert_eq!(
        agent.requests().len(),
        1,
        "one request: {:?}",
        agent.requests()
    );

    // Expire the window rather than wait 5 s.
    let expired = format!("{} {socket}", now.saturating_sub(1));
    std::fs::write(shell.late_window(), expired).expect("expire");
    answering.store(true, Ordering::Relaxed);
    shell.prompt(&dir, &[BUDGET_ENV]);
    assert_eq!(
        agent.requests().len(),
        2,
        "asked again: {:?}",
        agent.requests()
    );
    assert!(
        !shell.late_window().exists(),
        "a reply in time closes the window"
    );
}

/// A `.reload` or `.reregister` flag means the agent is up: the prompt
/// that consumes it closes the window and asks the agent again.
#[test]
#[serial]
fn consumed_flag_closes_the_window() {
    for flag in ["reload", "reregister"] {
        let (shell, dir, agent, _) = wedged();
        shell.prompt(&dir, &[BUDGET_ENV]);
        assert!(
            shell.late_window().exists(),
            "{flag}: the late window opened"
        );
        let flag_file = shell.shell_flag(flag);
        std::fs::create_dir_all(flag_file.parent().expect("shells dir")).expect("mkdir shells");
        std::fs::write(&flag_file, "").expect("write the flag");
        shell.prompt(&dir, &[BUDGET_ENV]);
        assert!(!flag_file.exists(), "{flag}: consumed");
        assert_eq!(
            agent.requests().len(),
            2,
            "{flag}: the agent is asked again: {:?}",
            agent.requests()
        );
    }
}
