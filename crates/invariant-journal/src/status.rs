use std::collections::HashSet;

use invariant_types::{AwaitKind, EventType, ExecutionStatus, JournalEntry, PromiseId};

/// Derive the current execution status by replaying journal events left-to-right.
///
/// This is the canonical recovery path: load persisted entries and fold them into
/// the latest `ExecutionStatus`.
///
/// Complexity: O(n) over `entries.len()`.
///
/// Precondition: journal invariants are enforced upstream (S-2 guarantees the
/// first event is `ExecutionStarted`), so an empty journal is treated as misuse.
pub fn derive_status(entries: &[JournalEntry]) -> ExecutionStatus {
    debug_assert!(
        !entries.is_empty(),
        "derive_status expects non-empty journal (S-2: starts_with_started)"
    );
    debug_assert!(
        matches!(
            entries.first().map(|e| &e.event),
            Some(EventType::ExecutionStarted { .. })
        ),
        "S-2 violated: first event must be ExecutionStarted"
    );
    entries
        .iter()
        .fold(ExecutionStatus::Running, |status, entry| {
            derive_next_status(status, &entry.event)
        })
}

/// Apply a single-event status transition.
///
/// Use this in append-time paths where status is already known and a new event
/// arrives; this gives O(1) incremental updates instead of re-folding the journal.
///
/// Semantics match one step of `derive_status`: events that do not affect status
/// return the previous `current_status` unchanged.
pub(crate) fn derive_next_status(
    current_status: ExecutionStatus,
    event_type: &EventType,
) -> ExecutionStatus {
    match event_type {
        EventType::ExecutionStarted { .. } => ExecutionStatus::Running,
        EventType::ExecutionAwaiting { waiting_on, kind } => ExecutionStatus::Blocked {
            waiting_on: waiting_on.clone(),
            kind: kind.clone(),
        },
        EventType::ExecutionResumed => ExecutionStatus::Running,
        EventType::CancelRequested { .. } => ExecutionStatus::Cancelling,
        EventType::ExecutionCancelled { .. } => ExecutionStatus::Cancelled,
        EventType::ExecutionCompleted { .. } => ExecutionStatus::Completed,
        EventType::ExecutionFailed { .. } => ExecutionStatus::Failed,
        _ => current_status,
    }
}

/// Collect promise IDs that have produced a completed/cached result in the journal.
///
/// This is the 5-event completion set:
/// - `InvokeCompleted`
/// - `TimerFired`
/// - `RandomGenerated`
/// - `TimeRecorded`
/// - `SignalReceived`
///
/// Intended use:
/// - Replay/cache population and inspection.
///
/// Important:
/// - This is broader than the wait-resolver set used by `can_resume`.
/// - `RandomGenerated` and `TimeRecorded` are immediate value captures and do not
///   participate in blocking/resume satisfaction.
pub fn completed_promises(entries: &[JournalEntry]) -> HashSet<PromiseId> {
    entries
        .iter()
        .filter_map(|entry| match &entry.event {
            EventType::InvokeCompleted { promise_id, .. } => Some(promise_id.clone()),
            EventType::TimerFired { promise_id } => Some(promise_id.clone()),
            EventType::RandomGenerated { promise_id, .. } => Some(promise_id.clone()),
            EventType::TimeRecorded { promise_id, .. } => Some(promise_id.clone()),
            EventType::SignalReceived { promise_id, .. } => Some(promise_id.clone()),
            _ => None,
        })
        .collect()
}

/// Returns the 3-event resolver set used for wait satisfaction in `can_resume`.
///
/// Included events:
/// - `InvokeCompleted`
/// - `TimerFired`
/// - `SignalReceived`
pub fn wait_resolvers(entries: &[JournalEntry]) -> HashSet<PromiseId> {
    entries
        .iter()
        .filter_map(|entry| match &entry.event {
            EventType::InvokeCompleted { promise_id, .. } => Some(promise_id.clone()),
            EventType::TimerFired { promise_id } => Some(promise_id.clone()),
            EventType::SignalReceived { promise_id, .. } => Some(promise_id.clone()),
            _ => None,
        })
        .collect()
}

