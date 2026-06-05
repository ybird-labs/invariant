//! Scheduling value objects for the local execution layer.
//!
//! Immutable, `Copy` newtypes in the pure domain: priority, deadline, and readiness gate.

use crate::domain::SchedulerTime;

/// Scheduling priority where a higher value is more urgent.
///
/// Priority is a `u8`, giving 256 levels ordered from [`MIN`](Self::MIN)
/// (least urgent) to [`MAX`](Self::MAX) (most urgent).
///
/// # Examples
///
/// ```
/// use invariant_scheduler::domain::Priority;
///
/// assert!(Priority::MAX > Priority::DEFAULT);
/// assert_eq!(Priority::new(200).value(), 200);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority(u8);

impl Priority {
    /// Least urgent priority.
    pub const MIN: Self = Self(u8::MIN);
    /// Most urgent priority.
    pub const MAX: Self = Self(u8::MAX);
    /// Neutral baseline at the middle of the range.
    pub const DEFAULT: Self = Self(128);

    /// Creates a priority from a raw level, where higher is more urgent.
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw priority level.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl Default for Priority {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The point by which a job should finish.
///
/// A deadline is best-effort: non-preemptive multi-worker scheduling cannot
/// guarantee it is met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Deadline(SchedulerTime);

impl Deadline {
    /// Creates a deadline at the given scheduler time.
    pub const fn new(time: SchedulerTime) -> Self {
        Self(time)
    }

    /// Returns the scheduler time at which this deadline falls.
    #[must_use]
    pub const fn time(self) -> SchedulerTime {
        self.0
    }
}

/// The earliest time a job may start.
///
/// This is a readiness gate that controls eligibility; it is not an ordering or
/// sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReadyAt(SchedulerTime);

impl ReadyAt {
    /// Creates a readiness gate at the given scheduler time.
    pub const fn new(time: SchedulerTime) -> Self {
        Self(time)
    }

    /// Returns the scheduler time before which the job is not eligible to start.
    #[must_use]
    pub const fn time(self) -> SchedulerTime {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SchedulerTime;

    #[test]
    fn priority_default_is_mid_range_baseline() {
        assert_eq!(Priority::default(), Priority::DEFAULT);
        assert_eq!(Priority::DEFAULT.value(), 128);
    }

    #[test]
    fn priority_min_max_pin_the_u8_range() {
        assert_eq!(Priority::MIN.value(), 0);
        assert_eq!(Priority::MAX.value(), 255);
    }

    #[test]
    fn priority_orders_higher_value_as_greater() {
        assert!(Priority::MAX > Priority::DEFAULT);
        assert!(Priority::DEFAULT > Priority::MIN);
    }

    #[test]
    fn deadline_round_trips_its_time() {
        let t = SchedulerTime::from_millis_since_epoch(42);
        assert_eq!(Deadline::new(t).time(), t);
    }

    #[test]
    fn ready_at_round_trips_its_time() {
        let t = SchedulerTime::from_millis_since_epoch(42);
        assert_eq!(ReadyAt::new(t).time(), t);
    }
}
