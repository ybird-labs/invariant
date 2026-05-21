use invariant_types::{DomainError, JoinSetId, PromiseId, SignalDeliveryId};

/// Describes a specific journal invariant violation.
///
/// Variants are grouped as Structural (S-1..S-6), Side Effects (SE-1..SE-4),
/// Control Flow (CF-1..CF-4), and JoinSet (JS-1..JS-7).
///
/// `AllocatedChildMismatch` is a recovery-time integrity check
/// that ensures recovered allocated child IDs match deterministic derivation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalViolation {
    /// S-1: Sequence numbers must equal their array index (0-indexed, strict equality).
    NonMonotonicSequence {
        entry_index: usize,
        expected: u64,
        actual: u64,
    },
    /// S-2: The first event in every journal must be `ExecutionStarted`.
    MissingExecutionStarted { first_event: String },
    /// S-3: At most one terminal event (`Completed`, `Failed`, `Cancelled`) per journal.
    MultipleTerminalEvents { first_at: u64, second_at: u64 },
    /// S-4: A terminal event must be the last entry in the journal.
    TerminalNotLast {
        terminal_seq: u64,
        journal_len: usize,
    },
    /// S-5: `ExecutionCancelled` requires a preceding `CancelRequested`.
    CancelledWithoutRequest { cancelled_seq: u64 },
    /// S-6: Recovery check — allocated child promise ID must match deterministic derivation
    /// from execution root and allocation sequence.
    AllocatedChildMismatch {
        event_seq: u64,
        event_name: String,
        expected: PromiseId,
        actual: PromiseId,
    },

    /// SE-1: `InvokeStarted` requires a preceding `InvokeScheduled` for the same promise.
    StartedWithoutScheduled {
        promise_id: PromiseId,
        started_seq: u64,
    },
    /// SE-2: `InvokeCompleted` requires a preceding `InvokeStarted` for the same promise.
    CompletedWithoutStarted {
        promise_id: PromiseId,
        completed_seq: u64,
    },
    /// SE-3: `InvokeRetrying` requires a preceding `InvokeStarted` with matching promise and attempt.
    RetryingWithoutStarted {
        promise_id: PromiseId,
        failed_attempt: u32,
        retrying_seq: u64,
    },
    /// SE-4: No `InvokeStarted`, `InvokeRetrying`, or second `InvokeCompleted`
    /// after `InvokeCompleted` for the same promise.
    EventAfterCompleted {
        promise_id: PromiseId,
        offending_seq: u64,
        offending_event: String,
    },

    /// CF-1: `TimerFired` requires a preceding `TimerScheduled` for the same promise.
    TimerFiredWithoutScheduled {
        promise_id: PromiseId,
        fired_seq: u64,
    },
    /// CF-2: `SignalReceived` requires a preceding `SignalDelivered` with matching name, delivery ID, and payload.
    SignalReceivedWithoutDelivery {
        signal_name: String,
        delivery_id: SignalDeliveryId,
        received_seq: u64,
    },
    /// CF-3: Each `(signal_name, delivery_id)` pair may be consumed by at most one `SignalReceived`.
    SignalConsumedTwice {
        signal_name: String,
        delivery_id: SignalDeliveryId,
        second_seq: u64,
    },
    /// CF-4: `ExecutionAwaiting` with `Signal` kind must have exactly one promise in
    /// `waiting_on`, and it must match `AwaitKind::Signal.promise_id`.
    AwaitSignalInconsistent {
        awaiting_seq: u64,
        waiting_on_count: usize,
    },
    /// Model-shape alignment: `ExecutionAwaiting.waiting_on` is set-like.
    /// Duplicate promise IDs are invalid.
    AwaitWaitingOnDuplicate {
        awaiting_seq: u64,
        promise_id: PromiseId,
    },

    /// JS-1: `JoinSetSubmitted` requires a preceding `JoinSetCreated` for the same set.
    SubmitWithoutCreate {
        join_set_id: JoinSetId,
        submitted_seq: u64,
    },
    /// JS-2: No `JoinSetSubmitted` after any `JoinSetAwaited` for the same set.
    SubmitAfterAwait {
        join_set_id: JoinSetId,
        submitted_seq: u64,
    },
    /// JS-3: `JoinSetAwaited` for a promise requires that promise was previously `JoinSetSubmitted` to the same set.
    AwaitedNotMember {
        join_set_id: JoinSetId,
        promise_id: PromiseId,
        awaited_seq: u64,
    },
    /// JS-4: `JoinSetAwaited` for a promise requires that promise has a prior `InvokeCompleted`.
    AwaitedNotCompleted {
        promise_id: PromiseId,
        awaited_seq: u64,
    },
    /// JS-5: No two `JoinSetAwaited` for the same `(join_set_id, promise_id)` pair.
    DoubleConsume {
        join_set_id: JoinSetId,
        promise_id: PromiseId,
        second_seq: u64,
    },
    /// JS-6: Per set, the number of `JoinSetAwaited` events must not exceed `JoinSetSubmitted` events.
    ConsumeExceedsSubmit {
        join_set_id: JoinSetId,
        submitted: u32,
        awaited: u32,
    },
    /// JS-7: A promise may be submitted to at most one join set.
    PromiseInMultipleJoinSets {
        promise_id: PromiseId,
        first_js: JoinSetId,
        second_js: JoinSetId,
    },
}

