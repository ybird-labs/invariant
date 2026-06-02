use crate::domain::{Deadline, DomainError, Priority, ReadyAt};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(String);

impl JobId {
    pub fn new(id: impl Into<String>) -> Result<Self, DomainError> {
        let value = id.into();
        if value.is_empty() {
            return Err(DomainError::EmptyJobId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetRef(String);

impl TargetRef {
    pub fn new(target: impl Into<String>) -> Result<Self, DomainError> {
        let value = target.into();
        if value.is_empty() {
            return Err(DomainError::EmptyTargetRef);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptNumber(u32);

impl AttemptNumber {
    pub const fn zero() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Returns the next attempt number, or `AttemptOverflow` at `u32::MAX`.
    pub fn checked_next(self) -> Result<Self, DomainError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(DomainError::AttemptOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Failed,
    Completed,
    Cancelled,
    Exhausted,
}

impl JobStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Exhausted
        )
    }
}

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

    #[must_use]
    pub fn id(&self) -> &JobId {
        &self.id
    }

    #[must_use]
    pub fn target(&self) -> &TargetRef {
        &self.target
    }

    #[must_use]
    pub fn priority(&self) -> Priority {
        self.priority
    }

    #[must_use]
    pub fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    #[must_use]
    pub fn ready_at(&self) -> Option<ReadyAt> {
        self.ready_at
    }

    #[must_use]
    pub fn status(&self) -> JobStatus {
        self.status
    }

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

    pub fn start(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Queued => {
                self.status = JobStatus::Running;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    pub fn complete(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Running => {
                self.status = JobStatus::Completed;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    pub fn fail(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Running => {
                self.status = JobStatus::Failed;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    pub fn cancel(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Queued | JobStatus::Running | JobStatus::Failed => {
                self.status = JobStatus::Cancelled;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

    pub fn exhaust(&mut self) -> Result<(), DomainError> {
        match self.status {
            JobStatus::Failed => {
                self.status = JobStatus::Exhausted;
                Ok(())
            }
            _ => Err(self.reject()),
        }
    }

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
}
