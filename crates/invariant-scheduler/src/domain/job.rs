use crate::domain::{Deadline, DomainError, Priority, ReadyAt};

/// A non-empty identifier that uniquely names a job.
///
/// Identifiers order lexicographically by their string value.
///
/// # Examples
///
/// ```
/// use invariant_scheduler::domain::JobId;
///
/// let id = JobId::new("job-1")?;
/// assert_eq!(id.as_str(), "job-1");
/// # Ok::<(), invariant_scheduler::domain::DomainError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(String);

impl JobId {
    /// Creates a job identifier from any string-like value.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyJobId`] if `id` is empty.
    pub fn new(id: impl Into<String>) -> Result<Self, DomainError> {
        let value = id.into();
        if value.is_empty() {
            return Err(DomainError::EmptyJobId);
        }
        Ok(Self(value))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A non-empty reference to the target a job executes against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetRef(String);

impl TargetRef {
    /// Creates a target reference from any string-like value.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyTargetRef`] if `target` is empty.
    pub fn new(target: impl Into<String>) -> Result<Self, DomainError> {
        let value = target.into();
        if value.is_empty() {
            return Err(DomainError::EmptyTargetRef);
        }
        Ok(Self(value))
    }

    /// Returns the target reference as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// The zero-based index of a job's current attempt; the first attempt is zero.
///
/// A job starts at [`zero`](Self::zero) and the index is incremented by
/// [`Job::retry`]. It is an ordinal, not a count: a job on its first attempt
/// has value `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptNumber(u32);

impl AttemptNumber {
    /// Returns the index of a job's first attempt (zero), before any retry.
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Returns the raw zero-based attempt index.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Returns the next attempt number.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::AttemptOverflow`] if the current value is
    /// `u32::MAX`.
    pub fn checked_next(self) -> Result<Self, DomainError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(DomainError::AttemptOverflow)
    }
}

/// The lifecycle status of a [`Job`].
///
/// The live (non-terminal) states are [`Queued`](Self::Queued),
/// [`Running`](Self::Running), and [`Failed`](Self::Failed). The terminal
/// states, from which no transition is legal, are [`Completed`](Self::Completed),
/// [`Cancelled`](Self::Cancelled), and [`Exhausted`](Self::Exhausted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// Waiting to start. A newly created job begins here.
    Queued,
    /// Currently executing.
    Running,
    /// A run failed; the job may be retried, exhausted, or cancelled.
    Failed,
    /// Finished successfully. Terminal.
    Completed,
    /// Abandoned before completion. Terminal.
    Cancelled,
    /// Out of retry attempts after failing. Terminal.
    Exhausted,
}

impl JobStatus {
    /// Returns `true` if this is a terminal status, namely
    /// [`Completed`](Self::Completed), [`Cancelled`](Self::Cancelled), or
    /// [`Exhausted`](Self::Exhausted).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Exhausted
        )
    }
}

/// A unit of schedulable work and its lifecycle state.
///
/// A job is created in [`JobStatus::Queued`] with attempt number zero and moves
/// through its [`JobStatus`] state machine via the transition methods
/// ([`start`](Self::start), [`complete`](Self::complete), [`fail`](Self::fail),
/// [`cancel`](Self::cancel), [`exhaust`](Self::exhaust), [`retry`](Self::retry)).
///
/// # Examples
///
/// ```
/// use invariant_scheduler::domain::{Job, JobId, JobStatus, Priority, TargetRef};
///
/// let mut job = Job::new(
///     JobId::new("job-1")?,
///     TargetRef::new("component:greet")?,
///     Priority::DEFAULT,
///     None,
///     None,
/// );
/// assert_eq!(job.status(), JobStatus::Queued);
///
/// job.start()?;
/// job.complete()?;
/// assert_eq!(job.status(), JobStatus::Completed);
/// # Ok::<(), invariant_scheduler::domain::DomainError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    id: JobId,
    target: TargetRef,
    priority: Priority,
    deadline: Option<Deadline>,
    ready_at: Option<ReadyAt>,
    status: JobStatus,
    attempt: AttemptNumber,
}

impl Job {
    /// Creates a job in [`JobStatus::Queued`] with attempt number zero.
    ///
    /// `deadline` and `ready_at` are optional scheduling hints; pass `None` when
    /// the job has no completion target or readiness gate.
    pub fn new(
        id: JobId,
        target: TargetRef,
        priority: Priority,
        deadline: Option<Deadline>,
        ready_at: Option<ReadyAt>,
    ) -> Self {
        Self {
            id,
            target,
            priority,
            deadline,
            ready_at,
            attempt: AttemptNumber::zero(),
            status: JobStatus::Queued,
        }
    }

    /// Returns the job's identifier.
    #[must_use]
    pub fn id(&self) -> &JobId {
        &self.id
    }

    /// Returns the reference to the target this job executes against.
    #[must_use]
    pub fn target(&self) -> &TargetRef {
        &self.target
    }

    /// Returns the job's scheduling priority.
    #[must_use]
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Returns the job's deadline, if one was set.
    #[must_use]
    pub fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// Returns the job's readiness gate, if one was set.
    #[must_use]
    pub fn ready_at(&self) -> Option<ReadyAt> {
        self.ready_at
    }

    /// Returns the job's current lifecycle status.
    #[must_use]
    pub fn status(&self) -> JobStatus {
        self.status
    }

    /// Returns how many times the job has been attempted.
    #[must_use]
    pub fn attempt(&self) -> AttemptNumber {
        self.attempt
    }

    /// Classifies a rejected transition: terminal source vs. live-but-illegal.
    fn reject(&self) -> DomainError {
        if self.status.is_terminal() {
            DomainError::JobTerminal
        } else {
            DomainError::IllegalTransition
        }
    }

    /// Transitions the job from [`Queued`](JobStatus::Queued) to
    /// [`Running`](JobStatus::Running).
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::IllegalTransition`] if the job is in any other
    /// live status, or [`DomainError::JobTerminal`] if it is terminal. The
    /// status is left unchanged on error.
    pub fn start(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Queued => {
                self.status = JobStatus::Running;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    /// Transitions the job from [`Running`](JobStatus::Running) to the terminal
    /// [`Completed`](JobStatus::Completed).
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::IllegalTransition`] if the job is in any other
    /// live status, or [`DomainError::JobTerminal`] if it is terminal. The
    /// status is left unchanged on error.
    pub fn complete(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Running => {
                self.status = JobStatus::Completed;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    /// Transitions the job from [`Running`](JobStatus::Running) to
    /// [`Failed`](JobStatus::Failed), from which it may be retried, exhausted,
    /// or cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::IllegalTransition`] if the job is in any other
    /// live status, or [`DomainError::JobTerminal`] if it is terminal. The
    /// status is left unchanged on error.
    pub fn fail(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Running => {
                self.status = JobStatus::Failed;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    /// Transitions the job from any live status
    /// ([`Queued`](JobStatus::Queued), [`Running`](JobStatus::Running), or
    /// [`Failed`](JobStatus::Failed)) to the terminal
    /// [`Cancelled`](JobStatus::Cancelled).
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::JobTerminal`] if the job is already terminal. The
    /// status is left unchanged on error.
    pub fn cancel(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Queued | JobStatus::Running | JobStatus::Failed => {
                self.status = JobStatus::Cancelled;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    /// Transitions the job from [`Failed`](JobStatus::Failed) to the terminal
    /// [`Exhausted`](JobStatus::Exhausted), marking it as out of retries.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::IllegalTransition`] if the job is in any other
    /// live status, or [`DomainError::JobTerminal`] if it is terminal. The
    /// status is left unchanged on error.
    pub fn exhaust(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Failed => {
                self.status = JobStatus::Exhausted;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    /// Transitions the job from [`Failed`](JobStatus::Failed) back to
    /// [`Queued`](JobStatus::Queued), incrementing its attempt number.
    ///
    /// The operation is atomic: if the attempt number cannot be incremented,
    /// neither the attempt nor the status is changed.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::AttemptOverflow`] if the attempt number is already
    /// `u32::MAX`. Returns [`DomainError::IllegalTransition`] if the job is in
    /// any other live status, or [`DomainError::JobTerminal`] if it is terminal.
    /// The status is left unchanged on error.
    pub fn retry(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Failed => {
                // Bump the attempt first: the fallible step must commit
                // before any status mutation, so retry stays atomic.
                self.attempt = self.attempt.checked_next()?;
                self.status = JobStatus::Queued;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh `Job` forced into `status`. Direct field access is legal
    /// because this test module is a descendant of the `job` module.
    fn job_in(status: JobStatus) -> Job {
        let mut job = Job::new(
            JobId::new("job-1").unwrap(),
            TargetRef::new("component:greet").unwrap(),
            Priority::DEFAULT,
            None,
            None,
        );
        job.status = status;
        job
    }

    mod job_id {
        use super::*;

        #[test]
        fn rejects_empty() {
            assert_eq!(JobId::new(""), Err(DomainError::EmptyJobId));
        }

        #[test]
        fn preserves_value() {
            assert_eq!(JobId::new("job-1").unwrap().as_str(), "job-1");
        }

        #[test]
        fn orders_lexicographically() {
            assert!(JobId::new("a").unwrap() < JobId::new("b").unwrap());
        }
    }

    mod target_ref {
        use super::*;

        #[test]
        fn rejects_empty() {
            assert_eq!(TargetRef::new(""), Err(DomainError::EmptyTargetRef));
        }

        #[test]
        fn preserves_value() {
            assert_eq!(
                TargetRef::new("component:greet").unwrap().as_str(),
                "component:greet"
            );
        }
    }

    mod attempt_number {
        use super::*;

        #[test]
        fn starts_at_zero() {
            assert_eq!(AttemptNumber::zero().value(), 0);
        }

        #[test]
        fn increments() {
            assert_eq!(AttemptNumber::zero().checked_next().unwrap().value(), 1);
        }

        #[test]
        fn rejects_overflow() {
            assert_eq!(
                AttemptNumber(u32::MAX).checked_next(),
                Err(DomainError::AttemptOverflow)
            );
        }
    }

    mod job_status {
        use super::*;

        #[test]
        fn terminal_states_are_terminal() {
            assert!(JobStatus::Completed.is_terminal());
            assert!(JobStatus::Cancelled.is_terminal());
            assert!(JobStatus::Exhausted.is_terminal());
        }

        #[test]
        fn live_states_are_not_terminal() {
            assert!(!JobStatus::Queued.is_terminal());
            assert!(!JobStatus::Running.is_terminal());
            assert!(!JobStatus::Failed.is_terminal()); // retryable, the key case
        }
    }

    mod transitions {
        use super::*;

        /// Every transition the `Job` state machine exposes, so the table can
        /// drive each one uniformly.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Event {
            Start,
            Complete,
            Fail,
            Cancel,
            Exhaust,
            Retry,
        }

        /// Expected outcome of one (state, event) cell.
        #[derive(Debug, Clone, PartialEq, Eq)]
        enum Expect {
            /// Legal: succeeds and lands in this status.
            Lands(JobStatus),
            /// Rejected: returns this error and leaves the status untouched.
            Rejects(DomainError),
        }

        const ALL_STATES: [JobStatus; 6] = [
            JobStatus::Queued,
            JobStatus::Running,
            JobStatus::Failed,
            JobStatus::Completed,
            JobStatus::Cancelled,
            JobStatus::Exhausted,
        ];

        const ALL_EVENTS: [Event; 6] = [
            Event::Start,
            Event::Complete,
            Event::Fail,
            Event::Cancel,
            Event::Exhaust,
            Event::Retry,
        ];

        fn apply(job: &mut Job, event: Event) -> Result<(), DomainError> {
            match event {
                Event::Start => job.start(),
                Event::Complete => job.complete(),
                Event::Fail => job.fail(),
                Event::Cancel => job.cancel(),
                Event::Exhaust => job.exhaust(),
                Event::Retry => job.retry(),
            }
        }

        /// The complete transition relation: 6 states x 6 events = 36 cells.
        /// Legal edges name their target; every other cell is a rejection,
        /// classified `JobTerminal` from a terminal source else `IllegalTransition`.
        #[rustfmt::skip]
        const TABLE: &[(JobStatus, Event, Expect)] = {
            use DomainError::{IllegalTransition, JobTerminal};
            use Event::{Cancel, Complete, Exhaust, Fail, Retry, Start};
            use Expect::{Lands, Rejects};
            use JobStatus::{Cancelled, Completed, Exhausted, Failed, Queued, Running};
            &[
                // from Queued (live)
                (Queued,    Start,    Lands(Running)),
                (Queued,    Complete, Rejects(IllegalTransition)),
                (Queued,    Fail,     Rejects(IllegalTransition)),
                (Queued,    Cancel,   Lands(Cancelled)),
                (Queued,    Exhaust,  Rejects(IllegalTransition)),
                (Queued,    Retry,    Rejects(IllegalTransition)),
                // from Running (live)
                (Running,   Start,    Rejects(IllegalTransition)),
                (Running,   Complete, Lands(Completed)),
                (Running,   Fail,     Lands(Failed)),
                (Running,   Cancel,   Lands(Cancelled)),
                (Running,   Exhaust,  Rejects(IllegalTransition)),
                (Running,   Retry,    Rejects(IllegalTransition)),
                // from Failed (live, retryable)
                (Failed,    Start,    Rejects(IllegalTransition)),
                (Failed,    Complete, Rejects(IllegalTransition)),
                (Failed,    Fail,     Rejects(IllegalTransition)),
                (Failed,    Cancel,   Lands(Cancelled)),
                (Failed,    Exhaust,  Lands(Exhausted)),
                (Failed,    Retry,    Lands(Queued)),
                // from Completed (terminal)
                (Completed, Start,    Rejects(JobTerminal)),
                (Completed, Complete, Rejects(JobTerminal)),
                (Completed, Fail,     Rejects(JobTerminal)),
                (Completed, Cancel,   Rejects(JobTerminal)),
                (Completed, Exhaust,  Rejects(JobTerminal)),
                (Completed, Retry,    Rejects(JobTerminal)),
                // from Cancelled (terminal)
                (Cancelled, Start,    Rejects(JobTerminal)),
                (Cancelled, Complete, Rejects(JobTerminal)),
                (Cancelled, Fail,     Rejects(JobTerminal)),
                (Cancelled, Cancel,   Rejects(JobTerminal)),
                (Cancelled, Exhaust,  Rejects(JobTerminal)),
                (Cancelled, Retry,    Rejects(JobTerminal)),
                // from Exhausted (terminal)
                (Exhausted, Start,    Rejects(JobTerminal)),
                (Exhausted, Complete, Rejects(JobTerminal)),
                (Exhausted, Fail,     Rejects(JobTerminal)),
                (Exhausted, Cancel,   Rejects(JobTerminal)),
                (Exhausted, Exhaust,  Rejects(JobTerminal)),
                (Exhausted, Retry,    Rejects(JobTerminal)),
            ]
        };

        #[test]
        fn table_covers_every_state_event_pair_exactly_once() {
            for state in ALL_STATES {
                for event in ALL_EVENTS {
                    let hits = TABLE
                        .iter()
                        .filter(|(s, e, _)| *s == state && *e == event)
                        .count();
                    assert_eq!(hits, 1, "{state:?} + {event:?} must appear exactly once");
                }
            }
            assert_eq!(TABLE.len(), 36);
        }

        #[test]
        fn every_transition_matches_the_table() {
            for (from, event, expect) in TABLE {
                let mut job = job_in(*from);
                let result = apply(&mut job, *event);
                match expect {
                    Expect::Lands(target) => {
                        assert_eq!(result, Ok(()), "{from:?} + {event:?} should succeed");
                        assert_eq!(job.status(), *target, "{from:?} + {event:?} target");
                    }
                    Expect::Rejects(err) => {
                        assert_eq!(result, Err(err.clone()), "{from:?} + {event:?} error");
                        assert_eq!(
                            job.status(),
                            *from,
                            "{from:?} + {event:?} must not mutate status on rejection"
                        );
                    }
                }
            }
        }

        #[test]
        fn retry_increments_attempt_and_requeues() {
            let mut job = job_in(JobStatus::Failed);
            assert_eq!(job.attempt().value(), 0);
            assert_eq!(job.retry(), Ok(()));
            assert_eq!(job.status(), JobStatus::Queued);
            assert_eq!(job.attempt().value(), 1);
        }

        #[test]
        fn retry_at_max_attempt_is_atomic() {
            let mut job = job_in(JobStatus::Failed);
            job.attempt = AttemptNumber(u32::MAX);
            assert_eq!(job.retry(), Err(DomainError::AttemptOverflow));
            // Atomicity: the failed bump leaves both fields untouched.
            assert_eq!(job.status(), JobStatus::Failed);
            assert_eq!(job.attempt().value(), u32::MAX);
        }
    }

    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        fn terminal_status() -> impl Strategy<Value = JobStatus> {
            prop_oneof![
                Just(JobStatus::Completed),
                Just(JobStatus::Cancelled),
                Just(JobStatus::Exhausted)
            ]
        }
        type Transition = fn(&mut Job) -> Result<(), DomainError>;

        const TRANSITIONS: [Transition; 6] = [
            Job::start,
            Job::complete,
            Job::fail,
            Job::cancel,
            Job::exhaust,
            Job::retry,
        ];

        proptest! {
            #[test]
            fn terminal_states_are_sinks(status in terminal_status()) {
                    for transition in TRANSITIONS {
                    let mut job = job_in(status);
                    prop_assert_eq!(transition(&mut job), Err(DomainError::JobTerminal));
                    prop_assert_eq!(job.status(), status);
                    }
            }

            #[test]
            fn retry_increments_or_leaves_untouched(attempt in any::<u32>()) {
                let mut job = job_in(JobStatus::Failed);
                job.attempt = AttemptNumber(attempt);
                let pre = job.clone();

                match job.retry() {
                    Ok(()) => {
                        prop_assert_eq!(job.attempt().value(), attempt + 1);
                        prop_assert_eq!(job.status(), JobStatus::Queued);
                    }
                    Err(err) => {
                        prop_assert_eq!(err, DomainError::AttemptOverflow);
                        // Atomicity: a failed bump leaves the whole job untouched.
                        prop_assert_eq!(job, pre);
                    }
                }
            }
        }
    }

    #[cfg(kani)]
    mod proofs {
        use super::*;

        #[kani::proof]
        fn retry_is_atomic_for_all_attempts() {
            let attempt: u32 = kani::any(); // symbolic: ALL 2^32 values at once
            let mut job = job_in(JobStatus::Failed);
            job.attempt = AttemptNumber(attempt);
            let before = job.clone();

            match job.retry() {
                Ok(()) => {
                    assert_eq!(job.attempt().value(), attempt + 1);
                    assert_eq!(job.status(), JobStatus::Queued);
                }
                Err(e) => {
                    assert_eq!(e, DomainError::AttemptOverflow);
                    assert_eq!(job, before); // atomicity, proven for every value
                }
            }
        }
    }
}
