use std::collections::HashSet;

use chrono::{DateTime, Utc};
use invariant_types::{
    DomainError, EventType, ExecutionId, ExecutionStatus, JournalEntry, Payload, PromiseId,
};

use crate::{
    command::{Command, CommandKind, CommandResult, allocating_to_event, non_allocating_to_event},
    error::{JournalError, JournalViolation},
    invariants::InvariantState,
    replay::ReplayCache,
    status::{self, derive_next_status},
};

/// Per-execution aggregate root.
///
/// Owns the append-only journal, derived status, invariant checking state,
/// replay cache, and child-ID allocation counter.
///
/// # Construction
///
/// - [`new()`](Self::new) — fresh execution (appends `ExecutionStarted` at seq 0).
/// - [`recover()`](Self::recover) — rebuild from a persisted journal.
///
/// # Invariants
///
/// Every appended entry passes through [`InvariantState::check_append`],
/// enforcing all 21 formal invariants (S-1..S-5, SE-1..SE-4, CF-1..CF-4,
/// JS-1..JS-7).
#[derive(Clone, Debug)]
pub struct ExecutionState {
    execution_id: ExecutionId,
    journal: Vec<JournalEntry>,
    status: ExecutionStatus,
    next_child_seq: ChildSeqCounter,
    allocated_children: HashSet<PromiseId>,
    invariant_state: InvariantState,
    replay_cache: ReplayCache,
}

impl ExecutionState {
    /// Create a new execution, appending the initial ExecutionStarted event.
    ///
    /// This is the only way to create an ExecutionState from scratch.
    /// The first event is always ExecutionStarted, satisfying invariant S-2.
    pub fn new(
        component_digest: Vec<u8>,
        input: Payload,
        parent_id: Option<PromiseId>,
        idempotency_key: String,
        now: DateTime<Utc>,
    ) -> Result<Self, JournalError> {
        let execution_id =
            ExecutionId::derive(&component_digest, &idempotency_key, parent_id.as_ref());
        let entry = JournalEntry {
            sequence: 0,
            timestamp: now,
            event: EventType::ExecutionStarted {
                component_digest,
                input,
                parent_id,
                idempotency_key,
            },
        };
        let mut invariant_state = InvariantState::new();
        invariant_state
            .check_append(&entry)
            .map_err(JournalError::InvariantViolation)?;
        Ok(Self {
            execution_id,
            journal: vec![entry],
            status: ExecutionStatus::Running,
            next_child_seq: ChildSeqCounter::default(),
            allocated_children: HashSet::new(),
            invariant_state,
            replay_cache: ReplayCache::default(),
        })
    }

