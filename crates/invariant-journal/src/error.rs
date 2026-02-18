use invariant_types::{JoinSetId, PromiseId, SignalDeliveryId};

/// Describes a specific journal invariant violation.
///
/// Each variant maps 1:1 to a formal invariant from the Quint spec.
/// Grouped: Structural (S-1..S-5), Side Effects (SE-1..SE-4),
/// Control Flow (CF-1..CF-4), JoinSet (JS-1..JS-7).
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
    /// SE-3: `InvokeRetrying` requires a preceding `InvokeStarted` for the same promise.
    RetryingWithoutStarted {
        promise_id: PromiseId,
        retrying_seq: u64,
    },
    /// SE-4: No `InvokeStarted` or `InvokeRetrying` after `InvokeCompleted` for the same promise.
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
    /// CF-4: `ExecutionAwaiting` with `Signal` kind must have exactly one promise in `waiting_on`.
    AwaitSignalInconsistent {
        awaiting_seq: u64,
        waiting_on_count: usize,
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
    InvariantViolation(JournalViolation),
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
                retrying_seq,
            } => write!(
                f,
                "SE-3: InvokeRetrying at seq {retrying_seq} for {promise_id} without prior InvokeStarted"
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
                "CF-4: ExecutionAwaiting(Signal) at seq {awaiting_seq} has {waiting_on_count} promises, expected 1"
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
