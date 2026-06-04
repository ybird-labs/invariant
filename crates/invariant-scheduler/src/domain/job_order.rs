//! Ready-queue ordering: the pluggable comparator seam.
//!
//! A [`JobOrder`] maps a [`Job`] to an immutable comparator
//! [`Key`](JobOrder::Key) computed once at enqueue time. The ready queue stores
//! that key and orders by it, so dispatch order never depends on `Job`
//! implementing `Ord` (it deliberately does not). [`Fifo`] and [`ByPriority`]
//! are the built-in policies.

use std::cmp::Reverse;

use crate::domain::{Deadline, Job, Priority};

/// A dispatch order over jobs, expressed as an immutable comparator key.
///
/// The key is computed once when a job is enqueued and never reads mutable job
/// state afterwards: changing an in-heap element's ordering is a `BinaryHeap`
/// logic error, so freezing the key at insertion keeps the queue sound.
pub trait JobOrder: Send + Sync {
    /// The comparator key; a greater key is dispatched first.
    type Key: Ord + Send + Sync + 'static;

    /// Computes the ordering key for `job`.
    fn key(&self, job: &Job) -> Self::Key;
}

/// First-in, first-out order.
///
/// [`Fifo`] maps every job to the same [`Key`](JobOrder::Key) (`()`), so on its
/// own it imposes no order at all. A `BinaryHeap` is not stable, so equal keys do
/// not pop in insertion order: true FIFO requires the queue to break ties with an
/// enqueue sequence number (or insertion timestamp). Use this ordering only with
/// a queue that supplies such a tiebreaker.
pub struct Fifo;

impl JobOrder for Fifo {
    type Key = ();

    fn key(&self, _job: &Job) -> Self::Key {}
}

/// Fixed-priority order: most urgent [`Priority`] first, then earliest
/// [`Deadline`] as a best-effort tiebreak, with undeadlined jobs last.
pub struct ByPriority;

impl JobOrder for ByPriority {
    type Key = (Priority, Option<Reverse<Deadline>>);

    fn key(&self, job: &Job) -> Self::Key {
        // `Reverse` flips the deadline so an earlier one yields a greater key;
        // `None` stays least, so undeadlined jobs sort last within a priority.
        (job.priority(), job.deadline().map(Reverse))
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;
    use crate::domain::{Deadline, JobId, SchedulerTime, TargetRef};

    /// Builds a `Job` via the public API. `ready_at` is always `None` because
    /// it is irrelevant to ordering.
    fn job(priority: u8, deadline_ms: Option<u64>) -> Job {
        let deadline =
            deadline_ms.map(|ms| Deadline::new(SchedulerTime::from_millis_since_epoch(ms)));
        Job::new(
            JobId::new("job-1").unwrap(),
            TargetRef::new("component:greet").unwrap(),
            Priority::new(priority),
            deadline,
            None,
        )
    }

    /// Compares two jobs through an order's key, exactly as a max-heap would:
    /// a `Greater` result means `left` pops before `right`.
    fn cmp_by<O: JobOrder>(order: &O, left: &Job, right: &Job) -> Ordering {
        order.key(left).cmp(&order.key(right))
    }

    mod by_priority {
        use super::*;

        #[test]
        fn comparator_truth_table() {
            // Each row compares `ByPriority.key(&left)` against
            // `ByPriority.key(&right)` against a MAX-heap: a GREATER key pops
            // first.
            let rows: &[(&str, Job, Job, Ordering)] = &[
                (
                    "higher priority beats urgent low priority",
                    job(10, None),
                    job(2, Some(1)),
                    Ordering::Greater,
                ),
                (
                    "earlier deadline wins within a band",
                    job(5, Some(1)),
                    job(5, Some(9)),
                    Ordering::Greater,
                ),
                (
                    "deadline beats no-deadline within a band",
                    job(5, Some(1)),
                    job(5, None),
                    Ordering::Greater,
                ),
                (
                    "equal priority and deadline tie",
                    job(5, Some(1)),
                    job(5, Some(1)),
                    Ordering::Equal,
                ),
                (
                    "equal priority and both none tie",
                    job(5, None),
                    job(5, None),
                    Ordering::Equal,
                ),
            ];

            for (name, left, right, expected) in rows {
                assert_eq!(cmp_by(&ByPriority, left, right), *expected, "{name}");
            }
        }
    }

    mod fifo {
        use super::*;

        #[test]
        fn keys_are_equal_regardless_of_priority_and_deadline() {
            // `Fifo::Key` is `()`, so the comparison is unconditionally `Equal`
            // even though the jobs differ in both priority and deadline.
            assert_eq!(
                cmp_by(&Fifo, &job(10, Some(1)), &job(2, None)),
                Ordering::Equal
            );
        }
    }
}