/// Errors produced by journal operations.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("journal is empty")]
    EmptyJournal,
    #[error("invariant violation: {0}")]
    InvariantViolation(Box<JournalViolation>),
    #[error("domain error: {0}")]
    DomainError(DomainError),
}

impl std::fmt::Display for JournalViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonMonotonicSequence {
                entry_index,
                expected,
                actual,
            } => write!(
                f,
                "S-1: non-monotonic sequence at index {entry_index}: expected {expected}, got {actual}"
            ),
            Self::MissingExecutionStarted { first_event } => write!(
                f,
                "S-2: first event must be ExecutionStarted, got {first_event}"
            ),
            Self::MultipleTerminalEvents {
                first_at,
                second_at,
            } => write!(
                f,
                "S-3: multiple terminal events at seq {first_at} and {second_at}"
            ),
            Self::TerminalNotLast {
                terminal_seq,
                journal_len,
            } => write!(
                f,
                "S-4: terminal event at seq {terminal_seq} is not last (journal len {journal_len})"
            ),
            Self::CancelledWithoutRequest { cancelled_seq } => write!(
                f,
                "S-5: ExecutionCancelled at seq {cancelled_seq} without prior CancelRequested"
            ),
            Self::AllocatedChildMismatch {
                event_seq,
                event_name,
                expected,
                actual,
            } => write!(
                f,
                "S-6: child allocation mismatch at seq {event_seq} ({event_name}): expected {expected}, got {actual}"
            ),
            Self::StartedWithoutScheduled {
                promise_id,
                started_seq,
            } => write!(
                f,
                "SE-1: InvokeStarted at seq {started_seq} for {promise_id} without prior InvokeScheduled"
            ),
            Self::CompletedWithoutStarted {
                promise_id,
                completed_seq,
            } => write!(
                f,
                "SE-2: InvokeCompleted at seq {completed_seq} for {promise_id} without prior InvokeStarted"
            ),
            Self::RetryingWithoutStarted {
                promise_id,
                failed_attempt,
                retrying_seq,
            } => write!(
                f,
                "SE-3: InvokeRetrying at seq {retrying_seq} for {promise_id} failed_attempt {failed_attempt} without prior matching InvokeStarted"
            ),
            Self::EventAfterCompleted {
                promise_id,
                offending_seq,
                offending_event,
            } => write!(
                f,
                "SE-4: {offending_event} at seq {offending_seq} for {promise_id} after InvokeCompleted"
            ),
            Self::TimerFiredWithoutScheduled {
                promise_id,
                fired_seq,
            } => write!(
                f,
                "CF-1: TimerFired at seq {fired_seq} for {promise_id} without prior TimerScheduled"
            ),
            Self::SignalReceivedWithoutDelivery {
                signal_name,
                delivery_id,
                received_seq,
            } => write!(
                f,
                "CF-2: SignalReceived at seq {received_seq} for signal '{signal_name}' delivery {delivery_id} without prior SignalDelivered"
            ),
            Self::SignalConsumedTwice {
                signal_name,
                delivery_id,
                second_seq,
            } => write!(
                f,
                "CF-3: signal '{signal_name}' delivery {delivery_id} consumed twice, second at seq {second_seq}"
            ),
            Self::AwaitSignalInconsistent {
                awaiting_seq,
                waiting_on_count,
            } => write!(
                f,
                "CF-4: ExecutionAwaiting(Signal) at seq {awaiting_seq} is inconsistent (waiting_on_count={waiting_on_count}); expected exactly one waiting promise matching AwaitKind::Signal.promise_id"
            ),
            Self::AwaitWaitingOnDuplicate {
                awaiting_seq,
                promise_id,
            } => write!(
                f,
                "ExecutionAwaiting at seq {awaiting_seq} contains duplicate waiting_on promise {promise_id}"
            ),
            Self::SubmitWithoutCreate {
                join_set_id,
                submitted_seq,
            } => write!(
                f,
                "JS-1: JoinSetSubmitted at seq {submitted_seq} for {join_set_id} without prior JoinSetCreated"
            ),
            Self::SubmitAfterAwait {
                join_set_id,
                submitted_seq,
            } => write!(
                f,
                "JS-2: JoinSetSubmitted at seq {submitted_seq} for {join_set_id} after JoinSetAwaited"
            ),
            Self::AwaitedNotMember {
                join_set_id,
                promise_id,
                awaited_seq,
            } => write!(
                f,
                "JS-3: JoinSetAwaited at seq {awaited_seq} for {promise_id} not a member of {join_set_id}"
            ),
            Self::AwaitedNotCompleted {
                promise_id,
                awaited_seq,
            } => write!(
                f,
                "JS-4: JoinSetAwaited at seq {awaited_seq} for {promise_id} which is not yet completed"
            ),
            Self::DoubleConsume {
                join_set_id,
                promise_id,
                second_seq,
            } => write!(
                f,
                "JS-5: {promise_id} consumed twice from {join_set_id}, second at seq {second_seq}"
            ),
            Self::ConsumeExceedsSubmit {
                join_set_id,
                submitted,
                awaited,
            } => write!(
                f,
                "JS-6: {join_set_id} has {awaited} awaits exceeding {submitted} submits"
            ),
            Self::PromiseInMultipleJoinSets {
                promise_id,
                first_js,
                second_js,
            } => write!(
                f,
                "JS-7: {promise_id} submitted to both {first_js} and {second_js}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }

    fn js(tag: u8) -> JoinSetId {
        JoinSetId(pid(tag))
    }

    #[test]
    fn journal_violation_display_should_render_stable_messages_for_all_variants() {
        let p1 = pid(1);
        let p2 = pid(2);
        let js1 = js(3);
        let js2 = js(4);
        let cases = vec![
            (
                JournalViolation::NonMonotonicSequence {
                    entry_index: 1,
                    expected: 2,
                    actual: 3,
                },
                "S-1: non-monotonic sequence at index 1: expected 2, got 3".to_string(),
            ),
            (
                JournalViolation::MissingExecutionStarted {
                    first_event: "TimerFired".into(),
                },
                "S-2: first event must be ExecutionStarted, got TimerFired".to_string(),
            ),
            (
                JournalViolation::MultipleTerminalEvents {
                    first_at: 5,
                    second_at: 8,
                },
                "S-3: multiple terminal events at seq 5 and 8".to_string(),
            ),
            (
                JournalViolation::TerminalNotLast {
                    terminal_seq: 9,
                    journal_len: 11,
                },
                "S-4: terminal event at seq 9 is not last (journal len 11)".to_string(),
            ),
            (
                JournalViolation::CancelledWithoutRequest { cancelled_seq: 12 },
                "S-5: ExecutionCancelled at seq 12 without prior CancelRequested".to_string(),
            ),
            (
                JournalViolation::AllocatedChildMismatch {
                    event_seq: 13,
                    event_name: "InvokeScheduled".into(),
                    expected: p1.clone(),
                    actual: p2.clone(),
                },
                format!(
                    "S-6: child allocation mismatch at seq 13 (InvokeScheduled): expected {p1}, got {p2}"
                ),
            ),
            (
                JournalViolation::StartedWithoutScheduled {
                    promise_id: p1.clone(),
                    started_seq: 14,
                },
                format!("SE-1: InvokeStarted at seq 14 for {p1} without prior InvokeScheduled"),
            ),
            (
                JournalViolation::CompletedWithoutStarted {
                    promise_id: p1.clone(),
                    completed_seq: 15,
                },
                format!("SE-2: InvokeCompleted at seq 15 for {p1} without prior InvokeStarted"),
            ),
            (
                JournalViolation::RetryingWithoutStarted {
                    promise_id: p1.clone(),
                    failed_attempt: 2,
                    retrying_seq: 16,
                },
                format!(
                    "SE-3: InvokeRetrying at seq 16 for {p1} failed_attempt 2 without prior matching InvokeStarted"
                ),
            ),
            (
                JournalViolation::EventAfterCompleted {
                    promise_id: p1.clone(),
                    offending_seq: 17,
                    offending_event: "InvokeStarted".into(),
                },
                format!("SE-4: InvokeStarted at seq 17 for {p1} after InvokeCompleted"),
            ),
            (
                JournalViolation::TimerFiredWithoutScheduled {
                    promise_id: p1.clone(),
                    fired_seq: 18,
                },
                format!("CF-1: TimerFired at seq 18 for {p1} without prior TimerScheduled"),
            ),
            (
                JournalViolation::SignalReceivedWithoutDelivery {
                    signal_name: "ready".into(),
                    delivery_id: 19,
                    received_seq: 20,
                },
                "CF-2: SignalReceived at seq 20 for signal 'ready' delivery 19 without prior SignalDelivered".to_string(),
            ),
            (
                JournalViolation::SignalConsumedTwice {
                    signal_name: "ready".into(),
                    delivery_id: 21,
                    second_seq: 22,
                },
                "CF-3: signal 'ready' delivery 21 consumed twice, second at seq 22".to_string(),
            ),
            (
                JournalViolation::AwaitSignalInconsistent {
                    awaiting_seq: 23,
                    waiting_on_count: 2,
                },
                "CF-4: ExecutionAwaiting(Signal) at seq 23 is inconsistent (waiting_on_count=2); expected exactly one waiting promise matching AwaitKind::Signal.promise_id".to_string(),
            ),
            (
                JournalViolation::AwaitWaitingOnDuplicate {
                    awaiting_seq: 24,
                    promise_id: p1.clone(),
                },
                format!("ExecutionAwaiting at seq 24 contains duplicate waiting_on promise {p1}"),
            ),
            (
                JournalViolation::SubmitWithoutCreate {
                    join_set_id: js1.clone(),
                    submitted_seq: 25,
                },
                format!("JS-1: JoinSetSubmitted at seq 25 for {js1} without prior JoinSetCreated"),
            ),
            (
                JournalViolation::SubmitAfterAwait {
                    join_set_id: js1.clone(),
                    submitted_seq: 26,
                },
                format!("JS-2: JoinSetSubmitted at seq 26 for {js1} after JoinSetAwaited"),
            ),
            (
                JournalViolation::AwaitedNotMember {
                    join_set_id: js1.clone(),
                    promise_id: p1.clone(),
                    awaited_seq: 27,
                },
                format!("JS-3: JoinSetAwaited at seq 27 for {p1} not a member of {js1}"),
            ),
            (
                JournalViolation::AwaitedNotCompleted {
                    promise_id: p1.clone(),
                    awaited_seq: 28,
                },
                format!("JS-4: JoinSetAwaited at seq 28 for {p1} which is not yet completed"),
            ),
            (
                JournalViolation::DoubleConsume {
                    join_set_id: js1.clone(),
                    promise_id: p1.clone(),
                    second_seq: 29,
                },
                format!("JS-5: {p1} consumed twice from {js1}, second at seq 29"),
            ),
            (
                JournalViolation::ConsumeExceedsSubmit {
                    join_set_id: js1.clone(),
                    submitted: 1,
                    awaited: 2,
                },
                format!("JS-6: {js1} has 2 awaits exceeding 1 submits"),
            ),
            (
                JournalViolation::PromiseInMultipleJoinSets {
                    promise_id: p1.clone(),
                    first_js: js1.clone(),
                    second_js: js2.clone(),
                },
                format!("JS-7: {p1} submitted to both {js1} and {js2}"),
            ),
        ];

        for (violation, expected) in cases {
            assert_eq!(violation.to_string(), expected);
        }
    }
}
