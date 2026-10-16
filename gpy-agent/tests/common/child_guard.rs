//! RAII guards that terminate spawned child processes when a test ends,
//! whether it returns, fails an assertion, panics, or times out.

#![allow(dead_code)]

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::process::{Child, Command, Output};

/// Kills and reaps a [`std::process::Child`] on drop.
pub struct KillOnDrop(Option<Child>);

impl KillOnDrop {
    #[must_use]
    pub fn new(child: Child) -> Self {
        Self(Some(child))
    }

    /// Wait for the child and collect its output, disarming the guard.
    ///
    /// # Errors
    ///
    /// Returns the I/O error from waiting on or reading the child.
    pub fn wait_with_output(mut self) -> std::io::Result<Output> {
        self.0
            .take()
            .expect("child is present until the guard is consumed")
            .wait_with_output()
    }
}

impl Deref for KillOnDrop {
    type Target = Child;

    fn deref(&self) -> &Child {
        self.0
            .as_ref()
            .expect("child is present until the guard is consumed")
    }
}

impl DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Child {
        self.0
            .as_mut()
            .expect("child is present until the guard is consumed")
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Kills and reaps a pty-attached child on drop.
pub struct PtyChild(Box<dyn portable_pty::Child + Send + Sync>);

impl PtyChild {
    #[must_use]
    pub fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self(child)
    }
}

impl Deref for PtyChild {
    type Target = dyn portable_pty::Child + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for PtyChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Stops a daemonised `gpy-agent` on `socket` when dropped.
///
/// `gpy-agent start` double-forks, so there is no child handle to kill; the
/// guard runs the same `stop` command a user would. `command` builds the
/// stop invocation (binary plus isolated environment) without the `--socket`
/// argument.
pub struct AgentStopGuard {
    command: Command,
    socket: PathBuf,
}

impl AgentStopGuard {
    #[must_use]
    pub fn new(command: Command, socket: PathBuf) -> Self {
        Self { command, socket }
    }
}

impl Drop for AgentStopGuard {
    fn drop(&mut self) {
        let _ = self
            .command
            .arg("stop")
            .arg("--socket")
            .arg(&self.socket)
            .status();
        let started = std::time::Instant::now();
        while self.socket.exists() && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
}
