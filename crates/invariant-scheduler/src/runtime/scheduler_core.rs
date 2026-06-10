//! The pure synchronous scheduler core: it folds commands into actions,
//! applying admission control and slot limits without performing any I/O.
//!
//! A [`SchedulerCore`] is sans-IO. It decides and mutates only its own state and
//! returns [`Action`]s for the runtime shell to execute; the core itself touches
//! nothing outside its fields. It is the sole owner of the ready queue, the
//! enqueue-sequence counter, and the running count, so those three move in lock
//! step and need no external synchronization.

// TODO: remove once the runtime shell drives a `SchedulerCore`.
#![allow(dead_code)]

use crate::{
    domain::{Job, JobOrder, QueuedJob, ReadyQueue},
    runtime::{admissions::Capacity, slot_supplier::FixedSlots},
};

/// An input fed to a [`SchedulerCore`].
#[derive(Debug)]
pub(crate) enum Command {
    /// Offer a job for scheduling, subject to admission control.
    Submit(Job),
}

/// A decision emitted by a [`SchedulerCore`] for the runtime shell to carry out.
#[derive(Debug)]
pub(crate) enum Action {
    /// Dispatch the job to run, consuming one slot.
    Spawn(QueuedJob),
    /// Refuse the job; admission control declined it before it was enqueued.
    ///
    /// Carries the bare [`Job`] rather than a [`QueuedJob`]: a refused job never
    /// entered the queue and so was never assigned an enqueue sequence.
    Reject(Job),
}

/// The pure scheduling state machine.
///
/// A core owns the ready [`queue`](ReadyQueue), its admission [`Capacity`], and a
/// fixed [`slots`](FixedSlots) bound. It folds each [`Command`] into a list of
/// [`Action`]s, advancing its own state but performing no I/O; the runtime shell
/// interprets the actions.
///
/// `capacity` and `slots` are two distinct, independent bounds and must never be
/// conflated: `capacity` limits how many jobs may *wait* in the ready queue
/// (admission), while `slots` limits how many may be *concurrently running*
/// (dispatch). A job admitted past the first bound can still be held back by the
/// second.
pub(crate) struct SchedulerCore<O: JobOrder> {
    queue: ReadyQueue<O>,
    /// Bounds the *waiting* queue occupancy, enforced at admission.
    capacity: Capacity,
    /// Bounds the *concurrently running* count, enforced at dispatch.
    slots: FixedSlots,
    /// The next enqueue ordinal to hand to [`QueuedJob`]: per-queue unique and
    /// strictly monotonic, consumed only on successful admission. [`ReadyQueue`]
    /// relies on this uniqueness and monotonicity for its total dispatch order,
    /// so it must only ever increase, and only here.
    next_seq: u128,
    /// Jobs the core counts as running, i.e. spawned but not yet completed. Held
    /// at or below `slots.get()` at all times.
    running_count: usize,
}

impl<O: JobOrder> SchedulerCore<O> {
    #[must_use]
    pub(crate) fn new(order: O, capacity: Capacity, slots: FixedSlots) -> Self {
        Self {
            queue: ReadyQueue::new(order),
            capacity,
            slots,
            next_seq: 0,
            running_count: 0,
        }
    }

    /// Returns the number of jobs the core currently considers running.
    #[must_use]
    pub(crate) fn running_count(&self) -> usize {
        self.running_count
    }

    /// Returns the number of jobs waiting in the ready queue.
    #[must_use]
    pub(crate) fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Returns the enqueue ordinal that the next admitted job will receive.
    ///
    /// This is the per-queue unique, strictly increasing sequence that
    /// [`ReadyQueue`]/[`QueuedJob`] use as their tiebreaker; it advances only
    /// when a job is admitted (see [`on_command`](Self::on_command)).
    #[must_use]
    pub(crate) fn next_seq(&self) -> u128 {
        self.next_seq
    }

