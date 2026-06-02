use std::time::Duration;

use crate::domain::error::DomainError;

/// A point in time on the scheduler's timeline.
///
/// `SchedulerTime` is measured as elapsed duration since the scheduler epoch.
/// Unlike `std::time::Instant`, it is a domain value rather than a process-local
/// clock reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchedulerTime(Duration);

impl SchedulerTime {
    /// Creates a time from a [`Duration`] elapsed since the scheduler epoch.
    pub fn from_duration_since_epoch(value: Duration) -> Self {
        Self(value)
    }

    /// Creates a time from milliseconds elapsed since the scheduler epoch.
    pub fn from_millis_since_epoch(value: u64) -> Self {
        Self(Duration::from_millis(value))
    }

    /// Creates a time from seconds elapsed since the scheduler epoch.
    pub fn from_secs_since_epoch(value: u64) -> Self {
        Self(Duration::from_secs(value))
    }

    /// Returns the [`Duration`] elapsed since the scheduler epoch.
    pub fn duration_since_epoch(self) -> Duration {
        self.0
    }

    /// Returns this time advanced by `duration`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::TimeOverflow`] if the result would exceed the
    /// representable range.
    pub fn checked_add_duration(self, duration: Duration) -> Result<Self, DomainError> {
        self.0
            .checked_add(duration)
            .map(Self)
            .ok_or(DomainError::TimeOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_add_duration_should_return_new_scheduler_time() {
        let now = SchedulerTime::from_millis_since_epoch(10);

        let result = now.checked_add_duration(Duration::from_millis(5));

        assert_eq!(result, Ok(SchedulerTime::from_millis_since_epoch(15)));
    }

    #[test]
    fn checked_add_duration_should_reject_overflow() {
        let now = SchedulerTime::from_duration_since_epoch(Duration::MAX);

        let result = now.checked_add_duration(Duration::from_nanos(1));

        assert_eq!(result, Err(DomainError::TimeOverflow));
    }
}
