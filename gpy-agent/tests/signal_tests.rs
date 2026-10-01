#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::default_numeric_fallback)]

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

    // # Panics
    //
    // Panics if installing or restoring the SIGURG doorbell handler fails or if the registry
    // cannot capture the current working directory.
    #[test]
    fn notify_repaint_invokes_doorbell_handler_for_registered_client() {
        HANDLED.store(false, Ordering::Relaxed);

        let handler = SigAction::new(
            SigHandler::Handler(handle_doorbell),
            SaFlags::empty(),
            SigSet::empty(),
        );
        let previous = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install handler");

        let shell_dir = tempfile::TempDir::new().expect("temp shell dir");
        let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
        let pid = std::process::id();
        let cwd = std::env::current_dir().expect("current dir");
        registry.register(pid, Some(cwd.clone()));

        registry.notify_repaint(Some(&cwd));

        let mut waited = 0;
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

        // Restore previous handler to avoid affecting other tests
        unsafe {
            sigaction(Signal::SIGURG, &previous).expect("restore handler");
        }
    }
}

#[cfg(not(unix))]
#[test]
fn notify_repaint_noop_on_non_unix() {
    // No-op placeholder to keep test suite green on non-UNIX platforms.
}