    /// Folds `command` into the actions the runtime shell must carry out.
    ///
    /// A [`Command::Submit`] consults admission against `capacity` (the *waiting*
    /// bound), then dispatches against `slots` (the *running* bound):
    ///
    /// - If the queue is at capacity the job is refused with a single
    ///   [`Action::Reject`] and *all state is left unchanged*: no sequence is
    ///   consumed and nothing is enqueued. Callers rely on this no-op guarantee.
    /// - Otherwise the job is assigned the next enqueue sequence, pushed onto the
    ///   ready queue, and the core dispatches as many jobs as free slots allow,
    ///   returning one [`Action::Spawn`] per dispatch (possibly zero).
    ///
    /// # Panics
    ///
    /// Panics if the enqueue sequence would overflow on admission, the guard
    /// against aliasing a live sequence. Practically unreachable: reaching it
    /// would take `u128::MAX` admissions.
    #[must_use = "dropping the actions leaks reserved slots"]
    pub(crate) fn on_command(&mut self, command: Command) -> Vec<Action> {
        match command {
            Command::Submit(job) => match self.capacity.admit(self.queue.len()) {
                Err(_) => vec![Action::Reject(job)],
                Ok(()) => {
                    // A wrap here would alias a live enqueue sequence and corrupt
                    // ReadyQueue's total order; the u128 space makes this
                    // unreachable in practice, so we assert rather than handle it.
                    assert!(self.next_seq < u128::MAX);
                    self.queue.push(QueuedJob::new(job, self.next_seq));
                    self.next_seq += 1;
                    self.fill()
                }
            },
        }
    }

    /// Dispatches queued jobs into free slots, one [`Action::Spawn`] each.
    fn fill(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        while self.running_count < self.slots.get() {
            let Some(queued) = self.queue.pop() else {
                break;
            };
            self.running_count += 1;
            actions.push(Action::Spawn(queued));
        }
        // Safety: never dispatch beyond the slot bound. The loop condition
        // guarantees it; this guards against a future edit breaking it.
        assert!(self.running_count <= self.slots.get());
        // Work-conserving: never leave a slot idle while a job waits. The loop
        // exits only on a full slot count or a drained queue, so one disjunct
        // always holds.
        assert!(self.running_count == self.slots.get() || self.queue.is_empty());
        actions
    }

