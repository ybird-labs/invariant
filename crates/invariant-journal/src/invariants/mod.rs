//! Journal invariant checking engine.
//!
//! Provides two modes of validation:
//! - **Incremental** ([`InvariantState::check_append`]): O(1) per entry via auxiliary state.
//!   Used at append time to reject invalid entries before they hit the journal.
//! - **Batch** ([`validate_journal`]): O(n) full scan that collects all violations.
//!   Used for diagnostics and journal recovery.
//!
//! Invariants are grouped into four sub-modules (21 checks total):
//! - [`structural`] (S-1..S-5): Sequence numbering, lifecycle bookends, terminal uniqueness.
//! - [`side_effects`] (SE-1..SE-4): Invoke lifecycle ordering (Scheduled -> Started -> Completed).
//! - [`control_flow`] (CF-1..CF-4): Timer, signal, and await consistency.
//! - [`join_set`] (JS-1..JS-7): JoinSet creation, submission, and consumption rules.
//!
//! Each sub-module exposes a single `check(&InvariantState, &JournalEntry) -> Result<(), JournalViolation>`
//! function. Sub-modules are read-only over state; all mutations happen in [`InvariantState::apply_entry`].

mod control_flow;
mod join_set;
mod side_effects;
mod structural;

use crate::error::JournalViolation;
use invariant_types::{
    EventType, ExecutionJournal, JoinSetId, JournalEntry, Payload, PromiseId, SignalDeliveryId,
};
use std::collections::{HashMap, HashSet};

/// Accumulated auxiliary state for O(1) incremental invariant checking.
///
/// Each field tracks just enough information from previously ingested entries
/// to validate the next append without rescanning the journal. Fields are
/// `pub(crate)` so sub-module checkers can read them; only [`apply_entry`]
/// mutates them.
#[derive(Clone, Debug, Default)]
pub struct InvariantState {
    /// Number of entries ingested so far. Used by S-1 (expected sequence == len).
    pub(crate) len: usize,

    /// Sequence number of the first terminal event, if any. Used by S-3 and S-4.
    /// `Some` implies a terminal has been seen; `None` means the journal is still open.
    pub(crate) terminal_seq: Option<u64>,

    /// Whether a `CancelRequested` event has been seen. Required by S-5
    /// before `ExecutionCancelled` is allowed.
    pub(crate) has_cancel_requested: bool,

    /// Promise IDs from `InvokeScheduled` events. Checked by SE-1.
    pub(crate) scheduled_pids: HashSet<PromiseId>,

    /// Promise IDs from `InvokeStarted` events. Checked by SE-2 and SE-3.
    pub(crate) started_pids: HashSet<PromiseId>,

    /// Promise IDs from `InvokeCompleted` events. Checked by SE-4 and JS-4.
    pub(crate) completed_pids: HashSet<PromiseId>,

    /// Promise IDs from `TimerScheduled` events. Checked by CF-1.
    pub(crate) scheduled_timer_pids: HashSet<PromiseId>,

    /// Delivered signals keyed by `(name, delivery_id)`, with payload stored
    /// for the equality check in CF-2.
    pub(crate) delivered_signals: HashMap<(String, SignalDeliveryId), Payload>,

    /// Signal deliveries already consumed by a `SignalReceived`. Checked by CF-3.
    pub(crate) consumed_signal_deliveries: HashSet<(String, SignalDeliveryId)>,

    /// Join set IDs from `JoinSetCreated` events. Checked by JS-1.
    pub(crate) created_joinsets: HashSet<JoinSetId>,

    /// Join sets that have had at least one `JoinSetAwaited`. Checked by JS-2
    /// to freeze further submissions.
    pub(crate) awaited_joinsets: HashSet<JoinSetId>,

    /// `(join_set_id, promise_id)` pairs from `JoinSetSubmitted`. Checked by JS-3.
    pub(crate) submitted_pairs: HashSet<(JoinSetId, PromiseId)>,

    /// `(join_set_id, promise_id)` pairs from `JoinSetAwaited`. Checked by JS-5.
    pub(crate) consumed_pairs: HashSet<(JoinSetId, PromiseId)>,

    /// Per join set: `(submitted_count, awaited_count)`. Checked by JS-6.
    pub(crate) joinset_counts: HashMap<JoinSetId, (u32, u32)>,

    /// Maps each promise to its owning join set (first writer wins). Checked by JS-7.
    pub(crate) pid_owner: HashMap<PromiseId, JoinSetId>,
}

