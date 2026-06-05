//! Slot supply: how many jobs the runtime may run at once.
//!
//! A *slot* is permission for exactly one job to be running concurrently. The
//! pull-loop reads the slot supply to decide how many jobs it may dispatch
//! before it must wait for a running one to finish.

// TODO: remove once the runtime loop constructs `FixedSlots`.
#![allow(dead_code)]

use std::num::NonZeroUsize;

/// A constant concurrency cap: the loop may run up to this many jobs at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FixedSlots(NonZeroUsize);

impl FixedSlots {
    #[must_use]
    pub(crate) const fn new(slots: NonZeroUsize) -> Self {
        Self(slots)
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_returns_the_configured_capacity() {
        let slots = FixedSlots::new(NonZeroUsize::new(4).unwrap());
        assert_eq!(slots.get(), 4);
    }
}
