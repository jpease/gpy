//! The instant cache as `gpy prompt` serves it: a stale git entry with the
//! agent down, wedged or up; a language entry under a chunked key; and the
//! variant fallbacks (another `prev_bg` token, another position).
//!
//! With `enabled_segments = ["git"]` (or `["language"]`) the segment is
//! first and last, so it reads `git_first_last` and, as the prompt's first
//! segment, `prev_bg = "black"`.

use crate::common::git::TestRepo;
use crate::common::wait::wait_until;
use crate::harness::{
    Answer, BOUNDED, LANDING, ScriptedAgent, Shell, ack, backdate, field, write_entry,
};
use serial_test::serial;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STALE: &str = "STALE-GIT-SENTINEL";
const NONE_TEXT: &str = "NONE-VARIANT-TEXT";
const AGENT_TEXT: &str = "AGENT-REPLY-TEXT";

/// A committed repository on `main`, and a shell drawing only `git`.
fn git_shell() -> (Shell, TestRepo) {
    (
        Shell::new(r#""git""#),
        TestRepo::new().with_initial_commit(),
    )
}

/// A stale `git_first_last.black` entry holding [`STALE`], with its marker.
fn stale_entry(shell: &Shell, repo: &Path) -> PathBuf {
    let entry = shell.cache_file(repo, "git_first_last", Some("black"));
    write_entry(&entry, STALE);
    backdate(&entry);
    write_entry(&shell.marker(repo, "git"), "");
    entry
}

/// The stale TTL: an entry a minute old is stale.
const STALE_TTL: (&str, &str) = ("GPY_GIT_INSTANT_CACHE_TTL_SECONDS", "5");

/// An agent that answers `op` with [`AGENT_TEXT`] after `delay` and acks
/// everything else.
fn agent_answering(shell: &Shell, op: &'static str, delay: Duration) -> ScriptedAgent {
    ScriptedAgent::spawn(shell.socket(), move |request| {
        if field(request, "op") == op {
            Answer::After(delay, AGENT_TEXT.to_owned())
        } else {
            ack()
        }
    })
}

/// No socket at all: the stale entry is never drawn; the in-process
/// oneshot draws the repository as it is now.
#[test]
#[serial]
fn stale_git_with_no_agent_draws_a_fresh_oneshot() {
    let (shell, repo) = git_shell();
    stale_entry(&shell, repo.path());
    let run = shell.prompt(repo.path(), &[STALE_TTL]);
    assert!(
        !run.stdout.contains(STALE),
        "stale entry served: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("main"),
        "no oneshot git status: {}",
        run.stdout
    );
}

/// A socket nobody listens on (the agent crashed): the stale entry is never
/// drawn; the refresh it attempts is refused at once and the in-process
/// oneshot draws the repository as it is now (#1284).
#[test]
#[serial]
fn stale_git_with_a_dead_socket_draws_a_fresh_oneshot() {
    let (shell, repo) = git_shell();
    stale_entry(&shell, repo.path());
    drop(std::os::unix::net::UnixListener::bind(shell.socket()).expect("bind"));
    let run = shell.prompt(repo.path(), &[STALE_TTL]);
    let _ = std::fs::remove_file(shell.socket());
    assert!(
        !run.stdout.contains(STALE),
        "stale entry served: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("main"),
        "no oneshot git status: {}",
        run.stdout
    );
    assert!(run.elapsed < BOUNDED, "the prompt took {:?}", run.elapsed);
}

/// Nothing cached and no agent to ask (#1284): no socket, an agent that
/// refuses the request, or a socket nobody listens on. Every shell draws
/// git with the in-process oneshot, never a blank segment.
#[test]
#[serial]
fn cold_git_without_a_working_agent_draws_a_oneshot() {
    let (shell, repo) = git_shell();
    let no_socket = shell.prompt(repo.path(), &[]);
    assert!(
        no_socket.stdout.contains("main"),
        "no socket: {}",
        no_socket.stdout
    );

    let agent = ScriptedAgent::spawn(shell.socket(), |request| {
        if field(request, "op") == "git" {
            Answer::Line(r#"{"error":"Parse error: path is not allowed"}"#.to_owned())
        } else {
            ack()
        }
    });
    let refused = shell.prompt(repo.path(), &[]);
    assert_eq!(agent.with_op("git").len(), 1, "{:?}", agent.requests());
    drop(agent);
    assert!(
        refused.stdout.contains("main"),
        "refused: {}",
        refused.stdout
    );

    drop(std::os::unix::net::UnixListener::bind(shell.socket()).expect("bind"));
    let dead = shell.prompt(repo.path(), &[]);
    let _ = std::fs::remove_file(shell.socket());
    assert!(dead.stdout.contains("main"), "dead socket: {}", dead.stdout);
    assert!(dead.elapsed < BOUNDED, "the prompt took {:?}", dead.elapsed);
}

/// An agent that accepts but never answers: the stale entry is drawn, a
/// background refresh for this context is sent, and nothing waits for it.
#[test]
#[serial]
fn stale_git_with_a_wedged_agent_serves_the_entry_and_requests_a_refresh() {
    let (shell, repo) = git_shell();
    stale_entry(&shell, repo.path());
    let agent = ScriptedAgent::spawn(shell.socket(), |_| Answer::Hold);
    let run = shell.prompt(repo.path(), &[STALE_TTL]);
    assert!(
        run.stdout.contains(STALE),
        "stale entry not served: {}",
        run.stdout
    );
    assert!(run.elapsed < BOUNDED, "the prompt took {:?}", run.elapsed);
    let refresh = agent.wait_for_op("git");
    let request = refresh.first().expect("one git request");
    assert_eq!(
        field(request, "prev_bg"),
        "black",
        "refresh for this context: {request}"
    );
    assert_eq!(field(request, "is_last"), true, "{request}");
    assert!(
        field(request, "is_first").is_null(),
        "refreshes omit is_first: {request}"
    );
    assert_eq!(
        refresh.len(),
        1,
        "one refresh, no bounded request: {refresh:?}"
    );
}

/// A live agent: the stale entry is drawn now and the agent rewrites it.
/// Written once the agent runs: starting, it clears entries it cannot
/// refresh.
#[test]
#[serial]
fn stale_git_with_a_live_agent_is_rewritten_by_the_refresh() {
    let (shell, repo) = git_shell();
    shell.sandbox.start_and_wait();
    let entry = stale_entry(&shell, repo.path());
    let run = shell.prompt(repo.path(), &[STALE_TTL]);
    assert!(
        run.stdout.contains(STALE),
        "stale entry not served: {}",
        run.stdout
    );
    let rewritten = wait_until(LANDING, Duration::from_millis(50), || {
        std::fs::read_to_string(&entry).is_ok_and(|text| !text.contains(STALE))
    });
    assert!(rewritten, "the agent did not rewrite {}", entry.display());
}

/// A project whose cache key is over 200 characters (stored under
/// 50-character chunk directories).
fn deep_project(shell: &Shell) -> PathBuf {
    let dir = shell
        .root()
        .join("a".repeat(75))
        .join("b".repeat(75))
        .join("c".repeat(50))
        .join("proj");
    std::fs::create_dir_all(&dir).expect("mkdir deep project");
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"deep\"\n").expect("Cargo.toml");
    std::fs::canonicalize(dir).expect("canonical deep project")
}

/// The language omit gate and the reader both find a chunked key: the
/// segment is drawn from it, and omitted once its presence marker is gone.
#[test]
#[serial]
fn language_under_a_chunked_cache_key_is_drawn() {
    let shell = Shell::new(r#""language""#);
    let project = deep_project(&shell);
    let entry = shell.cache_file(&project, "lang_first_last", Some("black"));
    assert_ne!(
        entry.parent(),
        Some(shell.cache_root().as_path()),
        "the key should be chunked: {}",
        entry.display()
    );
    write_entry(&entry, "LANG-CHUNKED-TEXT");
    let marker = shell.marker(&project, "lang");
    write_entry(&marker, "");
    let marked = shell.prompt(&project, &[]);
    assert!(
        marked.stdout.contains("LANG-CHUNKED-TEXT"),
        "not drawn: {}",
        marked.stdout
    );
    std::fs::remove_file(&marker).expect("remove marker");
    let unmarked = shell.prompt(&project, &[]);
    assert!(
        !unmarked.stdout.contains("LANG-CHUNKED-TEXT"),
        "drawn unmarked: {}",
        unmarked.stdout
    );
}

/// Language never waits: a `.none` fallback is drawn as is and a refresh
/// for the real `prev_bg` goes out in the background (long bucket names
/// hash their throttle stamp).
#[test]
#[serial]
fn language_none_fallback_is_drawn_and_refreshed_in_the_background() {
    let shell = Shell::new(r#""language""#);
    let project = deep_project(&shell);
    write_entry(
        &shell.cache_file(&project, "lang_first_last", None),
        NONE_TEXT,
    );
    write_entry(&shell.marker(&project, "lang"), "");
    let agent = agent_answering(&shell, "lang", Duration::ZERO);
    let run = shell.prompt(&project, &[]);
    assert!(
        run.stdout.contains(NONE_TEXT),
        ".none not drawn: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains(AGENT_TEXT),
        "language waited: {}",
        run.stdout
    );
    let refresh = agent.wait_for_op("lang");
    let request = refresh.first().expect("one lang request");
    assert_eq!(field(request, "prev_bg"), "black", "{request}");
}

/// Only the context-free `.none` git entry exists: its chevron is wrong
/// for `black`, so the prompt asks the agent within the warm budget and
/// draws the reply.
#[test]
#[serial]
fn git_none_fallback_draws_the_agents_reply() {
    let (shell, repo) = git_shell();
    write_entry(
        &shell.cache_file(repo.path(), "git_first_last", None),
        NONE_TEXT,
    );
    write_entry(&shell.marker(repo.path(), "git"), "");
    let agent = agent_answering(&shell, "git", Duration::ZERO);
    let run = shell.prompt(repo.path(), &[]);
    assert!(
        run.stdout.contains(AGENT_TEXT),
        "reply not drawn: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains(NONE_TEXT),
        ".none drawn: {}",
        run.stdout
    );
    let request = agent
        .with_op("git")
        .into_iter()
        .next()
        .expect("a git request");
    assert_eq!(field(&request, "prev_bg"), "black", "{request}");
    assert_eq!(field(&request, "is_first"), true, "{request}");
    assert_eq!(field(&request, "is_last"), true, "{request}");
}

/// The `.none` fallback with no agent, or one slower than the warm budget
/// (100 ms here): the `.none` text is drawn, never a oneshot.
#[test]
#[serial]
fn git_none_fallback_without_a_timely_agent_draws_none() {
    let (shell, repo) = git_shell();
    write_entry(
        &shell.cache_file(repo.path(), "git_first_last", None),
        NONE_TEXT,
    );
    write_entry(&shell.marker(repo.path(), "git"), "");
    let down = shell.prompt(repo.path(), &[]);
    assert!(down.stdout.contains(NONE_TEXT), "no agent: {}", down.stdout);
    assert!(
        !down.stdout.contains("main"),
        "no agent: oneshot drawn: {}",
        down.stdout
    );

    let agent = agent_answering(&shell, "git", Duration::from_millis(500));
    let slow = shell.prompt(repo.path(), &[]);
    assert!(
        slow.stdout.contains(NONE_TEXT),
        "slow agent: {}",
        slow.stdout
    );
    assert!(
        !slow.stdout.contains(AGENT_TEXT),
        "slow agent waited for: {}",
        slow.stdout
    );
    assert_eq!(
        agent.with_op("git").len(),
        1,
        "one bounded request: {:?}",
        agent.requests()
    );
}

/// Another position's entry (`git_last`) or another context's
/// (`git_first_last.red`) is a cold miss, not a fallback: never drawn;
/// with no agent the oneshot draws git (#1284), with one the prompt waits
/// the cold budget (5 s here, so a 500 ms reply lands) and draws it.
#[test]
#[serial]
fn git_other_variants_are_a_cold_miss() {
    for (variant, prev_bg) in [("git_last", "black"), ("git_first_last", "red")] {
        let (shell, repo) = git_shell();
        let other = format!("OTHER-{variant}-{prev_bg}");
        write_entry(
            &shell.cache_file(repo.path(), variant, Some(prev_bg)),
            &other,
        );
        write_entry(&shell.marker(repo.path(), "git"), "");

        let down = shell.prompt(repo.path(), &[]);
        assert!(
            !down.stdout.contains(&other),
            "{other} drawn: {}",
            down.stdout
        );
        assert!(
            down.stdout.contains("main"),
            "{other}: no oneshot: {}",
            down.stdout
        );

        let agent = agent_answering(&shell, "git", Duration::from_millis(500));
        let up = shell.prompt(repo.path(), &[]);
        assert!(
            up.stdout.contains(AGENT_TEXT),
            "{other}: cold reply not drawn: {}",
            up.stdout
        );
        let request = agent
            .with_op("git")
            .into_iter()
            .next()
            .expect("a git request");
        assert_eq!(field(&request, "prev_bg"), "black", "{other}: {request}");
    }
}
