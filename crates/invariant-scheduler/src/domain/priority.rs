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

    pub fn new(value: u8) -> Self {
        Self(value)
    }

    pub fn value(self) -> u8 {
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
    pub fn new(time: SchedulerTime) -> Self {
        Self(time)
    }

    pub fn time(self) -> SchedulerTime {
        self.0
    }
}

/// Earliest time a job may start.
///
/// A readiness gate (eligibility) consumed by the ReadyClock, never an ordering/sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReadyAt(SchedulerTime);

impl ReadyAt {
    pub fn new(time: SchedulerTime) -> Self {
        Self(time)
    }

    pub fn time(self) -> SchedulerTime {
        self.0
    }
}