impl InvariantState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate and ingest a single journal entry (incremental path).
    ///
    /// Runs all 21 invariant checks against the current accumulated state,
    /// then updates state on success. Short-circuits on the first violation
    /// within each group, and bails across groups via `?`.
    pub fn check_append(&mut self, entry: &JournalEntry) -> Result<(), JournalViolation> {
        structural::check(self, entry)?;
        side_effects::check(self, entry)?;
        control_flow::check(self, entry)?;
        join_set::check(self, entry)?;
        self.apply_entry(entry);
        Ok(())
    }

    /// Run all invariant groups, collecting up to one violation per group.
    ///
    /// Unlike [`check_append`], this does not short-circuit across groups --
    /// all four groups run regardless of earlier failures. Used by
    /// [`validate_journal`] to surface multiple independent issues in a
    /// single pass over a corrupt journal.
    fn collect_entry_violations(
        &self,
        entry: &JournalEntry,
        violations: &mut Vec<JournalViolation>,
    ) {
        if let Err(v) = structural::check(self, entry) {
            violations.push(v);
        }
        if let Err(v) = side_effects::check(self, entry) {
            violations.push(v);
        }
        if let Err(v) = control_flow::check(self, entry) {
            violations.push(v);
        }
        if let Err(v) = join_set::check(self, entry) {
            violations.push(v);
        }
    }

    /// Update auxiliary state after an entry passes validation (or is force-applied
    /// during batch validation).
    ///
    /// Centralized here rather than spread across sub-modules so that all state
    /// mutations are visible in one place. Increments `len` as the final step.
    fn apply_entry(&mut self, entry: &JournalEntry) {
        match &entry.event {
            // S-3/S-4: record first terminal sequence number
            EventType::ExecutionCompleted { .. }
            | EventType::ExecutionFailed { .. }
            | EventType::ExecutionCancelled { .. } => {
                self.terminal_seq.get_or_insert(entry.sequence);
            }
            // S-5: gate for ExecutionCancelled
            EventType::CancelRequested { .. } => {
                self.has_cancel_requested = true;
            }
            // SE-1: InvokeStarted requires this
            EventType::InvokeScheduled { promise_id, .. } => {
                self.scheduled_pids.insert(promise_id.clone());
            }
            // SE-2, SE-3: InvokeCompleted and InvokeRetrying require this
            EventType::InvokeStarted { promise_id, .. } => {
                self.started_pids.insert(promise_id.clone());
            }
            // SE-4: blocks further Started/Retrying; JS-4: gate for JoinSetAwaited
            EventType::InvokeCompleted { promise_id, .. } => {
                self.completed_pids.insert(promise_id.clone());
            }
            // CF-1: TimerFired requires this
            EventType::TimerScheduled { promise_id, .. } => {
                self.scheduled_timer_pids.insert(promise_id.clone());
            }
            // CF-2: SignalReceived checks name + delivery_id + payload match
            EventType::SignalDelivered {
                signal_name,
                payload,
                delivery_id,
            } => {
                self.delivered_signals
                    .insert((signal_name.clone(), *delivery_id), payload.clone());
            }
            // CF-3: tracks consumed deliveries for duplicate detection
            EventType::SignalReceived {
                signal_name,
                delivery_id,
                ..
            } => {
                self.consumed_signal_deliveries
                    .insert((signal_name.clone(), *delivery_id));
            }
            // JS-1: JoinSetSubmitted requires this
            EventType::JoinSetCreated { join_set_id } => {
                self.created_joinsets.insert(join_set_id.clone());
            }
            // JS-2 (submitted_pairs), JS-6 (counts), JS-7 (pid_owner)
            EventType::JoinSetSubmitted {
                join_set_id,
                promise_id,
            } => {
                self.submitted_pairs
                    .insert((join_set_id.clone(), promise_id.clone()));

                let counts = self
                    .joinset_counts
                    .entry(join_set_id.clone())
                    .or_insert((0, 0));
                counts.0 = counts.0.saturating_add(1);

                self.pid_owner
                    .entry(promise_id.clone())
                    .or_insert(join_set_id.clone());
            }
            // JS-2 (freezes set), JS-5 (consumed_pairs), JS-6 (counts)
            EventType::JoinSetAwaited {
                join_set_id,
                promise_id,
                ..
            } => {
                self.awaited_joinsets.insert(join_set_id.clone());
                self.consumed_pairs
                    .insert((join_set_id.clone(), promise_id.clone()));

                let counts = self
                    .joinset_counts
                    .entry(join_set_id.clone())
                    .or_insert((0, 0));
                counts.1 = counts.1.saturating_add(1);
            }
            // Events that don't contribute to invariant state:
            // ExecutionStarted, ExecutionAwaiting, ExecutionResumed,
            // InvokeRetrying, TimerFired, RandomGenerated, TimeRecorded
            _ => {}
        }
        self.len += 1;
    }
}

/// Batch-validate an entire journal, returning all detected violations.
///
/// Creates a fresh [`InvariantState`] and feeds every entry through
/// [`InvariantState::collect_entry_violations`], always applying state
/// regardless of errors so that later entries are checked against accurate
/// accumulated state. An empty journal is reported as
/// [`JournalViolation::MissingExecutionStarted`].
pub fn validate_journal(journal: &ExecutionJournal) -> Vec<JournalViolation> {
    if journal.entries.is_empty() {
        return vec![JournalViolation::MissingExecutionStarted {
            first_event: "<empty>".to_string(),
        }];
    }

    let mut state = InvariantState::new();
    let mut violations = Vec::new();

    for entry in &journal.entries {
        state.collect_entry_violations(entry, &mut violations);
        state.apply_entry(entry);
    }

    violations
}
