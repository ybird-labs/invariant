use std::time::Duration;

use chrono::{DateTime, Utc};
use invariant_types::{
    AwaitKind, EventType, ExecutionError, InvokeKind, JoinSetId, JournalEntry, Payload, PromiseId,
    RetryPolicy, SignalDeliveryId,
};

/// Caller intent for journal mutation.
///
/// Use [`classify()`](Self::classify) to decompose into allocating vs
/// non-allocating form for type-safe event conversion.
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
    /// Decompose into allocating or non-allocating form for type-safe
    /// event conversion.
    pub(crate) fn classify(self) -> CommandKind {
        match self {
            // ── Allocating (6) ──
            Command::ScheduleInvoke {
                kind,
                function_name,
                input,
                retry_policy,
            } => CommandKind::Allocating(AllocatingCommand::ScheduleInvoke {
                kind,
                function_name,
                input,
                retry_policy,
            }),
            Command::CaptureRandom { value } => {
                CommandKind::Allocating(AllocatingCommand::CaptureRandom { value })
            }
            Command::CaptureTime { time } => {
                CommandKind::Allocating(AllocatingCommand::CaptureTime { time })
            }
            Command::ScheduleTimer { duration, fire_at } => {
                CommandKind::Allocating(AllocatingCommand::ScheduleTimer { duration, fire_at })
            }
            Command::ConsumeSignal {
                signal_name,
                payload,
                delivery_id,
            } => CommandKind::Allocating(AllocatingCommand::ConsumeSignal {
                signal_name,
                payload,
                delivery_id,
            }),
            Command::CreateJoinSet => CommandKind::Allocating(AllocatingCommand::CreateJoinSet),
            // ── Non-allocating (13) ──
            Command::Complete { result } => {
                CommandKind::NonAllocating(NonAllocatingCommand::Complete { result })
            }
            Command::Fail { error } => {
                CommandKind::NonAllocating(NonAllocatingCommand::Fail { error })
            }
            Command::RequestCancel { reason } => {
                CommandKind::NonAllocating(NonAllocatingCommand::RequestCancel { reason })
            }
            Command::Cancel { reason } => {
                CommandKind::NonAllocating(NonAllocatingCommand::Cancel { reason })
            }
            Command::StartInvoke {
                promise_id,
                attempt,
            } => CommandKind::NonAllocating(NonAllocatingCommand::StartInvoke {
                promise_id,
                attempt,
            }),
            Command::CompleteInvoke {
                promise_id,
                result,
                attempt,
            } => CommandKind::NonAllocating(NonAllocatingCommand::CompleteInvoke {
                promise_id,
                result,
                attempt,
            }),
            Command::RetryInvoke {
                promise_id,
                failed_attempt,
                error,
                retry_at,
            } => CommandKind::NonAllocating(NonAllocatingCommand::RetryInvoke {
                promise_id,
                failed_attempt,
                error,
                retry_at,
            }),
            Command::FireTimer { promise_id } => {
                CommandKind::NonAllocating(NonAllocatingCommand::FireTimer { promise_id })
            }
            Command::DeliverSignal {
                signal_name,
                payload,
                delivery_id,
            } => CommandKind::NonAllocating(NonAllocatingCommand::DeliverSignal {
                signal_name,
                payload,
                delivery_id,
            }),
            Command::Await { waiting_on, kind } => {
                CommandKind::NonAllocating(NonAllocatingCommand::Await { waiting_on, kind })
            }
            Command::Resume => CommandKind::NonAllocating(NonAllocatingCommand::Resume),
            Command::SubmitToJoinSet {
                join_set_id,
                promise_id,
            } => CommandKind::NonAllocating(NonAllocatingCommand::SubmitToJoinSet {
                join_set_id,
                promise_id,
            }),
            Command::ConsumeFromJoinSet {
                join_set_id,
                promise_id,
                result,
            } => CommandKind::NonAllocating(NonAllocatingCommand::ConsumeFromJoinSet {
                join_set_id,
                promise_id,
                result,
            }),
        }
    }
}

/// Result of [`classify()`](Command::classify): either an allocating or
/// non-allocating command, ready for type-safe event conversion.
pub(crate) enum CommandKind {
    Allocating(AllocatingCommand),
    NonAllocating(NonAllocatingCommand),
}

