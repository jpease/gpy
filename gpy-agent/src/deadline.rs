//! A point in time after which a polling loop gives up.

use std::time::{Duration, Instant};

/// When a bounded wait runs out. A budget too large to represent as an
/// [`Instant`] never expires, rather than panicking on `Instant + Duration`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadline(Option<Instant>);

impl Deadline {
    /// A deadline `budget` from now.
    pub(crate) fn after(budget: Duration) -> Self {
        Self(Instant::now().checked_add(budget))
    }

    /// Whether the budget has been spent.
    pub(crate) fn expired(self) -> bool {
        self.0.is_some_and(|at| Instant::now() >= at)
    }
}

#[cfg(test)]
mod tests {
    use super::Deadline;
    use std::time::Duration;

    #[test]
    fn a_zero_budget_is_already_spent() {
        assert!(Deadline::after(Duration::ZERO).expired());
    }

    #[test]
    fn a_long_budget_is_not_spent() {
        assert!(!Deadline::after(Duration::from_secs(3600)).expired());
    }

    #[test]
    fn an_unrepresentable_budget_never_expires() {
        assert!(!Deadline::after(Duration::MAX).expired());
    }
}
