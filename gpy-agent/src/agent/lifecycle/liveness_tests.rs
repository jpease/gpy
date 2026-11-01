// `inspect_socket` retries are tested in `tests/stale_socket_tests.rs`, which
// needs `unsafe` env mutation the lib denies.
use super::{
    AgentLiveness, PING_STALE_ATTEMPTS, PingOutcome, SHUTDOWN_WAIT_MAX, ping_agent_liveness_with,
    wait_for_agent_shutdown,
};
use crate::Error;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The daemon's wire `Response::Ack`, pinned by
/// `client::tests::ack_wire_form_is_what_the_json_formatter_emits`.
const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

#[test]
fn wait_for_agent_shutdown_returns_promptly_when_dead() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("missing.sock"); // nothing listening

    let started = Instant::now();
    let stopped = wait_for_agent_shutdown(&socket_path);
    let elapsed = started.elapsed();

    assert!(
        stopped,
        "must report the agent as confirmed stopped when nothing is listening"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "must not wait the full fallback budget when the agent is already gone, took {elapsed:?}"
    );
}

/// A rebound path releases the captured socket even though something answers.
#[test]
fn wait_for_socket_release_sees_a_rebound_path_as_released() {
    use super::{ShutdownWait, wait_for_socket_release};
    use crate::ipc::socket_identity::SocketIdentity;

    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("agent.sock");
    let old = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    let identity = SocketIdentity::at(&socket_path).unwrap();
    std::fs::remove_file(&socket_path).unwrap();
    let _replacement = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    drop(old);

    let started = Instant::now();
    let outcome = wait_for_socket_release(&socket_path, identity);
    let elapsed = started.elapsed();

    assert_eq!(outcome, ShutdownWait::Released);
    assert!(
        elapsed < Duration::from_secs(1),
        "a rebound path must end the wait at once, took {elapsed:?}"
    );
}

/// Drive the classifier over scripted ping outcomes with no backoff.
fn classify(outcomes: Vec<crate::Result<PingOutcome>>) -> (AgentLiveness, usize) {
    let call_counter = RefCell::new(0_usize);
    let script = RefCell::new(outcomes.into_iter());
    let verdict = ping_agent_liveness_with(
        || {
            let mut call_count = call_counter.borrow_mut();
            *call_count = call_count.saturating_add(1);
            drop(call_count);
            script
                .borrow_mut()
                .next()
                .unwrap_or_else(|| panic!("classifier asked for more pings than scripted"))
        },
        Duration::ZERO,
    );
    let calls = *call_counter.borrow();
    (verdict, calls)
}

/// A ping that failed locally.
///
/// # Errors
/// Always.
fn local_failure() -> crate::Result<PingOutcome> {
    Err(Error::agent("simulated"))
}

#[test]
fn an_answered_ping_stops_the_round_immediately() {
    let (verdict, calls) = classify(vec![Ok(PingOutcome::Answered)]);
    assert_eq!(verdict, AgentLiveness::Responding);
    assert_eq!(calls, 1, "a responding agent must not be pinged again");
}

#[test]
fn pings_that_never_reach_the_agent_are_a_confirmed_stop() {
    let unanswered = (0..PING_STALE_ATTEMPTS)
        .map(|_| Ok(PingOutcome::NotListening))
        .collect();
    let (verdict, calls) = classify(unanswered);
    assert_eq!(verdict, AgentLiveness::NotResponding);
    assert_eq!(
        calls,
        usize::try_from(PING_STALE_ATTEMPTS).expect("PING_STALE_ATTEMPTS fits in usize")
    );
}

/// All pings accepted but unanswered: wedged, not stopped.
#[test]
fn unanswered_pings_that_all_reached_the_agent_are_accepted_no_reply() {
    let unanswered = (0..PING_STALE_ATTEMPTS)
        .map(|_| Ok(PingOutcome::AcceptedNoReply))
        .collect();
    let (verdict, calls) = classify(unanswered);
    assert_eq!(verdict, AgentLiveness::AcceptedNoReply);
    assert_eq!(
        calls,
        usize::try_from(PING_STALE_ATTEMPTS).expect("PING_STALE_ATTEMPTS fits in usize")
    );
}

