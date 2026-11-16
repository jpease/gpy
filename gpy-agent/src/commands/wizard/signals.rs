//! Termination signals for the wizard. Raw mode leaves the terminal broken if
//! the process dies without restoring it, so SIGINT, SIGTERM and SIGHUP set a
//! flag the event loop polls, and the loop leaves through the normal path.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A flag that becomes `true` once the process is asked to terminate.
///
/// If the handlers cannot be installed the flag never fires and the default
/// signal behaviour stays in place.
#[cfg(unix)]
pub fn watch() -> Arc<AtomicBool> {
    use std::sync::atomic::Ordering;
    use tokio::signal::unix::{SignalKind, signal};

    let flag = Arc::new(AtomicBool::new(false));
    let watcher_flag = Arc::clone(&flag);
    let spawned = std::thread::Builder::new()
        .name("wizard-signals".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let (Ok(mut term), Ok(mut hup), Ok(mut int)) = (
                    signal(SignalKind::terminate()),
                    signal(SignalKind::hangup()),
                    signal(SignalKind::interrupt()),
                ) else {
                    return;
                };
                tokio::select! {
                    _ = term.recv() => {}
                    _ = hup.recv() => {}
                    _ = int.recv() => {}
                }
                watcher_flag.store(true, Ordering::SeqCst);
            });
        });
    // A failed spawn only loses signal handling.
    drop(spawned);
    flag
}

/// A flag that never fires: terminal signals are only handled on Unix.
#[cfg(not(unix))]
pub fn watch() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
