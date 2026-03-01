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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;
    use invariant_types::{
        Codec, ErrorKind, ExecutionError, InvokeKind, JoinSetId, Payload, PromiseId,
    };

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

    // ── Invoke lifecycle ──

    #[test]
    fn invoke_scheduled_found() {
        let p = pid(1);
        let entries = vec![entry(
            0,
            EventType::InvokeScheduled {
                promise_id: p.clone(),
                kind: InvokeKind::Function,
                function_name: "work".into(),
                input: payload(),
                retry_policy: None,
            },
        )];
        assert!(is_invoke_scheduled(&entries, &p));
    }

    #[test]
    fn invoke_scheduled_not_found() {
        let p = pid(1);
        let other = pid(2);
        let entries = vec![entry(
            0,
            EventType::InvokeScheduled {
                promise_id: p,
                kind: InvokeKind::Function,
                function_name: "work".into(),
                input: payload(),
                retry_policy: None,
            },
        )];
        assert!(!is_invoke_scheduled(&entries, &other));
    }

    #[test]
    fn invoke_started_found() {
        let p = pid(1);
        let entries = vec![entry(
            0,
            EventType::InvokeStarted {
                promise_id: p.clone(),
                attempt: 1,
            },
        )];
        assert!(is_invoke_started(&entries, &p));
        assert!(!is_invoke_started(&entries, &pid(99)));
    }

    #[test]
    fn invoke_completed_found() {
        let p = pid(1);
        let entries = vec![entry(
            0,
            EventType::InvokeCompleted {
                promise_id: p.clone(),
                result: payload(),
                attempt: 1,
            },
        )];
        assert!(is_invoke_completed(&entries, &p));
        assert!(!is_invoke_completed(&entries, &pid(99)));
    }

    // ── Timer lifecycle ──

    #[test]
    fn timer_scheduled_found() {
        let p = pid(1);
        let entries = vec![entry(
            0,
            EventType::TimerScheduled {
                promise_id: p.clone(),
                duration: Duration::from_secs(5),
                fire_at: Utc::now(),
            },
        )];
        assert!(is_timer_scheduled(&entries, &p));
        assert!(!is_timer_scheduled(&entries, &pid(99)));
    }

    #[test]
    fn timer_fired_found() {
        let p = pid(1);
        let entries = vec![entry(
            0,
            EventType::TimerFired {
                promise_id: p.clone(),
            },
        )];
        assert!(is_timer_fired(&entries, &p));
        assert!(!is_timer_fired(&entries, &pid(99)));
    }

    // ── Signal lifecycle ──

    #[test]
    fn signal_delivered_found() {
        let entries = vec![entry(
            0,
            EventType::SignalDelivered {
                signal_name: "approval".into(),
                payload: payload(),
                delivery_id: 42,
            },
        )];
        assert!(is_signal_delivered(&entries, "approval", 42));
    }

    #[test]
    fn signal_delivered_wrong_id() {
        let entries = vec![entry(
            0,
            EventType::SignalDelivered {
                signal_name: "approval".into(),
                payload: payload(),
                delivery_id: 42,
            },
        )];
        assert!(!is_signal_delivered(&entries, "approval", 99));
        assert!(!is_signal_delivered(&entries, "other", 42));
    }

    #[test]
    fn signal_consumed_found() {
        let entries = vec![entry(
            0,
            EventType::SignalReceived {
                promise_id: pid(1),
                signal_name: "approval".into(),
                payload: payload(),
                delivery_id: 7,
            },
        )];
        assert!(is_signal_consumed(&entries, "approval", 7));
        assert!(!is_signal_consumed(&entries, "approval", 99));
        assert!(!is_signal_consumed(&entries, "other", 7));
    }

    // ── JoinSet queries ──

    #[test]
    fn join_set_created_found() {
        let js = JoinSetId(pid(10));
        let entries = vec![entry(
            0,
            EventType::JoinSetCreated {
                join_set_id: js.clone(),
            },
        )];
        assert!(is_join_set_created(&entries, &js));
        assert!(!is_join_set_created(&entries, &JoinSetId(pid(99))));
    }

    #[test]
    fn join_set_members_returns_ordered() {
        let js = JoinSetId(pid(10));
        let p1 = pid(1);
        let p2 = pid(2);
        let p3 = pid(3);
        let other_js = JoinSetId(pid(20));

        let entries = vec![
            entry(
                0,
                EventType::JoinSetSubmitted {
                    join_set_id: js.clone(),
                    promise_id: p1.clone(),
                },
            ),
            // Interleave a different join set — should be ignored
            entry(
                1,
                EventType::JoinSetSubmitted {
                    join_set_id: other_js,
                    promise_id: pid(50),
                },
            ),
            entry(
                2,
                EventType::JoinSetSubmitted {
                    join_set_id: js.clone(),
                    promise_id: p2.clone(),
                },
            ),
            entry(
                3,
                EventType::JoinSetSubmitted {
                    join_set_id: js.clone(),
                    promise_id: p3.clone(),
                },
            ),
        ];

        let members = join_set_members(&entries, &js);
        assert_eq!(members, vec![p1, p2, p3]);
    }

    #[test]
    fn join_set_consumed_returns_ordered() {
        let js = JoinSetId(pid(10));
        let p1 = pid(1);
        let p2 = pid(2);

        let entries = vec![
            entry(
                0,
                EventType::JoinSetAwaited {
                    join_set_id: js.clone(),
                    promise_id: p1.clone(),
                    result: payload(),
                },
            ),
            entry(
                1,
                EventType::JoinSetAwaited {
                    join_set_id: js.clone(),
                    promise_id: p2.clone(),
                    result: payload(),
                },
            ),
        ];

        let consumed = join_set_consumed(&entries, &js);
        assert_eq!(consumed, vec![p1, p2]);
    }

    #[test]
    fn promise_owner_returns_first() {
        let js_a = JoinSetId(pid(10));
        let js_b = JoinSetId(pid(20));
        let p = pid(1);

        let entries = vec![
            entry(
                0,
                EventType::JoinSetSubmitted {
                    join_set_id: js_a.clone(),
                    promise_id: p.clone(),
                },
            ),
            // Second submit to different join set — should not override
            entry(
                1,
                EventType::JoinSetSubmitted {
                    join_set_id: js_b,
                    promise_id: p.clone(),
                },
            ),
        ];

        assert_eq!(promise_owner(&entries, &p), Some(js_a));
        assert_eq!(promise_owner(&entries, &pid(99)), None);
    }

    // ── Cancel / Terminal / Retry ──

    #[test]
    fn has_cancel_requested_true_and_false() {
        let without = vec![entry(
            0,
            EventType::ExecutionStarted {
                component_digest: vec![1],
                input: payload(),
                parent_id: None,
                idempotency_key: "k".into(),
            },
        )];
        assert!(!has_cancel_requested(&without));

        let with = vec![entry(
            0,
            EventType::CancelRequested {
                reason: "stop".into(),
            },
        )];
        assert!(has_cancel_requested(&with));
    }

    #[test]
    fn terminal_event_returns_first() {
        let entries = vec![
            entry(
                0,
                EventType::ExecutionStarted {
                    component_digest: vec![1],
                    input: payload(),
                    parent_id: None,
                    idempotency_key: "k".into(),
                },
            ),
            entry(
                1,
                EventType::InvokeScheduled {
                    promise_id: pid(1),
                    kind: InvokeKind::Function,
                    function_name: "f".into(),
                    input: payload(),
                    retry_policy: None,
                },
            ),
            entry(2, EventType::ExecutionCompleted { result: payload() }),
        ];

        let term = terminal_event(&entries);
        assert!(matches!(term, Some(EventType::ExecutionCompleted { .. })));

        // No terminal in a non-terminal journal
        let no_term = vec![entries[0].clone(), entries[1].clone()];
        assert!(terminal_event(&no_term).is_none());
    }

    #[test]
    fn retry_count_counts_retries() {
        let p = pid(1);
        let other = pid(2);
        let now = Utc::now();

        let entries = vec![
            entry(
                0,
                EventType::InvokeRetrying {
                    promise_id: p.clone(),
                    failed_attempt: 1,
                    error: ExecutionError::new(ErrorKind::Uncategorized, "err"),
                    retry_at: now,
                },
            ),
            entry(
                1,
                EventType::InvokeRetrying {
                    promise_id: p.clone(),
                    failed_attempt: 2,
                    error: ExecutionError::new(ErrorKind::Uncategorized, "err"),
                    retry_at: now,
                },
            ),
            // Different pid — should not count
            entry(
                2,
                EventType::InvokeRetrying {
                    promise_id: other.clone(),
                    failed_attempt: 1,
                    error: ExecutionError::new(ErrorKind::Uncategorized, "err"),
                    retry_at: now,
                },
            ),
        ];

        assert_eq!(retry_count(&entries, &p), 2);
        assert_eq!(retry_count(&entries, &other), 1);
        assert_eq!(retry_count(&entries, &pid(99)), 0);
    }

    // ── Empty journal ──

    #[test]
    fn empty_journal_returns_defaults() {
        let empty: &[JournalEntry] = &[];
        let p = pid(1);
        let js = JoinSetId(pid(10));

        assert!(!is_invoke_scheduled(empty, &p));
        assert!(!is_invoke_started(empty, &p));
        assert!(!is_invoke_completed(empty, &p));
        assert!(!is_timer_scheduled(empty, &p));
        assert!(!is_timer_fired(empty, &p));
        assert!(!is_signal_delivered(empty, "s", 1));
        assert!(!is_signal_consumed(empty, "s", 1));
        assert!(!is_join_set_created(empty, &js));
        assert!(join_set_members(empty, &js).is_empty());
        assert!(join_set_consumed(empty, &js).is_empty());
        assert!(promise_owner(empty, &p).is_none());
        assert!(!has_cancel_requested(empty));
        assert!(terminal_event(empty).is_none());
        assert_eq!(retry_count(empty, &p), 0);
    }
}