/// One `NotListening` denies `AcceptedNoReply`: it may be a replaced dead agent.
#[test]
fn a_mix_of_not_listening_and_accepted_no_reply_resolves_toward_not_responding() {
    let (verdict, calls) = classify(vec![
        Ok(PingOutcome::AcceptedNoReply),
        Ok(PingOutcome::NotListening),
        Ok(PingOutcome::AcceptedNoReply),
    ]);
    assert_eq!(verdict, AgentLiveness::NotResponding);
    assert_eq!(
        calls,
        usize::try_from(PING_STALE_ATTEMPTS).expect("PING_STALE_ATTEMPTS fits in usize")
    );
}

/// A ping that never left this process is not evidence about the agent.
#[test]
fn one_local_failure_makes_the_whole_round_indeterminate() {
    let (verdict, _) = classify(vec![
        Ok(PingOutcome::NotListening),
        local_failure(),
        Ok(PingOutcome::NotListening),
    ]);
    assert_eq!(
        verdict,
        AgentLiveness::Indeterminate,
        "a round containing a ping we never managed to send cannot confirm a shutdown"
    );
}

/// A local failure also overrides `AcceptedNoReply` evidence.
#[test]
fn a_local_failure_mixed_with_accepted_no_reply_is_still_indeterminate() {
    let (verdict, _) = classify(vec![
        Ok(PingOutcome::AcceptedNoReply),
        local_failure(),
        Ok(PingOutcome::AcceptedNoReply),
    ]);
    assert_eq!(
        verdict,
        AgentLiveness::Indeterminate,
        "a round containing a ping we never managed to send cannot confirm anything, \
         even when every other attempt reached the agent"
    );
}

#[test]
fn a_round_of_only_local_failures_is_indeterminate() {
    let (verdict, _) = classify(vec![local_failure(), local_failure(), local_failure()]);
    assert_eq!(verdict, AgentLiveness::Indeterminate);
}

/// A later answer outranks an earlier local failure.
#[test]
fn a_later_answer_outranks_an_earlier_local_failure() {
    let (verdict, calls) = classify(vec![local_failure(), Ok(PingOutcome::Answered)]);
    assert_eq!(verdict, AgentLiveness::Responding);
    assert_eq!(calls, 2);
}

/// An agent that closes connections unanswered is busy, not wedged.
#[test]
fn a_round_of_closed_unanswered_pings_is_busy() {
    let closed = (0..PING_STALE_ATTEMPTS)
        .map(|_| Ok(PingOutcome::ClosedUnanswered))
        .collect();
    let (verdict, _) = classify(closed);
    assert_eq!(verdict, AgentLiveness::Busy);
}

#[test]
fn a_local_failure_outranks_busy() {
    let (verdict, _) = classify(vec![
        Ok(PingOutcome::ClosedUnanswered),
        local_failure(),
        Ok(PingOutcome::ClosedUnanswered),
    ]);
    assert_eq!(verdict, AgentLiveness::Indeterminate);
}

/// A dead agent's pending connections are reset once, then refused.
#[test]
fn a_reset_then_refused_round_is_not_busy() {
    let (verdict, _) = classify(vec![
        Ok(PingOutcome::ClosedUnanswered),
        Ok(PingOutcome::NotListening),
        Ok(PingOutcome::NotListening),
    ]);
    assert_eq!(verdict, AgentLiveness::NotResponding);
}

/// Cap on retried `ECONNABORTED` (a ping that timed out in the backlog) and
/// `EINTR` accept errors before the fake agent gives up.
const MAX_TRANSIENT_ACCEPT_ERRORS: u32 = 64;

