//! [`SigurgCounter`]: counts the SIGURG doorbells the agent rings on this
//! test process. The only place the tests change a signal disposition.

#![cfg(unix)]
#![allow(
    unsafe_code,
    reason = "installing and restoring a SIGURG handler needs `nix::sys::signal::sigaction`, an unsafe fn"
)]
#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "a test helper fails the calling test by panicking"
)]

use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
use std::sync::atomic::{AtomicUsize, Ordering};

static RINGS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn count_ring(_signal: nix::libc::c_int) {
    RINGS.fetch_add(1, Ordering::Relaxed);
}

/// Counts every SIGURG the process receives while it is alive and restores
/// the previous disposition on drop, including when an assertion fails.
///
/// The disposition and the count are process-wide, so a test that holds one
/// must be `#[serial]` and hold no other. A SIGURG arriving while the handler
/// runs stays pending and runs it again, so separate rings are never merged.
pub struct SigurgCounter {
    previous: SigAction,
}

impl SigurgCounter {
    /// Reset the count to zero and install the counting handler.
    #[must_use]
    pub fn install() -> Self {
        RINGS.store(0, Ordering::Relaxed);
        let handler = SigAction::new(
            SigHandler::Handler(count_ring),
            SaFlags::empty(),
            SigSet::empty(),
        );
        // SAFETY: `count_ring` only does a relaxed `fetch_add` on a static atomic,
        // which is async-signal-safe.
        let previous =
            unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install SIGURG handler");
        Self { previous }
    }

    /// SIGURGs received since [`install`](Self::install).
    #[must_use]
    #[allow(
        clippy::unused_self,
        reason = "the count is process-wide; taking `&self` only reads it while a counter is installed"
    )]
    pub fn count(&self) -> usize {
        RINGS.load(Ordering::Relaxed)
    }
}

impl Drop for SigurgCounter {
    fn drop(&mut self) {
        // SAFETY: reinstalls the disposition `install` replaced.
        let restored = unsafe { sigaction(Signal::SIGURG, &self.previous) };
        if !std::thread::panicking() {
            restored.expect("restore SIGURG handler");
        }
    }
}
