//! Helpers for driving `gpy` through a real pty: open one, capture what the
//! child writes, and wait on output or exit with a deadline.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test helper fails the calling test by panicking"
)]

use super::wait::wait_until;
use portable_pty::{Child, ExitStatus, PtyPair, PtySize, native_pty_system};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How long a pty child gets to exit once it has been told to.
pub const EXIT_DEADLINE: Duration = Duration::from_secs(10);
/// How long a pty child gets to print an expected marker.
pub const OUTPUT_DEADLINE: Duration = Duration::from_secs(10);

const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Everything the pty child has written so far.
pub type Captured = Arc<Mutex<Vec<u8>>>;

/// Open a default-sized pty and clone its master's reader.
pub fn open_pty() -> (PtyPair, Box<dyn Read + Send>) {
    let pair = native_pty_system()
        .openpty(PtySize::default())
        .expect("open pty");
    let reader = pair.master.try_clone_reader().expect("clone pty reader");
    (pair, reader)
}

/// Drain `reader` on a background thread until EOF (every copy of the pty
/// slave fd, including the child's, has closed).
pub fn spawn_reader_thread(mut reader: Box<dyn Read + Send>) -> Captured {
    let captured = Captured::default();
    let sink = Arc::clone(&captured);
    thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink
                    .lock()
                    .expect("captured output lock")
                    .extend_from_slice(buf.get(..n).expect("read() never returns n > buf.len()")),
            }
        }
    });
    captured
}

/// Whether the child has written `needle`.
pub fn contains(captured: &Captured, needle: &[u8]) -> bool {
    captured
        .lock()
        .expect("captured output lock")
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Wait until the child has written `needle`, panicking with everything it
/// wrote if `deadline` passes first.
pub fn wait_for_output(captured: &Captured, needle: &[u8], deadline: Duration) {
    assert!(
        wait_until(deadline, POLL_INTERVAL, || contains(captured, needle)),
        "did not observe {:?} within {deadline:?}; captured: {:?}",
        String::from_utf8_lossy(needle),
        String::from_utf8_lossy(&captured.lock().expect("captured output lock"))
    );
}

/// The child's exit status if it exits within `deadline`.
pub fn exit_within(child: &mut dyn Child, deadline: Duration) -> Option<ExitStatus> {
    let mut status = None;
    let exited = wait_until(deadline, POLL_INTERVAL, || {
        status = child.try_wait().expect("poll child exit status");
        status.is_some()
    });
    if exited { status } else { None }
}

/// Wait for the child to exit, panicking if it is still running after
/// `deadline`.
pub fn wait_for_exit(child: &mut dyn Child, deadline: Duration) -> ExitStatus {
    exit_within(child, deadline)
        .unwrap_or_else(|| panic!("pty child did not exit within {deadline:?}"))
}
