use crate::join_set::JoinSetId;
use crate::payload::Payload;
use crate::promise_id::PromiseId;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Categorizes the type of side-effect invocation.
///
/// Extensible: new side effect types (DB queries, gRPC calls) are added as
/// variants here, not as new event types. All share the same 3-phase
/// Scheduled → Started → Completed structure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvokeKind {
    /// Function/task/workflow invocation.
    Function,
    /// HTTP request to external service.
    Http,
}

/// Determines the wait satisfaction condition for `ExecutionAwaiting`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AwaitKind {
    /// Wait for a single promise.
    Single,
    /// Wait for any one of the promises (JoinSet js.next()).
    Any,
    /// Wait for all promises (JoinSet js.all()).
    All,
    /// Wait for a named signal.
    Signal { name: String },
}

// Retry policy for invocations.
// TODO: Still need to be defined
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {}

/// Monotonic per-signal-name delivery counter.
pub type SignalDeliveryId = u64;

/// All 20 journal event types, grouped by category.
///
/// Each category satisfies a distinct formal correctness property.
/// See JOURNAL_DESIGN.md for the full specification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    // ── Category 1: Lifecycle (Soundness) ──
    /// Always the first event. Pins execution to a specific component version.
    ExecutionStarted {
        component_digest: Vec<u8>,
        input: Payload,
        parent_id: Option<PromiseId>,
        idempotency_key: String,
    },
    /// Function returned Ok (terminal).
    ExecutionCompleted { result: Payload },
    /// Function returned Err or WASM trap (terminal).
    ExecutionFailed { error: String },
    /// External cancel signal arrived. Transitions to Cancelling.
    CancelRequested { reason: String },
    /// Cancellation finalized after cleanup (terminal). Requires preceding CancelRequested.
    ExecutionCancelled { reason: String },

    // ── Category 2: Side Effects (Replay Correctness) ──
    // 3-phase pattern: Scheduled → Started → Completed
    /// Intent to invoke. Enables exactly-once via replay matching.
    InvokeScheduled {
        promise_id: PromiseId,
        kind: InvokeKind,
        function_name: String,
        input: Payload,
        retry_policy: Option<RetryPolicy>,
    },
    /// Invocation is in-flight. Enables timeout detection.
    InvokeStarted { promise_id: PromiseId, attempt: u32 },
    /// Invocation result. Cached for replay.
    InvokeCompleted {
        promise_id: PromiseId,
        result: Payload,
        attempt: u32,
    },
    /// Transient failure, will retry.
    InvokeRetrying {
        promise_id: PromiseId,
        failed_attempt: u32,
        error: String,
        retry_at: DateTime<Utc>,
    },

    // ── Category 3: Nondeterminism (Determinism Guarantee) ──
    // Single-phase: pure value capture, no execution to track.
    /// `random()` called. Value captured for deterministic replay.
    RandomGenerated {
        promise_id: PromiseId,
        value: Vec<u8>,
    },
    /// `now()` called. Wall-clock time captured for deterministic replay.
    TimeRecorded {
        promise_id: PromiseId,
        time: DateTime<Utc>,
    },

    // ── Category 4: Control Flow (State Reconstruction) ──
    /// `sleep(duration)` called. Records both the requested duration and computed fire time.
    TimerScheduled {
        promise_id: PromiseId,
        duration: Duration,
        fire_at: DateTime<Utc>,
    },
    /// Timer duration elapsed. Resolves the timer's promise_id.
    TimerFired { promise_id: PromiseId },
    /// External signal arrived at execution. Durable buffer — no promise_id.
    SignalDelivered {
        signal_name: String,
        payload: Payload,
        delivery_id: SignalDeliveryId,
    },
    /// Workflow consumed signal via await_signal(). Carries promise_id for replay cache.
    SignalReceived {
        promise_id: PromiseId,
        signal_name: String,
        payload: Payload,
        delivery_id: SignalDeliveryId,
    },
    /// Workflow blocks on pending promises. Explicit suspend per IEEE 1849 (XES).
    ExecutionAwaiting {
        waiting_on: Vec<PromiseId>,
        kind: AwaitKind,
    },
    /// Blocked → Running. Wait condition satisfied.
    ExecutionResumed,

    // ── Category 5: Concurrency (Total Ordering) ──
    /// Opens a concurrent region. Allocates a child position in the call tree.
    JoinSetCreated { join_set_id: JoinSetId },
    /// Adds a scheduled promise to the set. No submits allowed after first await (JS-2).
    JoinSetSubmitted {
        join_set_id: JoinSetId,
        promise_id: PromiseId,
    },
    /// Records which result was consumed at this point. Replay marker, not state transition.
    JoinSetAwaited {
        join_set_id: JoinSetId,
        promise_id: PromiseId,
        result: Payload,
    },
}

impl EventType {
    /// Returns the variant name as a static string for error messages and logging.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ExecutionStarted { .. } => "ExecutionStarted",
            Self::ExecutionCompleted { .. } => "ExecutionCompleted",
            Self::ExecutionFailed { .. } => "ExecutionFailed",
            Self::CancelRequested { .. } => "CancelRequested",
            Self::ExecutionCancelled { .. } => "ExecutionCancelled",
            Self::InvokeScheduled { .. } => "InvokeScheduled",
            Self::InvokeStarted { .. } => "InvokeStarted",
            Self::InvokeCompleted { .. } => "InvokeCompleted",
            Self::InvokeRetrying { .. } => "InvokeRetrying",
            Self::RandomGenerated { .. } => "RandomGenerated",
            Self::TimeRecorded { .. } => "TimeRecorded",
            Self::TimerScheduled { .. } => "TimerScheduled",
            Self::TimerFired { .. } => "TimerFired",
            Self::SignalDelivered { .. } => "SignalDelivered",
            Self::SignalReceived { .. } => "SignalReceived",
            Self::ExecutionAwaiting { .. } => "ExecutionAwaiting",
            Self::ExecutionResumed => "ExecutionResumed",
            Self::JoinSetCreated { .. } => "JoinSetCreated",
            Self::JoinSetSubmitted { .. } => "JoinSetSubmitted",
            Self::JoinSetAwaited { .. } => "JoinSetAwaited",
        }
    }

    /// Whether this event ends the execution (Completed, Failed, or Cancelled).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::ExecutionCompleted { .. }
                | Self::ExecutionFailed { .. }
                | Self::ExecutionCancelled { .. }
        )
    }
}
