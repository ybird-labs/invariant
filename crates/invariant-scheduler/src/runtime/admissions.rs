//! Admission control: whether the local execution buffer may accept another
//! job, given its current occupancy and a fixed capacity bound.

// TODO: remove once the runtime loop constructs `Capacity`.
#![allow(dead_code)]

use std::num::NonZeroUsize;

use crate::domain::DomainError;

/// A fixed upper bound on how many jobs the local execution buffer may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capacity(NonZeroUsize);

impl Capacity {
    #[must_use]
    pub(crate) const fn new(capacity: NonZeroUsize) -> Self {
        Self(capacity)
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0.get()
    }

    /// Admits one job into a buffer currently holding `occupancy` jobs.
    ///
    /// # Errors
    /// Returns [`DomainError::QueueFull`] when the buffer is already at
    /// capacity (`occupancy >= capacity`), rejecting the incoming job.
    pub(crate) const fn admit(self, occupancy: usize) -> Result<(), DomainError> {
        if occupancy < self.get() {
            Ok(())
        } else {
            Err(DomainError::QueueFull)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_below_capacity_and_rejects_at_or_above() {
        let capacity = Capacity::new(NonZeroUsize::new(4).unwrap());
        assert_eq!(capacity.admit(3), Ok(()));
        assert_eq!(capacity.admit(4), Err(DomainError::QueueFull));
        assert_eq!(capacity.admit(5), Err(DomainError::QueueFull));
    }
}