    /// Rebuild an [`ExecutionState`] from a persisted journal.
    ///
    /// Replays every entry through [`InvariantState::check_append`] to
    /// re-validate the full invariant set. Then derives status, rebuilds
    /// the replay cache, and reconstructs the child-allocation counter
    /// by scanning for allocating events.
    ///
    /// # Errors
    ///
    /// - [`JournalError::EmptyJournal`] — `journal` is empty.
    /// - [`JournalError::InvariantViolation`] — first entry is not
    ///   `ExecutionStarted` (S-2), any entry fails invariant checking, or
    ///   a recovered allocated child ID does not match deterministic derivation.
    /// - [`JournalError::DomainError`] — child-sequence arithmetic overflows
    ///   while rebuilding allocation state.
    pub fn recover(journal: Vec<JournalEntry>) -> Result<Self, JournalError> {
        let Some(first) = journal.first() else {
            return Err(JournalError::EmptyJournal);
        };
        let EventType::ExecutionStarted {
            ref component_digest,
            ref idempotency_key,
            ref parent_id,
            ..
        } = first.event
        else {
            return Err(JournalError::InvariantViolation(
                JournalViolation::MissingExecutionStarted {
                    first_event: first.event.name().into(),
                },
            ));
        };
        let execution_id =
            ExecutionId::derive(component_digest, idempotency_key, parent_id.as_ref());

        let mut invariant_state = InvariantState::new();
        for entry in &journal {
            invariant_state
                .check_append(entry)
                .map_err(JournalError::InvariantViolation)?;
        }

        let status = status::derive_status(&journal);
        let replay_cache = ReplayCache::build(&journal);
        let (child_seq, allocated_children) = build_child_state(&execution_id, &journal)?;

        Ok(Self {
            execution_id,
            journal,
            status,
            next_child_seq: child_seq,
            allocated_children,
            invariant_state,
            replay_cache,
        })
    }
    /// Process a command: validate, then commit all state changes atomically.
    ///
    /// No state mutation occurs until every validation step succeeds.
    /// On invariant failure, the aggregate is unchanged.
    ///
    /// # Errors
    ///
    /// - [`JournalError::DomainError`] — child counter overflow
    ///   (`MaxChildrenExceeded`) or invalid execution depth.
    /// - [`JournalError::InvariantViolation`] — any of the 21 formal
    ///   invariants rejected the resulting entry.
    pub fn handle(
        &mut self,
        cmd: Command,
        now: DateTime<Utc>,
    ) -> Result<CommandResult, JournalError> {
        // 1. Classify the command, then derive child ID + build event.
        //    No state mutation until all validation succeeds.
        let (event, allocated_id, permit) = match cmd.classify() {
            CommandKind::Allocating(alloc_cmd) => {
                let permit = self
                    .next_child_seq
                    .check_advance()
                    .map_err(JournalError::DomainError)?;
                let child_id = self
                    .execution_id
                    .child(self.next_child_seq.current())
                    .map_err(JournalError::DomainError)?;
                let event = allocating_to_event(alloc_cmd, child_id.clone());
                (event, Some(child_id), Some(permit))
            }
            CommandKind::NonAllocating(ref_cmd) => {
                let event = non_allocating_to_event(ref_cmd);
                (event, None, None)
            }
        };

        // 2. Build journal entry.
        let entry = JournalEntry {
            sequence: self.journal.len() as u64,
            timestamp: now,
            event,
        };

        // 3. Validate invariants — check_append calls apply_entry internally
        //    on success. On failure, InvariantState remains unchanged.
        self.invariant_state
            .check_append(&entry)
            .map_err(JournalError::InvariantViolation)?;

        // 4. Commit — entirely infallible from here.
        if let (Some(pid), Some(permit)) = (&allocated_id, permit) {
            self.next_child_seq.advance(permit);
            self.allocated_children.insert(pid.clone());
        }
        self.status = derive_next_status(self.status.clone(), &entry.event);
        self.replay_cache.insert_event(&entry);
        self.journal.push(entry.clone());

        Ok(CommandResult {
            entry,
            allocated_id,
        })
    }

    // ── Accessors ──

    /// The root promise ID for this execution.
    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    /// The append-only event log.
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Derived execution status (Running, Blocked, terminal, etc.).
    pub fn status(&self) -> &ExecutionStatus {
        &self.status
    }

    /// Optimistic concurrency journal_version — equal to the number of journal entries.
    pub fn journal_version(&self) -> u64 {
        self.journal.len() as u64
    }

    /// Set of all promise IDs allocated by this execution so far.
    pub fn allocated_children(&self) -> &HashSet<PromiseId> {
        &self.allocated_children
    }

    /// Replay cache for deterministic re-execution lookups.
    pub fn replay_cache(&self) -> &ReplayCache {
        &self.replay_cache
    }

    /// Whether the execution has reached a terminal state (Completed, Failed, or Cancelled).
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Next child sequence number for allocating child promise IDs.
    pub fn next_child_seq(&self) -> u32 {
        self.next_child_seq.current()
    }
}

/// Reconstruct the child-allocation counter and set from journal entries.
///
/// Scans for the 6 allocating event kinds, verifies each recovered
/// promise ID matches deterministic derivation from the execution root,
/// and rebuilds the counter and set.
fn build_child_state(
    execution_id: &ExecutionId,
    entries: &[JournalEntry],
) -> Result<(ChildSeqCounter, HashSet<PromiseId>), JournalError> {
    let mut next_child_seq = ChildSeqCounter::from_count(0);
    let mut allocated_children = HashSet::new();

    for entry in entries {
        let allocated_id = match &entry.event {
            EventType::InvokeScheduled { promise_id, .. }
            | EventType::RandomGenerated { promise_id, .. }
            | EventType::TimeRecorded { promise_id, .. }
            | EventType::TimerScheduled { promise_id, .. }
            | EventType::SignalReceived { promise_id, .. } => Some(promise_id.clone()),
            EventType::JoinSetCreated { join_set_id } => Some(join_set_id.0.clone()),
            _ => None,
        };

        if let Some(actual) = allocated_id {
            let expected = execution_id
                .child(next_child_seq.current())
                .map_err(JournalError::DomainError)?;

            if actual != expected {
                return Err(JournalError::InvariantViolation(
                    JournalViolation::AllocatedChildMismatch {
                        event_seq: entry.sequence,
                        event_name: entry.event.name().to_string(),
                        expected,
                        actual,
                    },
                ));
            }

            allocated_children.insert(actual);
            let permit = next_child_seq
                .check_advance()
                .map_err(JournalError::DomainError)?;
            next_child_seq.advance(permit);
        }
    }

    Ok((next_child_seq, allocated_children))
}