/// What the fake agent did; relaxed atomics, since stderr writes hide the races.
struct FakeAgentCounters {
    answered: Arc<AtomicU32>,
    transient_errors: Arc<AtomicU32>,
    fatal_errno: Arc<AtomicI32>,
}

impl FakeAgentCounters {
    fn new() -> Self {
        Self {
            answered: Arc::new(AtomicU32::new(0)),
            transient_errors: Arc::new(AtomicU32::new(0)),
            fatal_errno: Arc::new(AtomicI32::new(0)),
        }
    }

    fn snapshot(&self) -> FakeAgentReport {
        FakeAgentReport {
            answered: self.answered.load(Ordering::Relaxed),
            transient_errors: self.transient_errors.load(Ordering::Relaxed),
            fatal_errno: self.fatal_errno.load(Ordering::Relaxed),
        }
    }
}

/// A reading of [`FakeAgentCounters`] taken the moment the verdict landed.
struct FakeAgentReport {
    answered: u32,
    transient_errors: u32,
    fatal_errno: i32,
}

impl std::fmt::Display for FakeAgentReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "agent answered {} pings, {} transient accept errors, fatal errno {}",
            self.answered, self.transient_errors, self.fatal_errno
        )
    }
}

impl FakeAgentReport {
    /// The premise of a "still responding" test: the agent was still there.
    fn assert_agent_stayed_up(&self) {
        assert_eq!(
            self.fatal_errno, 0_i32,
            "the fake agent stopped accepting; this run measured a departed \
             fixture, not shutdown detection ({self})"
        );
        assert!(
            self.answered > 0,
            "the fake agent answered no pings, so nothing here exercised \
             shutdown detection ({self})"
        );
    }
}

/// Serve pings on a blocking `accept()` loop until `stop` is set.
fn spawn_fake_agent(
    listener: std::os::unix::net::UnixListener,
    stop: &Arc<AtomicBool>,
    counters: &FakeAgentCounters,
) -> std::thread::JoinHandle<()> {
    let stop_flag = Arc::clone(stop);
    let answered = Arc::clone(&counters.answered);
    let transient_errors = Arc::clone(&counters.transient_errors);
    let fatal_errno = Arc::clone(&counters.fatal_errno);

    std::thread::spawn(move || {
        loop {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    if transient_errors.fetch_add(1, Ordering::Relaxed)
                        >= MAX_TRANSIENT_ACCEPT_ERRORS
                    {
                        fatal_errno.store(e.raw_os_error().unwrap_or(-1_i32), Ordering::Relaxed);
                        break;
                    }
                    continue;
                }
                Err(e) => {
                    fatal_errno.store(e.raw_os_error().unwrap_or(-1_i32), Ordering::Relaxed);
                    break;
                }
            };
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(ACK_WIRE_FORM);
            answered.fetch_add(1, Ordering::Relaxed);
        }
    })
}

// `gpy stop` must not report success while the agent still answers pings.
#[test]
fn wait_for_agent_shutdown_reports_still_responding_on_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("alive.sock");

    let listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("bind fake socket");
    let stop = Arc::new(AtomicBool::new(false));
    let counters = FakeAgentCounters::new();
    let server = spawn_fake_agent(listener, &stop, &counters);

    let started = Instant::now();
    let stopped = wait_for_agent_shutdown(&socket_path);
    let elapsed = started.elapsed();
    let report = counters.snapshot();

    stop.store(true, Ordering::Relaxed);
    let _ = std::os::unix::net::UnixStream::connect(&socket_path);
    server.join().expect("join fake server thread");

    // Premise first, so a departed fixture is not mistaken for a bug.
    report.assert_agent_stayed_up();

    assert!(
        !stopped,
        "must report the agent as still responding, not confirmed stopped ({report})"
    );
    assert!(
        elapsed >= SHUTDOWN_WAIT_MAX,
        "must wait the full bounded budget before giving up, took {elapsed:?}"
    );
}
