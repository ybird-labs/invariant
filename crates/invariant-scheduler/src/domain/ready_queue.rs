//! The ready queue: a priority queue of jobs awaiting dispatch.
//!
//! [`ReadyQueue`] wraps a [`BinaryHeap`](std::collections::BinaryHeap) (a
//! max-heap) and orders jobs by a [`JobOrder`] comparator key that is computed
//! once at [`push`](ReadyQueue::push) and frozen for the job's lifetime in the
//! queue. Freezing matters because mutating an in-heap element's ordering is a
//! `BinaryHeap` logic error; the key is captured at insertion and never
//! recomputed.
//!
//! A [`QueuedJob`] pairs a [`Job`] with an `enqueue_seq` supplied by the
//! runtime. Ties on the comparator key are broken by `enqueue_seq` so that the
//! older job dispatches first, which makes drain order a strict total order and
//! independent of push order. That determinism is the foundation of replay, and
//! it holds only if every `enqueue_seq` in a queue is unique and monotonic; the
//! pure domain cannot enforce that, so the caller must guarantee it.

use std::collections::BinaryHeap;

use crate::domain::{Job, JobOrder};

/// A [`Job`] paired with its enqueue sequence number, ready to be queued.
///
/// The `enqueue_seq` is a per-queue, runtime-assigned ordinal used to break
/// ties between jobs with equal comparator keys, so that the job enqueued
/// earlier dispatches first. [`QueuedJob`] deliberately does not implement
/// [`Ord`]: ordering lives only on the queue's frozen key, never on the job's
/// mutable state.
#[derive(Debug, Clone)]
pub struct QueuedJob {
    job: Job,
    enqueue_seq: u128,
}

impl QueuedJob {
    /// Creates a [`QueuedJob`] from a [`Job`] and its enqueue sequence number.
    ///
    /// `enqueue_seq` must be unique and monotonically increasing within a single
    /// queue: it is the tiebreaker that makes drain order a strict total order
    /// and independent of insertion order. Reusing or decreasing a value within
    /// one queue is a caller error that the pure domain does not detect and that
    /// breaks deterministic replay.
    #[must_use]
    pub fn new(job: Job, enqueue_seq: u128) -> Self {
        Self { job, enqueue_seq }
    }

    /// Returns a reference to the wrapped [`Job`].
    #[must_use]
    pub fn job(&self) -> &Job {
        &self.job
    }

    /// Returns the enqueue sequence number used as the dispatch tiebreaker.
    #[must_use]
    pub fn enqueue_seq(&self) -> u128 {
        self.enqueue_seq
    }
}

#[derive(Debug)]
struct Entry<K> {
    key: K,
    queued: QueuedJob,
}

impl<K: Ord> Ord for Entry<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| other.queued.enqueue_seq().cmp(&self.queued.enqueue_seq()))
    }
}

impl<K: Ord> PartialOrd for Entry<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord> PartialEq for Entry<K> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl<K: Ord> Eq for Entry<K> {}

/// An unbounded priority queue of jobs awaiting dispatch.
///
/// Jobs are ordered by the comparator key produced by the [`JobOrder`] `O`,
/// most-urgent-first, with ties broken so the earlier `enqueue_seq` dispatches
/// first. The key is frozen at [`push`](Self::push) and never recomputed, so
/// later mutation of a queued [`Job`] cannot corrupt the heap's ordering.
///
/// The queue has no capacity bound; admission control lives in a separate layer.
pub struct ReadyQueue<O: JobOrder> {
    heap: BinaryHeap<Entry<O::Key>>,
    order: O,
}

impl<O: JobOrder> ReadyQueue<O> {
    /// Creates an empty queue ordered by `order`.
    #[must_use]
    pub fn new(order: O) -> Self {
        Self {
            heap: BinaryHeap::new(),
            order,
        }
    }

    /// Enqueues `queued`, computing and freezing its comparator key now.
    ///
    /// The [`JobOrder`] key is read from the job exactly once, here, and stored
    /// alongside it; subsequent mutation of the job never changes its position
    /// in the queue. The queue is unbounded, so this never fails.
    ///
    /// The `enqueue_seq` carried by `queued` must be unique and monotonic within
    /// this queue (see [`QueuedJob::new`]). Determinism of drain order depends on
    /// it, and the pure domain cannot enforce it.
    pub fn push(&mut self, queued: QueuedJob) {
        let key = self.order.key(queued.job()); // compute key ONCE, freeze it
        self.heap.push(Entry { key, queued });
    }