/// Proof token from [`ChildSeqCounter::check_advance`].
///
/// Consumed by [`ChildSeqCounter::advance`] to make the increment infallible.
struct AdvancePermit {
    _private: (),
}

/// Monotonically increasing counter for child sequence allocation.
///
/// Overflow is checked before mutation: [`check_advance`](Self::check_advance)
/// returns an [`AdvancePermit`] that [`advance`](Self::advance) consumes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ChildSeqCounter(u32);

impl ChildSeqCounter {
    fn from_count(count: u32) -> Self {
        Self(count)
    }

    /// Peek at the current value without advancing.
    fn current(self) -> u32 {
        self.0
    }

    /// Check that the counter can advance without overflow.
    ///
    /// Returns an [`AdvancePermit`] required by [`advance`](Self::advance).
    fn check_advance(&self) -> Result<AdvancePermit, DomainError> {
        self.0
            .checked_add(1)
            .map(|_| AdvancePermit { _private: () })
            .ok_or(DomainError::MaxChildrenExceeded { max: u32::MAX })
    }

    /// Increment the counter. Requires an [`AdvancePermit`] from [`check_advance`].
    fn advance(&mut self, _permit: AdvancePermit) {
        self.0 += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use invariant_types::{
        AwaitKind, Codec, ErrorKind, ExecutionError, InvokeKind, JoinSetId, Payload,
    };
    use std::time::Duration;

    const DIGEST: &[u8] = &[1, 2, 3];
    const KEY: &str = "test-key";

    fn payload() -> Payload {
        Payload::new(vec![], Codec::Json)
    }

    /// Build a fresh ExecutionState with valid execution_id derivation.
    /// Starts with 1 journal entry (ExecutionStarted at seq 0), status Running.
    fn new_state() -> ExecutionState {
        ExecutionState::new(
            DIGEST.to_vec(),
            payload(),
            None,
            KEY.to_string(),
            Utc::now(),
        )
        .expect("new() with valid inputs must succeed")
    }

    // ── Task 7: Lifecycle commands ──

    #[test]
    fn handle_complete_maps_event_and_transitions_status() {
        let mut state = new_state();
        let now = Utc::now();

        let result = state
            .handle(Command::Complete { result: payload() }, now)
            .expect("Complete on Running must succeed");

        assert!(matches!(
            result.entry.event,
            EventType::ExecutionCompleted { .. }
        ));
        assert!(result.allocated_id.is_none());
        assert_eq!(result.entry.sequence, 1);
        assert_eq!(*state.status(), ExecutionStatus::Completed);
        assert!(state.is_terminal());
        assert_eq!(state.journal().len(), 2);
        assert_eq!(state.next_child_seq(), 0);
    }

    #[test]
    fn handle_fail_maps_event_and_propagates_error() {
        let mut state = new_state();
        let now = Utc::now();

        let error = ExecutionError::new(ErrorKind::Uncategorized, "boom");
        let result = state
            .handle(
                Command::Fail {
                    error: error.clone(),
                },
                now,
            )
            .expect("Fail on Running must succeed");

        assert!(matches!(
            result.entry.event,
            EventType::ExecutionFailed { .. }
        ));
        assert!(result.allocated_id.is_none());
        assert_eq!(*state.status(), ExecutionStatus::Failed);
        assert!(state.is_terminal());
        assert_eq!(state.journal().len(), 2);
    }

    #[test]
    fn handle_cancel_flow_transitions_through_cancelling_to_cancelled() {
        let mut state = new_state();
        let now = Utc::now();

        // Step 1: RequestCancel → Cancelling (non-terminal)
        let req = state
            .handle(
                Command::RequestCancel {
                    reason: "stop".into(),
                },
                now,
            )
            .expect("RequestCancel on Running must succeed");

        assert!(matches!(req.entry.event, EventType::CancelRequested { .. }));
        assert_eq!(*state.status(), ExecutionStatus::Cancelling);
        assert!(!state.is_terminal());

        // Step 2: Cancel → Cancelled (terminal), requires prior RequestCancel (S-5)
        let cancel = state
            .handle(
                Command::Cancel {
                    reason: "stopped".into(),
                },
                now,
            )
            .expect("Cancel after RequestCancel must succeed");

        assert!(matches!(
            cancel.entry.event,
            EventType::ExecutionCancelled { .. }
        ));
        assert_eq!(cancel.entry.sequence, 2);
        assert_eq!(*state.status(), ExecutionStatus::Cancelled);
        assert!(state.is_terminal());
        assert_eq!(state.journal().len(), 3);
    }

    #[test]
    fn handle_cancel_without_request_rejects_and_preserves_state() {
        let mut state = new_state();
        let now = Utc::now();

        let err = state
            .handle(
                Command::Cancel {
                    reason: "nope".into(),
                },
                now,
            )
            .expect_err("Cancel without prior RequestCancel must fail");

        assert!(matches!(
            err,
            JournalError::InvariantViolation(JournalViolation::CancelledWithoutRequest { .. })
        ));
        // State unchanged — no mutation on validation failure
        assert_eq!(*state.status(), ExecutionStatus::Running);
        assert_eq!(state.journal().len(), 1);
        assert_eq!(state.next_child_seq(), 0);
    }

    // ── Task 8: Allocating commands ──

    #[test]
    fn allocating_schedule_invoke_assigns_child_zero() {
        let mut state = new_state();
        let now = Utc::now();

        let result = state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "do_work".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .expect("ScheduleInvoke on Running must succeed");

        let child_0 = state.execution_id().child(0).unwrap();
        assert_eq!(result.allocated_id, Some(child_0.clone()));
        assert!(matches!(
            result.entry.event,
            EventType::InvokeScheduled { .. }
        ));
        assert_eq!(state.next_child_seq(), 1);
        assert_eq!(state.allocated_children().len(), 1);
        assert!(state.allocated_children().contains(&child_0));
    }

    #[test]
    fn allocating_capture_random_assigns_child_and_caches() {
        let mut state = new_state();
        let now = Utc::now();

        let result = state
            .handle(
                Command::CaptureRandom {
                    value: vec![0xAB, 0xCD],
                },
                now,
            )
            .expect("CaptureRandom on Running must succeed");

        let child_0 = state.execution_id().child(0).unwrap();
        assert_eq!(result.allocated_id, Some(child_0.clone()));
        assert_eq!(
            state.replay_cache().get_random(&child_0),
            Some(&[0xAB, 0xCD][..])
        );
    }

    #[test]
    fn allocating_capture_time_records_timestamp_and_caches() {
        let mut state = new_state();
        let now = Utc::now();

        let result = state
            .handle(Command::CaptureTime { time: now }, now)
            .expect("CaptureTime on Running must succeed");

        let child_0 = state.execution_id().child(0).unwrap();
        assert_eq!(result.allocated_id, Some(child_0.clone()));
        assert_eq!(state.replay_cache().get_time(&child_0), Some(now));
    }

    #[test]
    fn allocating_schedule_timer_assigns_child() {
        let mut state = new_state();
        let now = Utc::now();
        let fire_at = now + chrono::Duration::seconds(5);

        let result = state
            .handle(
                Command::ScheduleTimer {
                    duration: Duration::from_secs(5),
                    fire_at,
                },
                now,
            )
            .expect("ScheduleTimer on Running must succeed");

        let child_0 = state.execution_id().child(0).unwrap();
        assert_eq!(result.allocated_id, Some(child_0));
        assert!(matches!(
            result.entry.event,
            EventType::TimerScheduled { .. }
        ));
    }

    #[test]
    fn sequential_allocating_commands_produce_sequential_children() {
        let mut state = new_state();
        let now = Utc::now();

        let r0 = state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "a".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();

        let r1 = state
            .handle(Command::CaptureRandom { value: vec![0x01] }, now)
            .unwrap();

        let r2 = state
            .handle(Command::CaptureTime { time: now }, now)
            .unwrap();

        let child_0 = state.execution_id().child(0).unwrap();
        let child_1 = state.execution_id().child(1).unwrap();
        let child_2 = state.execution_id().child(2).unwrap();

        assert_eq!(r0.allocated_id, Some(child_0));
        assert_eq!(r1.allocated_id, Some(child_1));
        assert_eq!(r2.allocated_id, Some(child_2));
        assert_eq!(state.next_child_seq(), 3);
        assert_eq!(state.allocated_children().len(), 3);
    }

    // ── Task 9: Referencing commands ──

    #[test]
    fn start_invoke_after_schedule_succeeds_with_no_allocation() {
        let mut state = new_state();
        let now = Utc::now();

        // Schedule → child(0)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "work".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();

        let child_0 = state.execution_id().child(0).unwrap();

        // Start — referencing command, no allocation
        let result = state
            .handle(
                Command::StartInvoke {
                    promise_id: child_0,
                    attempt: 1,
                },
                now,
            )
            .expect("StartInvoke after Schedule must succeed");

        assert!(result.allocated_id.is_none());
        assert!(matches!(
            result.entry.event,
            EventType::InvokeStarted { .. }
        ));
    }

    #[test]
    fn complete_invoke_caches_result_in_replay() {
        let mut state = new_state();
        let now = Utc::now();

        // Schedule → Start → Complete
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "work".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();

        let child_0 = state.execution_id().child(0).unwrap();

        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_0.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        let result_payload = Payload::new(vec![42], Codec::Json);
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: child_0.clone(),
                    result: result_payload.clone(),
                    attempt: 1,
                },
                now,
            )
            .expect("CompleteInvoke after Start must succeed");

