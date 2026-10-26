//! Range-checked process ID for registered IPC clients.

use std::fmt;

/// Strongly-typed process ID for registered clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientPid(u32);

impl ClientPid {
    /// Largest plausible process ID (Linux's default `pid_max` ceiling).
    const MAX: u32 = 4_194_304;

    /// Create a new `ClientPid` after validation.
    ///
    /// Range check only; liveness is probed once at IPC route time.
    ///
    /// # Errors
    /// Returns an error if the PID is 0 or above the plausible maximum.
    pub fn new(pid: u32) -> crate::Result<Self> {
        if Self::pid_in_range(pid) {
            Ok(Self(pid))
        } else {
            Err(crate::Error::invalid(format!("Invalid PID: {pid}")))
        }
    }

    /// Whether `pid` is nonzero and within the platform's maximum.
    #[must_use]
    pub const fn pid_in_range(pid: u32) -> bool {
        pid > 0 && pid <= Self::MAX
    }

    /// Get the raw PID value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ClientPid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<u32> for ClientPid {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ClientPid> for u32 {
    fn eq(&self, other: &ClientPid) -> bool {
        *self == other.0
    }
}
