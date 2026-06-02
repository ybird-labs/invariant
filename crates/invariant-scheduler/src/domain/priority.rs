//! Scheduling value objects for the local execution layer.
//!
//! Immutable, `Copy` newtypes in the pure domain: priority, deadline, and readiness gate.

use crate::domain::SchedulerTime;

/// Scheduling priority where a higher value is more urgent.
///
/// Coarse `u8` (256 levels) by design for v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority(u8);

impl Priority {
    /// Least urgent priority.
    pub const MIN: Self = Self(u8::MIN);
    /// Most urgent priority.
    pub const MAX: Self = Self(u8::MAX);
    /// Neutral baseline, mid-range so callers have headroom in both directions.
    pub const DEFAULT: Self = Self(128);

    pub const fn new(value: u8) -> Self {
        Self(value)
    }

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

/// Point by which a job should finish.
///
/// ATTENTION:
/// Best-effort only: non-preemptive multi-worker scheduling cannot guarantee it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Deadline(SchedulerTime);

impl Deadline {
    pub const fn new(time: SchedulerTime) -> Self {
        Self(time)
    }

    #[must_use]
    pub const fn time(self) -> SchedulerTime {
        self.0
    }
}

/// Earliest time a job may start.
///
/// A readiness gate (eligibility) consumed by the ReadyClock, never an ordering/sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReadyAt(SchedulerTime);

impl ReadyAt {
    pub const fn new(time: SchedulerTime) -> Self {
        Self(time)
    }

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