        assert_eq!(
            state.replay_cache().get_invoke(&child_0),
            Some(&result_payload)
        );
    }

    #[test]
    fn fire_timer_after_schedule_completes_and_caches() {
        let mut state = new_state();
        let now = Utc::now();
        let fire_at = now + chrono::Duration::seconds(5);

        // ScheduleTimer → child(0)
        state
            .handle(
                Command::ScheduleTimer {
                    duration: Duration::from_secs(5),
                    fire_at,
                },
                now,
            )
            .unwrap();

        let child_0 = state.execution_id().child(0).unwrap();

        // FireTimer
        state
            .handle(
                Command::FireTimer {
                    promise_id: child_0.clone(),
                },
                fire_at,
            )
            .expect("FireTimer after ScheduleTimer must succeed");

        assert!(state.replay_cache().is_timer_complete(&child_0));
    }

    #[test]
    fn signal_deliver_then_consume_populates_replay() {
        let mut state = new_state();
        let now = Utc::now();
        let sig_payload = Payload::new(vec![99], Codec::Json);

        // DeliverSignal — non-allocating
        let deliver_result = state
            .handle(
                Command::DeliverSignal {
                    signal_name: "approval".into(),
                    payload: sig_payload.clone(),
                    delivery_id: 1,
                },
                now,
            )
            .unwrap();
        assert!(deliver_result.allocated_id.is_none());

        // ConsumeSignal — allocating → child(0)
        state
            .handle(
                Command::ConsumeSignal {
                    signal_name: "approval".into(),
                    payload: sig_payload.clone(),
                    delivery_id: 1,
                },
                now,
            )
            .expect("ConsumeSignal after DeliverSignal must succeed");

        let child_0 = state.execution_id().child(0).unwrap();
        assert_eq!(
            state.replay_cache().get_signal(&child_0),
            Some(&sig_payload)
        );
    }

    // ── Task 10: JoinSet commands ──

    #[test]
    fn joinset_lifecycle_create_submit_consume() {
        let mut state = new_state();
        let now = Utc::now();

        // CreateJoinSet → child(0)
        let js_result = state
            .handle(Command::CreateJoinSet, now)
            .expect("CreateJoinSet must succeed");
        let child_0 = state.execution_id().child(0).unwrap();
        assert_eq!(js_result.allocated_id, Some(child_0.clone()));
        let js_id = JoinSetId(child_0);

        // ScheduleInvoke → child(1)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "task_a".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let child_1 = state.execution_id().child(1).unwrap();

        // StartInvoke + CompleteInvoke
        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_1.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();
        let invoke_result = Payload::new(vec![1], Codec::Json);
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: child_1.clone(),
                    result: invoke_result.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // SubmitToJoinSet
        state
            .handle(
                Command::SubmitToJoinSet {
                    join_set_id: js_id.clone(),
                    promise_id: child_1.clone(),
                },
                now,
            )
            .expect("SubmitToJoinSet must succeed");

        // ConsumeFromJoinSet
        state
            .handle(
                Command::ConsumeFromJoinSet {
                    join_set_id: js_id,
                    promise_id: child_1,
                    result: invoke_result,
                },
                now,
            )
            .expect("ConsumeFromJoinSet must succeed");
    }

    #[test]
    fn joinset_two_members_any_await_consume_first_completed() {
        let mut state = new_state();
        let now = Utc::now();

        // CreateJoinSet → child(0)
        state.handle(Command::CreateJoinSet, now).unwrap();
        let child_0 = state.execution_id().child(0).unwrap();
        let js_id = JoinSetId(child_0);

        // ScheduleInvoke "a" → child(1)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "a".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let pid_a = state.execution_id().child(1).unwrap();

        // ScheduleInvoke "b" → child(2)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "b".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let pid_b = state.execution_id().child(2).unwrap();

        // Submit both to JoinSet
        state
            .handle(
                Command::SubmitToJoinSet {
                    join_set_id: js_id.clone(),
                    promise_id: pid_a.clone(),
                },
                now,
            )
            .unwrap();
        state
            .handle(
                Command::SubmitToJoinSet {
                    join_set_id: js_id.clone(),
                    promise_id: pid_b.clone(),
                },
                now,
            )
            .unwrap();

        // Await([pid_a, pid_b], Any) → Blocked
        state
            .handle(
                Command::Await {
                    waiting_on: vec![pid_a.clone(), pid_b.clone()],
                    kind: AwaitKind::Any,
                },
                now,
            )
            .unwrap();
        assert!(matches!(state.status(), ExecutionStatus::Blocked { .. }));

        // Complete pid_b first
        state
            .handle(
                Command::StartInvoke {
                    promise_id: pid_b.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();
        let result_b = Payload::new(vec![2], Codec::Json);
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: pid_b.clone(),
                    result: result_b.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // Resume
        state.handle(Command::Resume, now).unwrap();
        assert_eq!(*state.status(), ExecutionStatus::Running);

        // ConsumeFromJoinSet(pid_b) — the first completed
        state
            .handle(
                Command::ConsumeFromJoinSet {
                    join_set_id: js_id,
                    promise_id: pid_b,
                    result: result_b,
                },
                now,
            )
            .expect("ConsumeFromJoinSet for completed pid must succeed");

        assert_eq!(state.next_child_seq(), 3);
    }

    #[test]
    fn submit_to_nonexistent_joinset_rejected() {
        let mut state = new_state();
        let now = Utc::now();

        // ScheduleInvoke → child(0)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "work".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let child_0 = state.execution_id().child(0).unwrap();

        // Fabricate a JoinSetId without CreateJoinSet
        let fake_js = JoinSetId(child_0.clone());

        let err = state
            .handle(
                Command::SubmitToJoinSet {
                    join_set_id: fake_js,
                    promise_id: child_0,
                },
                now,
            )
            .expect_err("SubmitToJoinSet without CreateJoinSet must fail");

        assert!(matches!(
            err,
            JournalError::InvariantViolation(JournalViolation::SubmitWithoutCreate { .. })
        ));
    }

    // ── Task 11: Rollback on validation failure ──

    #[test]
    fn start_invoke_without_schedule_rejected_state_unchanged() {
        let mut state = new_state();
        let now = Utc::now();

        // Fabricate a promise_id without prior ScheduleInvoke
        let fabricated = state.execution_id().child(0).unwrap();

        let err = state
            .handle(
                Command::StartInvoke {
                    promise_id: fabricated,
                    attempt: 1,
                },
                now,
            )
            .expect_err("StartInvoke without Schedule must fail");

        assert!(matches!(
            err,
            JournalError::InvariantViolation(JournalViolation::StartedWithoutScheduled { .. })
        ));
        assert_eq!(state.journal().len(), 1);
        assert_eq!(*state.status(), ExecutionStatus::Running);
        assert_eq!(state.next_child_seq(), 0);
        assert!(state.allocated_children().is_empty());
    }

    #[test]
    fn second_terminal_rejected_state_unchanged() {
        let mut state = new_state();
        let now = Utc::now();

        // First Complete → Completed
        state
            .handle(Command::Complete { result: payload() }, now)
            .unwrap();
        assert_eq!(*state.status(), ExecutionStatus::Completed);

        // Second Complete → rejected
        let err = state
            .handle(Command::Complete { result: payload() }, now)
            .expect_err("Second terminal must fail");

        assert!(matches!(
            err,
            JournalError::InvariantViolation(JournalViolation::MultipleTerminalEvents { .. })
        ));
        assert_eq!(state.journal().len(), 2);
        assert_eq!(*state.status(), ExecutionStatus::Completed);
    }

    #[test]
    fn timer_fired_without_scheduled_rejected_state_unchanged() {
        let mut state = new_state();
        let now = Utc::now();

        // Fabricate a promise_id without prior ScheduleTimer
        let fabricated = state.execution_id().child(0).unwrap();

        let err = state
            .handle(
                Command::FireTimer {
                    promise_id: fabricated,
                },
                now,
            )
            .expect_err("FireTimer without ScheduleTimer must fail");

        assert!(matches!(
            err,
            JournalError::InvariantViolation(JournalViolation::TimerFiredWithoutScheduled { .. })
        ));
        assert_eq!(state.journal().len(), 1);
        assert_eq!(*state.status(), ExecutionStatus::Running);
        assert_eq!(state.next_child_seq(), 0);
        assert!(state.allocated_children().is_empty());
    }

    // ── Task 12: recover() round-trip ──

    #[test]
    fn recover_round_trip_matches_handle_state() {
        let mut state = new_state();
        let now = Utc::now();

        // Build up ~5 commands
        state
            .handle(
                Command::CaptureRandom {
                    value: vec![0x01, 0x02],
                },
                now,
            )
            .unwrap();
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "fetch".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let child_1 = state.execution_id().child(1).unwrap();
        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_1.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: child_1,
                    result: Payload::new(vec![7], Codec::Json),
                    attempt: 1,
                },
                now,
            )
            .unwrap();
        state
            .handle(Command::CaptureTime { time: now }, now)
            .unwrap();

        // Extract journal and recover
        let journal = state.journal().to_vec();
        let recovered =
            ExecutionState::recover(journal).expect("recover from valid journal must succeed");

        assert_eq!(*recovered.status(), *state.status());
        assert_eq!(recovered.journal_version(), state.journal_version());
        assert_eq!(recovered.next_child_seq(), state.next_child_seq());
        assert_eq!(recovered.allocated_children(), state.allocated_children());
        assert_eq!(recovered.is_terminal(), state.is_terminal());
        assert_eq!(recovered.execution_id(), state.execution_id());
    }

    #[test]
    fn recover_rejects_corrupted_sequence_numbers() {
        let mut state = new_state();
        let now = Utc::now();

        state
            .handle(Command::CaptureRandom { value: vec![0x01] }, now)
            .unwrap();

        // Extract and tamper with sequence number
        let mut journal = state.journal().to_vec();
        journal[1].sequence = 99; // should be 1

        let err = ExecutionState::recover(journal).expect_err("Corrupted sequence must fail");

        assert!(matches!(
            err,
            JournalError::InvariantViolation(JournalViolation::NonMonotonicSequence { .. })
        ));
    }

    // ── Task 13: Full workflow 25-event scenario ──

    #[test]
    fn full_workflow_25_event_scenario() {
        let mut state = new_state();
        let now = Utc::now();

        // seq 0: ExecutionStarted — already done by new_state()
        assert_eq!(state.journal().len(), 1);
        assert_eq!(*state.status(), ExecutionStatus::Running);

        // seq 1: CaptureRandom → child(0)
        state
            .handle(
                Command::CaptureRandom {
                    value: vec![0x1a, 0x2b],
                },
                now,
            )
            .unwrap();
        let child_0 = state.execution_id().child(0).unwrap();

        // seq 2: ScheduleInvoke("fetch_user") → child(1)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "fetch_user".into(),
                    input: Payload::new(vec![42], Codec::Json),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let child_1 = state.execution_id().child(1).unwrap();

        // seq 3: Await([child(1)], Single) → Blocked
        state
            .handle(
                Command::Await {
                    waiting_on: vec![child_1.clone()],
                    kind: AwaitKind::Single,
                },
                now,
            )
            .unwrap();
        assert!(matches!(state.status(), ExecutionStatus::Blocked { .. }));

        // seq 4: InvokeStarted(child(1), attempt=1)
        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_1.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // seq 5: InvokeCompleted(child(1))
        let user_payload = Payload::new(vec![1, 2, 3], Codec::Json);
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: child_1.clone(),
                    result: user_payload.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // seq 6: Resume → Running
        state.handle(Command::Resume, now).unwrap();
        assert_eq!(*state.status(), ExecutionStatus::Running);

        // seq 7: CreateJoinSet → child(2)
        state.handle(Command::CreateJoinSet, now).unwrap();
        let child_2 = state.execution_id().child(2).unwrap();
        let js_id = JoinSetId(child_2);

        // seq 8: ScheduleInvoke("send_email") → child(3)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "send_email".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let child_3 = state.execution_id().child(3).unwrap();

        // seq 9: SubmitToJoinSet(js, child(3))
        state
            .handle(
                Command::SubmitToJoinSet {
                    join_set_id: js_id.clone(),
                    promise_id: child_3.clone(),
                },
                now,
            )
            .unwrap();

        // seq 10: ScheduleInvoke("send_sms") → child(4)
        state
            .handle(
                Command::ScheduleInvoke {
                    kind: InvokeKind::Function,
                    function_name: "send_sms".into(),
                    input: payload(),
                    retry_policy: None,
                },
                now,
            )
            .unwrap();
        let child_4 = state.execution_id().child(4).unwrap();

        // seq 11: SubmitToJoinSet(js, child(4))
        state
            .handle(
                Command::SubmitToJoinSet {
                    join_set_id: js_id.clone(),
                    promise_id: child_4.clone(),
                },
                now,
            )
            .unwrap();

        // seq 12: Await([child(3), child(4)], Any) → Blocked
        state
            .handle(
                Command::Await {
                    waiting_on: vec![child_3.clone(), child_4.clone()],
                    kind: AwaitKind::Any,
                },
                now,
            )
            .unwrap();
        assert!(matches!(state.status(), ExecutionStatus::Blocked { .. }));

        // seq 13: InvokeStarted(child(4), attempt=1)
        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_4.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // seq 14: InvokeCompleted(child(4))
        let sms_payload = Payload::new(vec![4, 5], Codec::Json);
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: child_4.clone(),
                    result: sms_payload.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // seq 15: Resume → Running
        state.handle(Command::Resume, now).unwrap();
        assert_eq!(*state.status(), ExecutionStatus::Running);

        // seq 16: ConsumeFromJoinSet(js, child(4))
        state
            .handle(
                Command::ConsumeFromJoinSet {
                    join_set_id: js_id.clone(),
                    promise_id: child_4.clone(),
                    result: sms_payload.clone(),
                },
                now,
            )
            .unwrap();

        // seq 17: Await([child(3)], Any) → Blocked
        state
            .handle(
                Command::Await {
                    waiting_on: vec![child_3.clone()],
                    kind: AwaitKind::Any,
                },
                now,
            )
            .unwrap();
        assert!(matches!(state.status(), ExecutionStatus::Blocked { .. }));

        // seq 18: InvokeStarted(child(3), attempt=1)
        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_3.clone(),
                    attempt: 1,
                },
                now,
            )
            .unwrap();

        // seq 19: InvokeRetrying(child(3), attempt=1, error, retry_at)
        let retry_at = now + chrono::Duration::seconds(30);
        state
            .handle(
                Command::RetryInvoke {
                    promise_id: child_3.clone(),
                    failed_attempt: 1,
                    error: ExecutionError::new(ErrorKind::Uncategorized, "timeout"),
                    retry_at,
                },
                now,
            )
            .unwrap();

        // seq 20: InvokeStarted(child(3), attempt=2)
        state
            .handle(
                Command::StartInvoke {
                    promise_id: child_3.clone(),
                    attempt: 2,
                },
                now,
            )
            .unwrap();

        // seq 21: InvokeCompleted(child(3), attempt=2)
        let email_payload = Payload::new(vec![6, 7], Codec::Json);
        state
            .handle(
                Command::CompleteInvoke {
                    promise_id: child_3.clone(),
                    result: email_payload.clone(),
                    attempt: 2,
                },
                now,
            )
            .unwrap();

        // seq 22: Resume → Running
        state.handle(Command::Resume, now).unwrap();
        assert_eq!(*state.status(), ExecutionStatus::Running);

        // seq 23: ConsumeFromJoinSet(js, child(3))
        state
            .handle(
                Command::ConsumeFromJoinSet {
                    join_set_id: js_id,
                    promise_id: child_3.clone(),
                    result: email_payload.clone(),
                },
                now,
            )
            .unwrap();

        // seq 24: Complete → Completed
        let final_result = Payload::new(vec![0xFF], Codec::Json);
        state
            .handle(
                Command::Complete {
                    result: final_result,
                },
                now,
            )
            .unwrap();

        // ── Final assertions ──
        assert_eq!(state.journal().len(), 25);
        assert_eq!(*state.status(), ExecutionStatus::Completed);
        assert!(state.is_terminal());
        assert_eq!(state.next_child_seq(), 5);
        assert_eq!(state.allocated_children().len(), 5);

        // Verify replay cache has entries for all completions
        assert_eq!(
            state.replay_cache().get_random(&child_0),
            Some(&[0x1a, 0x2b][..])
        );
        assert_eq!(
            state.replay_cache().get_invoke(&child_1),
            Some(&user_payload)
        );
        assert_eq!(
            state.replay_cache().get_invoke(&child_4),
            Some(&sms_payload)
        );
        assert_eq!(
            state.replay_cache().get_invoke(&child_3),
            Some(&email_payload)
        );

        // ── recover() round-trip ──
        let journal = state.journal().to_vec();
        let recovered =
            ExecutionState::recover(journal).expect("recover from 25-event journal must succeed");

        assert_eq!(*recovered.status(), *state.status());
        assert_eq!(recovered.journal_version(), state.journal_version());
        assert_eq!(recovered.next_child_seq(), state.next_child_seq());
        assert_eq!(recovered.allocated_children(), state.allocated_children());
        assert_eq!(recovered.is_terminal(), state.is_terminal());
        assert_eq!(recovered.execution_id(), state.execution_id());
    }
}