    /// Removes and returns the highest-priority [`QueuedJob`], or [`None`] if the
    /// queue is empty.
    ///
    /// "Highest-priority" means the greatest comparator key; ties go to the
    /// smaller `enqueue_seq` (the job enqueued earlier). Draining a queue fully
    /// yields a strict total order that does not depend on push order.
    pub fn pop(&mut self) -> Option<QueuedJob> {
        self.heap.pop().map(|entry| entry.queued) // unwrap the Entry
    }

    /// Returns an iterator over the queued jobs in an unspecified order.
    ///
    /// The order is arbitrary and is **not** the dispatch order; in particular it
    /// does not reflect priority or `enqueue_seq`. Callers must not depend on it,
    /// as doing so would break deterministic replay. Use [`pop`](Self::pop) to
    /// observe dispatch order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &QueuedJob> + '_ {
        self.heap.iter().map(|entry| &entry.queued)
    }

    /// Returns the number of jobs currently in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns `true` if the queue contains no jobs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;
    use crate::domain::{JobId, Priority, TargetRef};

    /// A throwaway job with a chosen id. Its contents are irrelevant to `Entry`
    /// ordering, which reads only the frozen key and the enqueue sequence — the
    /// id varies only to prove equality ignores the inner `Job`.
    fn job_named(id: &str) -> Job {
        Job::new(
            JobId::new(id).unwrap(),
            TargetRef::new("component:greet").unwrap(),
            Priority::DEFAULT,
            None,
            None,
        )
    }

    /// An `Entry` with an explicit `u8` key and enqueue sequence, so these tests
    /// exercise the ordering in isolation from any `JobOrder` implementation.
    fn entry(key: u8, seq: u128) -> Entry<u8> {
        Entry {
            key,
            queued: QueuedJob::new(job_named("job-1"), seq),
        }
    }

    #[test]
    fn entry_ordering_truth_table() {
        // `BinaryHeap` is a MAX-heap: it pops the GREATEST, so "pops first"
        // means "compares `Greater`". Each row asserts `left.cmp(&right)`.
        let rows: &[(&str, Entry<u8>, Entry<u8>, Ordering)] = &[
            (
                "higher key pops first, regardless of seq",
                entry(5, 9),
                entry(3, 0),
                Ordering::Greater,
            ),
            (
                "equal key: older (smaller seq) pops first",
                entry(5, 0),
                entry(5, 1),
                Ordering::Greater,
            ),
            (
                "equal key, equal seq: same ordering position",
                entry(5, 7),
                entry(5, 7),
                Ordering::Equal,
            ),
        ];

        for (name, left, right, expected) in rows {
            assert_eq!(left.cmp(right), *expected, "{name}");
        }
    }

    #[test]
    fn partial_eq_follows_cmp_not_struct_fields() {
        // Equal key and seq but DIFFERENT inner jobs. A structural `derive` would
        // compare the jobs and report `!=`; the hand-written `PartialEq` follows
        // `cmp`, which never reads the job, so these compare equal. This is the
        // `a == b  <=>  cmp == Equal` contract the heap relies on.
        let a = Entry {
            key: 5u8,
            queued: QueuedJob::new(job_named("a"), 7),
        };
        let b = Entry {
            key: 5u8,
            queued: QueuedJob::new(job_named("b"), 7),
        };
        assert_eq!(a, b);
    }

    mod props {
        use proptest::prelude::*;

        use super::*; // brings ReadyQueue, QueuedJob, Entry, and the parent's job_named
        use crate::domain::{
            ByPriority, Deadline, JobId, JobOrder, Priority, SchedulerTime, TargetRef,
        };

        /// One generated batch: `(enqueue_seq, priority, deadline_ms)` rows.
        type Batch = Vec<(u128, u8, Option<u64>)>;

        fn make_job(id: &str, priority: u8, deadline_ms: Option<u64>) -> Job {
            let deadline =
                deadline_ms.map(|ms| Deadline::new(SchedulerTime::from_millis_since_epoch(ms)));
            Job::new(
                JobId::new(id).unwrap(),
                TargetRef::new("component:greet").unwrap(),
                Priority::new(priority),
                deadline,
                None, // ready_at irrelevant to ordering
            )
        }

        fn items() -> impl Strategy<Value = Vec<(u128, u8, Option<u64>)>> {
            prop::collection::vec((any::<u8>(), prop::option::of(any::<u64>())), 0..32).prop_map(
                |rows| {
                    rows.into_iter()
                        .enumerate()
                        .map(|(i, (priority, deadline))| (i as u128, priority, deadline))
                        .collect()
                },
            )
        }

        /// Builds a queue by pushing `order` front-to-back, then drains it fully
        /// via `pop`, returning the popped `enqueue_seq`s in pop order. This is
        /// the system under test for the ordering properties.
        fn drain_in_push_order(order: &[(u128, u8, Option<u64>)]) -> Vec<u128> {
            let mut queue = ReadyQueue::new(ByPriority);
            for (seq, priority, deadline) in order {
                let job = make_job(&format!("job-{seq}"), *priority, *deadline);
                queue.push(QueuedJob::new(job, *seq));
            }
            let mut popped = Vec::new();
            while let Some(job) = queue.pop() {
                popped.push(job.enqueue_seq());
            }
            popped
        }

        /// A batch paired with a shuffled permutation of itself: the SAME
        /// `(seq, priority, deadline)` multiset in two different push orders.
        /// Each row travels whole, so only insertion order differs.
        fn shuffled_pair() -> impl Strategy<Value = (Batch, Batch)> {
            items()
                .prop_flat_map(|original| (Just(original.clone()), Just(original).prop_shuffle()))
        }

        #[test]
        fn empty_pop_is_none() {
            let mut queue = ReadyQueue::new(ByPriority);
            assert!(queue.pop().is_none());
            assert!(queue.is_empty());
            assert_eq!(queue.len(), 0);
        }

        proptest! {
            #[test]
            fn pop_order_matches_sorted_oracle(batch in items()) {
                let popped = drain_in_push_order(&batch);

                // Oracle: compute each frozen key ONCE via the (trusted) comparator,
                // then sort by key DESC, seq ASC. Never touches a second heap, so
                // the heap is genuinely isolated as the thing under test.
                let mut expected: Vec<(u128, _)> = batch
                    .iter()
                    .map(|(seq, priority, deadline)| {
                        (*seq, ByPriority.key(&make_job("oracle", *priority, *deadline)))
                    })
                    .collect();
                expected.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                let expected_seqs: Vec<u128> = expected.iter().map(|(seq, _)| *seq).collect();

                prop_assert_eq!(popped, expected_seqs);
            }

            #[test]
            fn pop_order_is_independent_of_push_order((original, shuffled) in shuffled_pair()) {
                // Deterministic replay: the same set pushed in any order drains
                // identically. Oracle-free — relies on no comparator assumption,
                // so it can't inherit a bug from the key.
                prop_assert_eq!(drain_in_push_order(&original), drain_in_push_order(&shuffled));
            }

            #[test]
            fn len_and_is_empty_track_push_and_pop(batch in items()) {
                let mut queue = ReadyQueue::new(ByPriority);
                prop_assert!(queue.is_empty());
                prop_assert_eq!(queue.len(), 0);

                for (pushed, (seq, priority, deadline)) in batch.iter().enumerate() {
                    queue.push(QueuedJob::new(make_job("j", *priority, *deadline), *seq));
                    prop_assert_eq!(queue.len(), pushed + 1);
                    prop_assert!(!queue.is_empty());
                }

                for remaining in (1..=batch.len()).rev() {
                    prop_assert_eq!(queue.len(), remaining);
                    queue.pop();
                }
                prop_assert!(queue.is_empty());
                prop_assert_eq!(queue.len(), 0);
            }

            #[test]
            fn drain_preserves_the_pushed_seq_set(batch in items()) {
                // No-loss: nothing dropped, nothing invented. Seqs are unique, so
                // the multiset is a set. This is what later proves a re-enqueued
                // aging duplicate is never silently lost.
                let pushed: std::collections::BTreeSet<u128> =
                    batch.iter().map(|(seq, _, _)| *seq).collect();
                let popped: std::collections::BTreeSet<u128> =
                    drain_in_push_order(&batch).into_iter().collect();
                prop_assert_eq!(pushed, popped);
            }
        }
    }
}
