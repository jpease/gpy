//! SIGURG doorbell delivery from `ClientDirectory::notify_repaint`.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

#[path = "common/sigurg.rs"]
mod sigurg;

#[cfg(unix)]
mod unix {
    use super::sigurg::SigurgCounter;
    use gpy_agent::ipc::ClientDirectory;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn notify_repaint_invokes_doorbell_handler_for_registered_client() {
        let doorbell = SigurgCounter::install();

        let shell_dir = tempfile::TempDir::new().expect("temp shell dir");
        let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
        let pid = std::process::id();
        let cwd = std::env::current_dir().expect("current dir");
        registry.register(pid, Some(cwd.clone()));

        registry.notify_repaint(Some(&cwd));

        let mut waited = 0_u32;
        while doorbell.count() == 0 && waited < 10 {
            thread::sleep(Duration::from_millis(10));
            waited += 1;
        }

        assert!(doorbell.count() > 0, "SIGURG doorbell handler should run");
        assert!(
            !shell_dir.path().join(format!("{pid}.reload")).exists(),
            "repaint must not write a reload flag"
        );
    }
}