/// Commands that require a new [`PromiseId`] assigned by the aggregate.
pub(crate) enum AllocatingCommand {
    ScheduleInvoke {
        kind: InvokeKind,
        function_name: String,
        input: Payload,
        retry_policy: Option<RetryPolicy>,
    },
    CaptureRandom {
        value: Vec<u8>,
    },
    CaptureTime {
        time: DateTime<Utc>,
    },
    ScheduleTimer {
        duration: Duration,
        fire_at: DateTime<Utc>,
    },
    ConsumeSignal {
        signal_name: String,
        payload: Payload,
        delivery_id: SignalDeliveryId,
    },
    CreateJoinSet,
}

/// Commands that carry their own [`PromiseId`] or need none.
pub(crate) enum NonAllocatingCommand {
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
    // Side Effects — referencing (3)
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
    // Control Flow — referencing (3)
    FireTimer {
        promise_id: PromiseId,
    },
    DeliverSignal {
        signal_name: String,
        payload: Payload,
        delivery_id: SignalDeliveryId,
    },
    Await {
        waiting_on: Vec<PromiseId>,
        kind: AwaitKind,
    },
    Resume,
    // Concurrency — referencing (2)
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

/// Output of [`ExecutionState::handle`]: the appended journal entry
/// and any allocated promise ID.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub entry: JournalEntry,
    pub allocated_id: Option<PromiseId>,
}

/// Convert an [`AllocatingCommand`] and its assigned [`PromiseId`]
/// into the corresponding [`EventType`].
pub(crate) fn allocating_to_event(cmd: AllocatingCommand, allocated_id: PromiseId) -> EventType {
    match cmd {
        AllocatingCommand::ScheduleInvoke {
            kind,
            function_name,
            input,
            retry_policy,
        } => EventType::InvokeScheduled {
            promise_id: allocated_id,
            kind,
            function_name,
            input,
            retry_policy,
        },
        AllocatingCommand::CaptureRandom { value } => EventType::RandomGenerated {
            promise_id: allocated_id,
            value,
        },
        AllocatingCommand::CaptureTime { time } => EventType::TimeRecorded {
            promise_id: allocated_id,
            time,
        },
        AllocatingCommand::ScheduleTimer { duration, fire_at } => EventType::TimerScheduled {
            promise_id: allocated_id,
            duration,
            fire_at,
        },
        AllocatingCommand::ConsumeSignal {
            signal_name,
            payload,
            delivery_id,
        } => EventType::SignalReceived {
            promise_id: allocated_id,
            signal_name,
            payload,
            delivery_id,
        },
        AllocatingCommand::CreateJoinSet => EventType::JoinSetCreated {
            join_set_id: JoinSetId(allocated_id),
        },
    }
}

/// Convert a [`NonAllocatingCommand`] into the corresponding [`EventType`].
pub(crate) fn non_allocating_to_event(cmd: NonAllocatingCommand) -> EventType {
    match cmd {
        // ── Lifecycle ──
        NonAllocatingCommand::Complete { result } => EventType::ExecutionCompleted { result },
        NonAllocatingCommand::Fail { error } => EventType::ExecutionFailed { error },
        NonAllocatingCommand::RequestCancel { reason } => EventType::CancelRequested { reason },
        NonAllocatingCommand::Cancel { reason } => EventType::ExecutionCancelled { reason },
        // ── Side Effects ──
        NonAllocatingCommand::StartInvoke {
            promise_id,
            attempt,
        } => EventType::InvokeStarted {
            promise_id,
            attempt,
        },
        NonAllocatingCommand::CompleteInvoke {
            promise_id,
            result,
            attempt,
        } => EventType::InvokeCompleted {
            promise_id,
            result,
            attempt,
        },
        NonAllocatingCommand::RetryInvoke {
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
        // ── Control Flow ──
        NonAllocatingCommand::FireTimer { promise_id } => EventType::TimerFired { promise_id },
        NonAllocatingCommand::DeliverSignal {
            signal_name,
            payload,
            delivery_id,
        } => EventType::SignalDelivered {
            signal_name,
            payload,
            delivery_id,
        },
        NonAllocatingCommand::Await { waiting_on, kind } => {
            EventType::ExecutionAwaiting { waiting_on, kind }
        }
        NonAllocatingCommand::Resume => EventType::ExecutionResumed,
        // ── Concurrency ──
        NonAllocatingCommand::SubmitToJoinSet {
            join_set_id,
            promise_id,
        } => EventType::JoinSetSubmitted {
            join_set_id,
            promise_id,
        },
        NonAllocatingCommand::ConsumeFromJoinSet {
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
