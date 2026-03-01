use std::time::Duration;

use chrono::{DateTime, Utc};
use invariant_types::{
    AwaitKind, EventType, ExecutionError, InvokeKind, JoinSetId, JournalEntry, Payload, PromiseId,
    RetryPolicy, SignalDeliveryId,
};

/// Caller intent for journal mutation.
///
/// Commands that allocate a new PromiseId omit it — the aggregate
/// assigns it from `next_child_seq`. Commands that reference an
/// existing promise carry the PromiseId explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    // Lifecycle (4)
    Complete {
        result: Payload,
    },
    Fail {
        error: ExecutionError,
    },
    RequestCancel {
        reason: String,
    },
    Cancel {
        reason: String,
    },
    // Side Effects (4)
    ScheduleInvoke {
        kind: InvokeKind,
        function_name: String,
        input: Payload,
        retry_policy: Option<RetryPolicy>,
    },
    StartInvoke {
        promise_id: PromiseId,
        attempt: u32,
    },
    CompleteInvoke {
        promise_id: PromiseId,
        result: Payload,
        attempt: u32,
    },
    RetryInvoke {
        promise_id: PromiseId,
        failed_attempt: u32,
        error: ExecutionError,
        retry_at: DateTime<Utc>,
    },
    // Nondeterminism (2)
    CaptureRandom {
        value: Vec<u8>,
    },
    CaptureTime {
        time: DateTime<Utc>,
    },
    // Control Flow (6)
    ScheduleTimer {
        duration: Duration,
        fire_at: DateTime<Utc>,
    },
    FireTimer {
        promise_id: PromiseId,
    },
    DeliverSignal {
        signal_name: String,
        payload: Payload,
        delivery_id: SignalDeliveryId,
    },
    ConsumeSignal {
        signal_name: String,
        payload: Payload,
        delivery_id: SignalDeliveryId,
    },
    Await {
        waiting_on: Vec<PromiseId>,
        kind: AwaitKind,
    },
    Resume,
    // Concurrency (3)
    CreateJoinSet,
    SubmitToJoinSet {
        join_set_id: JoinSetId,
        promise_id: PromiseId,
    },
    ConsumeFromJoinSet {
        join_set_id: JoinSetId,
        promise_id: PromiseId,
        result: Payload,
    },
}
impl Command {
    pub fn is_allocating(&self) -> bool {
        matches!(
            self,
            Self::ScheduleInvoke { .. }
                | Self::CaptureRandom { .. }
                | Self::CaptureTime { .. }
                | Self::ScheduleTimer { .. }
                | Self::ConsumeSignal { .. }
                | Self::CreateJoinSet
        )
    }
}

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub entry: JournalEntry,
    pub allocated_id: Option<PromiseId>,
}

/// Convert a [`Command`] into its corresponding [`EventType`].
///
/// Pure function — no state access. Allocating commands pull their
/// `PromiseId` from `allocated_id`; non-allocating commands either
/// carry their own or have none.
///
/// # Panics
///
/// Panics if `allocated_id` is `None` for an allocating command.
/// This is a programming error — callers must pair [`Command::is_allocating`]
/// with ID generation before calling this function.
pub(crate) fn command_to_event(cmd: Command, allocated_id: Option<&PromiseId>) -> EventType {
    /// Clone the allocated ID or panic. Only called for the 6 allocating
    /// commands, where `handle()` guarantees `Some`.
    fn take_allocated(allocated_id: Option<&PromiseId>) -> PromiseId {
        allocated_id
            .expect("allocating command must have allocated_id")
            .clone()
    }

    match cmd {
        // ── Lifecycle ──
        Command::Complete { result } => EventType::ExecutionCompleted { result },
        Command::Fail { error } => EventType::ExecutionFailed { error },
        Command::RequestCancel { reason } => EventType::CancelRequested { reason },
        Command::Cancel { reason } => EventType::ExecutionCancelled { reason },

        // ── Side Effects ──
        Command::ScheduleInvoke {
            kind,
            function_name,
            input,
            retry_policy,
        } => EventType::InvokeScheduled {
            promise_id: take_allocated(allocated_id),
            kind,
            function_name,
            input,
            retry_policy,
        },
        Command::StartInvoke {
            promise_id,
            attempt,
        } => EventType::InvokeStarted {
            promise_id,
            attempt,
        },
        Command::CompleteInvoke {
            promise_id,
            result,
            attempt,
        } => EventType::InvokeCompleted {
            promise_id,
            result,
            attempt,
        },
        Command::RetryInvoke {
            promise_id,
            failed_attempt,
            error,
            retry_at,
        } => EventType::InvokeRetrying {
            promise_id,
            failed_attempt,
            error,
            retry_at,
        },

        // ── Nondeterminism ──
        Command::CaptureRandom { value } => EventType::RandomGenerated {
            promise_id: take_allocated(allocated_id),
            value,
        },
        Command::CaptureTime { time } => EventType::TimeRecorded {
            promise_id: take_allocated(allocated_id),
            time,
        },

        // ── Control Flow ──
        Command::ScheduleTimer { duration, fire_at } => EventType::TimerScheduled {
            promise_id: take_allocated(allocated_id),
            duration,
            fire_at,
        },
        Command::FireTimer { promise_id } => EventType::TimerFired { promise_id },
        Command::DeliverSignal {
            signal_name,
            payload,
            delivery_id,
        } => EventType::SignalDelivered {
            signal_name,
            payload,
            delivery_id,
        },
        Command::ConsumeSignal {
            signal_name,
            payload,
            delivery_id,
        } => EventType::SignalReceived {
            promise_id: take_allocated(allocated_id),
            signal_name,
            payload,
            delivery_id,
        },
        Command::Await { waiting_on, kind } => EventType::ExecutionAwaiting { waiting_on, kind },
        Command::Resume => EventType::ExecutionResumed,

        // ── Concurrency ──
        Command::CreateJoinSet => EventType::JoinSetCreated {
            join_set_id: JoinSetId(take_allocated(allocated_id)),
        },
        Command::SubmitToJoinSet {
            join_set_id,
            promise_id,
        } => EventType::JoinSetSubmitted {
            join_set_id,
            promise_id,
        },
        Command::ConsumeFromJoinSet {
            join_set_id,
            promise_id,
            result,
        } => EventType::JoinSetAwaited {
            join_set_id,
            promise_id,
            result,
        },
    }
}
