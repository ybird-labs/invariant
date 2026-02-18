use invariant_types::{EventType, JoinSetId, JournalEntry, PromiseId, SignalDeliveryId};

/// Returns true if the invocation identified by `pid` was ever scheduled.
///
/// Scan complexity: O(n).
pub fn is_invoke_scheduled(entries: &[JournalEntry], pid: &PromiseId) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::InvokeScheduled { promise_id, .. } => promise_id == pid,
        _ => false,
    })
}

/// Returns true if the invocation identified by `pid` was ever started.
///
/// Scan complexity: O(n).
pub fn is_invoke_started(entries: &[JournalEntry], pid: &PromiseId) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::InvokeStarted { promise_id, .. } => promise_id == pid,
        _ => false,
    })
}

/// Returns true if the invocation identified by `pid` was ever completed.
///
/// Scan complexity: O(n).
pub fn is_invoke_completed(entries: &[JournalEntry], pid: &PromiseId) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::InvokeCompleted { promise_id, .. } => promise_id == pid,
        _ => false,
    })
}

/// Returns true if the timer identified by `pid` was ever scheduled.
///
/// Scan complexity: O(n).
pub fn is_timer_scheduled(entries: &[JournalEntry], pid: &PromiseId) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::TimerScheduled { promise_id, .. } => promise_id == pid,
        _ => false,
    })
}

/// Returns true if the timer identified by `pid` was ever fired.
///
/// Scan complexity: O(n).
pub fn is_timer_fired(entries: &[JournalEntry], pid: &PromiseId) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::TimerFired { promise_id } => promise_id == pid,
        _ => false,
    })
}

/// Returns true if a signal delivery `(name, delivery_id)` exists in the journal.
///
/// This checks durable delivery (`SignalDelivered`), not consumption.
/// Scan complexity: O(n).
pub fn is_signal_delivered(
    entries: &[JournalEntry],
    name: &str,
    delivery_id: SignalDeliveryId,
) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::SignalDelivered {
            signal_name,
            delivery_id: did,
            ..
        } => signal_name == name && *did == delivery_id,
        _ => false,
    })
}

/// Returns true if a signal delivery `(name, delivery_id)` was consumed by workflow code.
///
/// This checks `SignalReceived` entries.
/// Scan complexity: O(n).
pub fn is_signal_consumed(
    entries: &[JournalEntry],
    name: &str,
    delivery_id: SignalDeliveryId,
) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::SignalReceived {
            signal_name,
            delivery_id: did,
            ..
        } => signal_name == name && *did == delivery_id,
        _ => false,
    })
}

/// Returns true if join set `js_id` was created.
///
/// Scan complexity: O(n).
pub fn is_join_set_created(entries: &[JournalEntry], js_id: &JoinSetId) -> bool {
    entries.iter().any(|e| match &e.event {
        EventType::JoinSetCreated { join_set_id } => join_set_id == js_id,
        _ => false,
    })
}

/// Returns submitted members for join set `js_id` in journal order.
///
/// Duplicates are preserved if the journal contains them.
/// Scan complexity: O(n).
pub fn join_set_members(entries: &[JournalEntry], js_id: &JoinSetId) -> Vec<PromiseId> {
    entries
        .iter()
        .filter_map(|e| match &e.event {
            EventType::JoinSetSubmitted {
                join_set_id,
                promise_id,
            } if join_set_id == js_id => Some(promise_id.clone()),
            _ => None,
        })
        .collect()
}

/// Returns consumed members for join set `js_id` in journal order.
///
/// Duplicates are preserved if the journal contains them.
/// Scan complexity: O(n).
pub fn join_set_consumed(entries: &[JournalEntry], js_id: &JoinSetId) -> Vec<PromiseId> {
    entries
        .iter()
        .filter_map(|e| match &e.event {
            EventType::JoinSetAwaited {
                join_set_id,
                promise_id,
                ..
            } if join_set_id == js_id => Some(promise_id.clone()),
            _ => None,
        })
        .collect()
}

/// Returns the first join set that submitted `pid`, if any.
///
/// "First" is based on journal order.
/// Scan complexity: O(n).
pub fn promise_owner(entries: &[JournalEntry], pid: &PromiseId) -> Option<JoinSetId> {
    entries.iter().find_map(|e| match &e.event {
        EventType::JoinSetSubmitted {
            join_set_id,
            promise_id,
        } if promise_id == pid => Some(join_set_id.clone()),
        _ => None,
    })
}

/// Returns true if a cancellation request appears anywhere in the journal.
///
/// Scan complexity: O(n).
pub fn has_cancel_requested(entries: &[JournalEntry]) -> bool {
    entries
        .iter()
        .any(|e| matches!(e.event, EventType::CancelRequested { .. }))
}

/// Returns the first terminal event in journal order, if present.
///
/// Terminal events are `ExecutionCompleted`, `ExecutionFailed`, or `ExecutionCancelled`.
/// Scan complexity: O(n).
pub fn terminal_event(entries: &[JournalEntry]) -> Option<&EventType> {
    entries.iter().find_map(|e| {
        if e.event.is_terminal() {
            Some(&e.event)
        } else {
            None
        }
    })
}

/// Counts retry attempts (`InvokeRetrying`) for invocation `pid`.
///
/// Scan complexity: O(n).
pub fn retry_count(entries: &[JournalEntry], pid: &PromiseId) -> usize {
    entries
        .iter()
        .filter(|e| match &e.event {
            EventType::InvokeRetrying { promise_id, .. } => promise_id == pid,
            _ => false,
        })
        .count()
}