    /// Records that one running job has reached a terminal outcome, freeing its
    /// slot, and dispatches into the slot just freed.
    ///
    /// The core counts slots, not identities: it neither knows nor needs to know
    /// *which* job finished or *how* it ended. Success, failure, and
    /// cancellation are accounted identically — each frees exactly one slot. The
    /// caller is therefore obligated to emit exactly one completion per
    /// [`Action::Spawn`] it carried out, on every exit path (including panic and
    /// cancel); under- or over-reporting corrupts the running count.
    ///
    /// Frees the slot and then re-enters [`fill`](Self::fill), the single
    /// dispatch chokepoint, so the freed slot goes to the highest-priority
    /// waiter (possibly none, yielding no actions).
    ///
    /// # Panics
    ///
    /// Panics if no job is running. A completion with `running_count == 0` is a
    /// structural double-free of a slot, not a runtime condition: it can only
    /// mean the caller violated the one-completion-per-spawn contract above.
    #[must_use = "dropping the actions leaks reserved slots"]
    pub(crate) fn on_completion(&mut self) -> Vec<Action> {
        assert!(self.running_count > 0);
        self.running_count -= 1;
        self.fill()
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::domain::{ByPriority, JobId, Priority, TargetRef};

    /// A throwaway job with a chosen id; its other fields are irrelevant to the
    /// scheduling decisions exercised here.
    fn job(id: &str) -> Job {
        Job::new(
            JobId::new(id).unwrap(),
            TargetRef::new("component:greet").unwrap(),
            Priority::DEFAULT,
            None,
            None,
        )
    }

    /// A throwaway job at a chosen priority, for exercising refill ordering.
    fn prioritized(id: &str, priority: u8) -> Job {
        Job::new(
            JobId::new(id).unwrap(),
            TargetRef::new("component:greet").unwrap(),
            Priority::new(priority),
            None,
            None,
        )
    }

    fn capacity(n: usize) -> Capacity {
        Capacity::new(NonZeroUsize::new(n).unwrap())
    }

    fn slots(n: usize) -> FixedSlots {
        FixedSlots::new(NonZeroUsize::new(n).unwrap())
    }

    fn core(cap: usize, slot: usize) -> SchedulerCore<ByPriority> {
        SchedulerCore::new(ByPriority, capacity(cap), slots(slot))
    }

    #[test]
    fn submits_up_to_slot_limit_spawn_immediately() {
        let mut core = core(64, 2);

        let first = core.on_command(Command::Submit(job("job-1")));
        let second = core.on_command(Command::Submit(job("job-2")));

        assert!(matches!(first.as_slice(), [Action::Spawn(_)]));
        assert!(matches!(second.as_slice(), [Action::Spawn(_)]));
        assert_eq!(core.running_count(), 2);
        assert_eq!(core.queue_len(), 0);
    }

    #[test]
    fn submit_beyond_slots_enqueues_without_spawning() {
        let mut core = core(64, 1);

        let first = core.on_command(Command::Submit(job("job-1")));
        let second = core.on_command(Command::Submit(job("job-2")));

        assert!(matches!(first.as_slice(), [Action::Spawn(_)]));
        assert!(second.is_empty());
        assert_eq!(core.running_count(), 1);
        assert_eq!(core.queue_len(), 1);
        assert_eq!(core.next_seq(), 2);
    }

    #[test]
    fn submit_at_capacity_rejects_and_leaves_state_unchanged() {
        // slots=1, queue bound=1: one running, one queued, then the buffer is full.
        let mut core = core(1, 1);

        let running = core.on_command(Command::Submit(job("job-1")));
        let queued = core.on_command(Command::Submit(job("job-2")));
        assert!(matches!(running.as_slice(), [Action::Spawn(_)]));
        assert!(queued.is_empty());
        assert_eq!(core.running_count(), 1);
        assert_eq!(core.queue_len(), 1);
        assert_eq!(core.next_seq(), 2);

        let rejected = core.on_command(Command::Submit(job("job-3")));

        assert!(matches!(rejected.as_slice(), [Action::Reject(_)]));
        assert_eq!(core.running_count(), 1);
        assert_eq!(core.queue_len(), 1);
        assert_eq!(core.next_seq(), 2);
    }

    #[test]
    fn rejected_submit_carries_the_bare_job() {
        let mut core = core(1, 1);
        let _ = core.on_command(Command::Submit(job("job-1")));
        let _ = core.on_command(Command::Submit(job("job-2")));

        let rejected = core.on_command(Command::Submit(job("job-3")));

        match rejected.as_slice() {
            [Action::Reject(rejected_job)] => assert_eq!(*rejected_job, job("job-3")),
            other => panic!("expected a single Reject, got {other:?}"),
        }
    }

    #[test]
    fn seq_only_advances_on_admission() {
        let mut core = core(1, 1);
        let _ = core.on_command(Command::Submit(job("job-1"))); // admitted, spawned
        let _ = core.on_command(Command::Submit(job("job-2"))); // admitted, queued
        assert_eq!(core.next_seq(), 2);

        let _ = core.on_command(Command::Submit(job("job-3"))); // rejected

        assert_eq!(core.next_seq(), 2);
    }

    #[test]
    fn completion_refills_the_next_queued_job() {
        let mut core = core(64, 1);
        let _ = core.on_command(Command::Submit(job("running")));
        let _ = core.on_command(Command::Submit(job("queued")));

        let refilled = core.on_completion();

        assert!(matches!(refilled.as_slice(), [Action::Spawn(_)]));
        assert_eq!(core.running_count(), 1);
        assert_eq!(core.queue_len(), 0);
    }

    #[test]
    fn completion_refills_the_highest_priority_waiter() {
        let mut core = core(64, 1);
        let _ = core.on_command(Command::Submit(prioritized("running", 100)));
        let _ = core.on_command(Command::Submit(prioritized("low", 10)));
        let _ = core.on_command(Command::Submit(prioritized("high", 200)));

        let refilled = core.on_completion();
        match refilled.as_slice() {
            [Action::Spawn(spawned)] => assert_eq!(spawned.job().id().as_str(), "high"),
            other => panic!("expected the high-priority waiter to spawn, got {other:?}"),
        }

        let next = core.on_completion();
        match next.as_slice() {
            [Action::Spawn(spawned)] => assert_eq!(spawned.job().id().as_str(), "low"),
            other => panic!("expected the low-priority waiter to spawn, got {other:?}"),
        }
    }

    #[test]
    fn completion_refills_exactly_one_waiter() {
        let mut core = core(64, 2);
        for i in 0..5 {
            let _ = core.on_command(Command::Submit(job(&format!("job-{i}"))));
        }
        assert_eq!(core.running_count(), 2);
        assert_eq!(core.queue_len(), 3);

        let refilled = core.on_completion();

        assert_eq!(refilled.len(), 1);
        assert_eq!(core.running_count(), 2);
        assert_eq!(core.queue_len(), 2);
    }

    #[test]
    fn completion_with_empty_queue_frees_the_slot() {
        let mut core = core(64, 2);
        let _ = core.on_command(Command::Submit(job("solo")));

        let freed = core.on_completion();

        assert!(freed.is_empty());
        assert_eq!(core.running_count(), 0);
    }

    #[test]
    fn completions_conserve_slots_and_drain_to_zero() {
        let (submitted, slot) = (5usize, 2usize);
        let mut core = core(64, slot);
        let mut spawned = 0;
        for i in 0..submitted {
            spawned += core
                .on_command(Command::Submit(job(&format!("job-{i}"))))
                .len();
        }

        let mut completed = 0;
        while core.running_count() > 0 {
            spawned += core.on_completion().len();
            completed += 1;
            assert_eq!(core.running_count(), (submitted - completed).min(slot));
        }

        assert_eq!(spawned, submitted);
        assert_eq!(core.queue_len(), 0);
        assert_eq!(core.running_count(), 0);
    }

    #[test]
    #[should_panic(expected = "running_count > 0")]
    fn completion_without_running_jobs_panics() {
        let _ = core(1, 1).on_completion();
    }

    mod props {
        use proptest::prelude::*;

        use super::*;

        #[derive(Debug, Clone, Copy)]
        enum Op {
            Submit,
            Complete,
        }

        /// Draws the submit weight per case so some cases are submit-heavy
        /// bursts: with a fixed 1:2 Submit:Complete mix, small capacities
        /// almost never overflow and the reject path goes untested.
        fn ops() -> impl Strategy<Value = Vec<Op>> {
            (1u32..=8).prop_flat_map(|submit_weight| {
                prop::collection::vec(
                    prop_oneof![
                        submit_weight => Just(Op::Submit),
                        2 => Just(Op::Complete),
                    ],
                    0..64,
                )
            })
        }

        proptest! {
            #[test]
            fn slot_accounting_holds_under_any_interleaving(
                cap in 1usize..8,
                slot in 1usize..4,
                ops in ops(),
            ) {
                let mut core = core(cap, slot);
                // Pure INPUT counts: never derived from emitted actions.
                let (mut submitted, mut completed) = (0usize, 0usize);
                // Independent emission-balance oracles.
                let (mut spawned, mut rejected) = (0usize, 0usize);

                for (i, op) in ops.iter().enumerate() {
                    match op {
                        Op::Submit => {
                            let before = (core.running_count(), core.queue_len(), core.next_seq());
                            let actions = core.on_command(Command::Submit(job(&format!("job-{i}"))));
                            submitted += 1;
                            if actions.iter().any(|a| matches!(a, Action::Reject(_))) {
                                prop_assert!(matches!(actions.as_slice(), [Action::Reject(_)]));
                                rejected += 1;
                                // No-op guarantee: a rejection changes nothing.
                                let after = (core.running_count(), core.queue_len(), core.next_seq());
                                prop_assert_eq!(after, before);
                            } else {
                                prop_assert!(actions.iter().all(|a| matches!(a, Action::Spawn(_))));
                                spawned += actions.len();
                            }
                        }
                        // Gate on the system's own truth, not a derived shadow.
                        Op::Complete if core.running_count() > 0 => {
                            spawned += core.on_completion().len();
                            completed += 1;
                        }
                        Op::Complete => {}
                    }

                    let admitted = submitted - rejected;
                    let expected_running = (admitted - completed).min(slot);
                    prop_assert!(core.running_count() <= slot);
                    prop_assert_eq!(core.running_count(), expected_running);
                    prop_assert!(core.running_count() == slot || core.queue_len() == 0);
                    prop_assert!(core.queue_len() <= cap);
                }

                while core.running_count() > 0 {
                    spawned += core.on_completion().len();
                    completed += 1;

                    let admitted = submitted - rejected;
                    let expected_running = (admitted - completed).min(slot);
                    prop_assert!(core.running_count() <= slot);
                    prop_assert_eq!(core.running_count(), expected_running);
                    prop_assert!(core.running_count() == slot || core.queue_len() == 0);
                    prop_assert!(core.queue_len() <= cap);
                }

                // Conservation at quiescence: every submit was either spawned
                // (and completed) or rejected; nothing was lost or duplicated.
                prop_assert_eq!(core.running_count(), 0);
                prop_assert_eq!(core.queue_len(), 0);
                prop_assert_eq!(spawned + rejected, submitted);
                prop_assert_eq!(completed, spawned);
            }
        }

        /// Coverage guard for the property above: drives the smallest core to a
        /// guaranteed [`Action::Reject`], so the reject path cannot silently
        /// become dead if the proptest op distribution shifts.
        #[test]
        fn reject_path_is_reachable_at_minimum_capacity() {
            let mut core = core(1, 1);
            let running = core.on_command(Command::Submit(job("running")));
            let waiting = core.on_command(Command::Submit(job("waiting")));
            assert!(matches!(running.as_slice(), [Action::Spawn(_)]));
            assert!(waiting.is_empty());

            let overflow = core.on_command(Command::Submit(job("overflow")));

            assert!(matches!(overflow.as_slice(), [Action::Reject(_)]));
        }
    }
}
