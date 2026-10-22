//! SIGURG doorbell delivery from `ClientDirectory::notify_repaint`.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(
    unsafe_code,
    reason = "installing and restoring a SIGURG handler needs `nix::sys::signal::sigaction`, an unsafe fn"
)]

#[cfg(unix)]
mod unix {
    use gpy_agent::ipc::ClientDirectory;
    use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    static HANDLED: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle_doorbell(_signal: i32) {
        HANDLED.store(true, Ordering::Relaxed);
    }

    #[test]
    fn notify_repaint_invokes_doorbell_handler_for_registered_client() {
        HANDLED.store(false, Ordering::Relaxed);

        let handler = SigAction::new(
            SigHandler::Handler(handle_doorbell),
            SaFlags::empty(),
            SigSet::empty(),
        );
        // SAFETY: the handler only does a relaxed store to a static atomic, which is
        // async-signal-safe, and this is the only test in this binary, so nothing
        // else changes the SIGURG disposition while it is installed.
        let previous = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install handler");

        let shell_dir = tempfile::TempDir::new().expect("temp shell dir");
        let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
        let pid = std::process::id();
        let cwd = std::env::current_dir().expect("current dir");
        registry.register(pid, Some(cwd.clone()));

        registry.notify_repaint(Some(&cwd));

        let mut waited = 0_u32;
        while !HANDLED.load(Ordering::Relaxed) && waited < 10 {
            thread::sleep(Duration::from_millis(10));
            waited += 1;
        }

        assert!(
            HANDLED.load(Ordering::Relaxed),
            "SIGURG doorbell handler should run"
        );
        assert!(
            !shell_dir.path().join(format!("{pid}.reload")).exists(),
            "repaint must not write a reload flag"
        );

        // SAFETY: reinstalls the disposition saved at install time, from the same test.
        unsafe {
            sigaction(Signal::SIGURG, &previous).expect("restore handler");
        }
    }
}
