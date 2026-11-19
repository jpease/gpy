//! Registration as `gpy prompt` keeps it: a `workspace` reply saying the
//! agent no longer knows the shell, or the agent's `.reregister` flag,
//! makes the prompt register again; a refused directory does not.

use crate::harness::{Answer, ScriptedAgent, Shell, ack, field};
use serial_test::serial;
use std::path::PathBuf;

/// A shell drawing only the directory (no git or language requests) and
/// two directories to `cd` between.
fn shell_with_dirs() -> (Shell, PathBuf, PathBuf) {
    let shell = Shell::new(r#""directory""#);
    let (a, b) = (shell.root().join("a"), shell.root().join("b"));
    std::fs::create_dir_all(&a).expect("mkdir a");
    std::fs::create_dir_all(&b).expect("mkdir b");
    (shell, a, b)
}

/// An agent that acks `register` and answers `workspace` with `error`.
fn agent_refusing_workspace(shell: &Shell, error: String) -> ScriptedAgent {
    ScriptedAgent::spawn(shell.socket(), move |request| {
        if field(request, "op") == "workspace" {
            Answer::Line(format!(r#"{{"error":"{error}"}}"#))
        } else {
            ack()
        }
    })
}

fn op(name: &str, cwd: &std::path::Path) -> (String, String) {
    (name.to_owned(), cwd.to_string_lossy().into_owned())
}

/// The agent lost the registration (it restarted): the `workspace` sync
/// after a `cd` is answered `not registered`, and the same prompt
/// registers again; later prompts in that directory send nothing.
#[test]
#[serial]
fn lost_registration_registers_again() {
    let (shell, a, b) = shell_with_dirs();
    let pid = std::process::id();
    let agent = agent_refusing_workspace(
        &shell,
        format!("PID {pid} not registered - please register first"),
    );
    shell.prompt(&a, &[]);
    shell.prompt(&a, &[]);
    assert_eq!(agent.ops(), [op("register", &a)], "registered once in a");
    shell.prompt(&b, &[]);
    shell.prompt(&b, &[]);
    assert_eq!(
        agent.ops(),
        [op("register", &a), op("workspace", &b), op("register", &b)],
        "a lost registration registers again, once"
    );
}

/// A refused directory keeps the registration and is not sent again until
/// the directory changes.
#[test]
#[serial]
fn refused_workspace_keeps_the_registration() {
    let (shell, a, b) = shell_with_dirs();
    let agent = agent_refusing_workspace(&shell, "path is not allowed".to_owned());
    shell.prompt(&a, &[]);
    shell.prompt(&b, &[]);
    shell.prompt(&b, &[]);
    assert_eq!(
        agent.ops(),
        [op("register", &a), op("workspace", &b)],
        "a refusal neither re-registers nor repeats"
    );
}

/// The agent's `.reregister` flag (written before its SIGURG doorbell)
/// makes the next prompt register again, even without a `cd`, and is
/// consumed.
#[test]
#[serial]
fn reregister_flag_registers_again() {
    let (shell, a, _) = shell_with_dirs();
    let agent = ScriptedAgent::spawn(shell.socket(), |_| ack());
    shell.prompt(&a, &[]);
    let flag = shell.shell_flag("reregister");
    assert!(
        flag.parent().is_some_and(std::path::Path::is_dir),
        "registering tracks the shell under {}",
        flag.display()
    );
    std::fs::write(&flag, "").expect("write the reregister flag");
    shell.prompt(&a, &[]);
    assert_eq!(agent.ops(), [op("register", &a), op("register", &a)]);
    assert!(!flag.exists(), "the flag is consumed");
}