/// Returns whether a blocked execution can resume based on resolved promises.
///
/// `resolved` should be the resolver set for wait satisfaction:
/// - InvokeCompleted
/// - TimerFired
/// - SignalReceived
///
/// For non-blocked statuses, this returns `false`.
pub fn can_resume(status: &ExecutionStatus, resolved: &HashSet<PromiseId>) -> bool {
    match status {
        ExecutionStatus::Blocked { waiting_on, kind } => match kind {
            AwaitKind::Single | AwaitKind::All => {
                waiting_on.iter().all(|pid| resolved.contains(pid))
            }
            AwaitKind::Any => waiting_on.iter().any(|pid| resolved.contains(pid)),
            AwaitKind::Signal { .. } => {
                debug_assert_eq!(
                    waiting_on.len(),
                    1,
                    "CF-4 violated: AwaitKind::Signal must have exactly one waiting_on promise"
                );
                if waiting_on.len() != 1 {
                    return false;
                }
                resolved.contains(&waiting_on[0])
            }
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use invariant_types::{Codec, Payload};

    use super::*;

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }

    fn payload() -> Payload {
        Payload::new(vec![], Codec::Json)
    }

    fn entry(sequence: u64, event: EventType) -> JournalEntry {
        JournalEntry {
            sequence,
            timestamp: Utc::now(),
            event,
        }
    }

    #[test]
    fn derive_status_matches_incremental_transitions() {
        let p1 = pid(1);

        let entries = vec![
            entry(
                0,
                EventType::ExecutionStarted {
                    component_digest: vec![1, 2, 3],
                    input: payload(),
                    parent_id: None,
                    idempotency_key: "k".into(),
                },
            ),
            entry(
                1,
                EventType::InvokeScheduled {
                    promise_id: p1.clone(),
                    kind: invariant_types::InvokeKind::Function,
                    function_name: "f".into(),
                    input: payload(),
                    retry_policy: None,
                },
            ),
            entry(
                2,
                EventType::ExecutionAwaiting {
                    waiting_on: vec![p1.clone()],
                    kind: AwaitKind::Single,
                },
            ),
            entry(3, EventType::ExecutionResumed),
            entry(
                4,
                EventType::CancelRequested {
                    reason: "stop".into(),
                },
            ),
            entry(
                5,
                EventType::ExecutionFailed {
                    error: "boom".into(),
                },
            ),
        ];

        let folded = derive_status(&entries);
        let incremental = entries
            .iter()
            .fold(ExecutionStatus::Running, |status, e| derive_next_status(status, &e.event));

        assert_eq!(folded, incremental);
    }

    #[test]
    fn wait_resolvers_only_contains_three_resolver_events() {
        let p_invoke = pid(10);
        let p_timer = pid(11);
        let p_signal = pid(12);
        let p_random = pid(13);
        let p_time = pid(14);

        let entries = vec![
            entry(
                0,
                EventType::InvokeCompleted {
                    promise_id: p_invoke.clone(),
                    result: payload(),
                    attempt: 1,
                },
            ),
            entry(
                1,
                EventType::TimerFired {
                    promise_id: p_timer.clone(),
                },
            ),
            entry(
                2,
                EventType::SignalReceived {
                    promise_id: p_signal.clone(),
                    signal_name: "s".into(),
                    payload: payload(),
                    delivery_id: 1,
                },
            ),
            entry(
                3,
                EventType::RandomGenerated {
                    promise_id: p_random.clone(),
                    value: vec![7, 8],
                },
            ),
            entry(
                4,
                EventType::TimeRecorded {
                    promise_id: p_time.clone(),
                    time: Utc::now(),
                },
            ),
        ];

        let resolvers = wait_resolvers(&entries);

        assert!(resolvers.contains(&p_invoke));
        assert!(resolvers.contains(&p_timer));
        assert!(resolvers.contains(&p_signal));
        assert!(!resolvers.contains(&p_random));
        assert!(!resolvers.contains(&p_time));
    }
}
