//! Control-flow invariants (CF-1 through CF-4).
//!
//! These checks enforce the causal ordering of timer, signal, and await
//! events. Timers follow a two-phase Scheduled → Fired lifecycle (CF-1).
//! Signals follow a Delivered → Received lifecycle with payload integrity
//! (CF-2) and at-most-once consumption (CF-3). The await-signal consistency
//! rule (CF-4) ensures that `ExecutionAwaiting` with `Signal` kind carries
//! exactly one promise in `waiting_on`, matching the Quint spec's
//! `awaitSignalConsistent` invariant. We also enforce set-like semantics
//! for `waiting_on` by rejecting duplicate promise IDs.

use invariant_types::{AwaitKind, EventType, JournalEntry};
use std::collections::HashSet;

use crate::error::JournalViolation;

use super::InvariantState;

/// Validate control-flow invariants against the current accumulated state.
///
/// The `SignalReceived` arm enforces two invariants in precedence order:
/// CF-2 (matching delivery exists) before CF-3 (not already consumed).
/// This mirrors the SE-4-before-SE-1 pattern in `side_effects`: existence
/// is checked first because a "consumed twice" error is misleading when
/// there was never a valid delivery to consume.
pub(crate) fn check(state: &InvariantState, entry: &JournalEntry) -> Result<(), JournalViolation> {
    match &entry.event {
        // CF-1: TimerFired requires prior TimerScheduled for the same promise.
        EventType::TimerFired { promise_id } => {
            if !state.scheduled_timer_pids.contains(promise_id) {
                return Err(JournalViolation::TimerFiredWithoutScheduled {
                    promise_id: promise_id.clone(),
                    fired_seq: entry.sequence,
                });
            }
        }
        // CF-2 / CF-3: SignalReceived must match prior delivery and be consumed once.
        // Precedence: CF-2 (missing/mismatched delivery) before CF-3 (double consume).
        EventType::SignalReceived {
            signal_name,
            payload,
            delivery_id,
            ..
        } => {
            let key = (signal_name.clone(), *delivery_id);

            match state.delivered_signals.get(&key) {
                Some(delivered_payload) if delivered_payload == payload => {}
                _ => {
                    return Err(JournalViolation::SignalReceivedWithoutDelivery {
                        signal_name: signal_name.clone(),
                        delivery_id: *delivery_id,
                        received_seq: entry.sequence,
                    });
                }
            }

            if state.consumed_signal_deliveries.contains(&key) {
                return Err(JournalViolation::SignalConsumedTwice {
                    signal_name: signal_name.clone(),
                    delivery_id: *delivery_id,
                    second_seq: entry.sequence,
                });
            }
        }
        EventType::ExecutionAwaiting { waiting_on, kind } => {
            // Quint models waiting_on as a set. Rust stores Vec for schema compatibility,
            // so enforce no-duplicates at validation time.
            let mut seen: HashSet<&invariant_types::PromiseId> =
                HashSet::with_capacity(waiting_on.len());
            for pid in waiting_on {
                if !seen.insert(pid) {
                    return Err(JournalViolation::AwaitWaitingOnDuplicate {
                        awaiting_seq: entry.sequence,
                        promise_id: pid.clone(),
                    });
                }
            }

            // CF-4: AwaitKind::Signal must wait on exactly one promise.
            if let AwaitKind::Signal { promise_id, .. } = kind {
                if waiting_on.len() != 1 {
                    return Err(JournalViolation::AwaitSignalInconsistent {
                        awaiting_seq: entry.sequence,
                        waiting_on_count: waiting_on.len(),
                    });
                }
                if waiting_on[0] != *promise_id {
                    return Err(JournalViolation::AwaitSignalInconsistent {
                        awaiting_seq: entry.sequence,
                        waiting_on_count: waiting_on.len(),
                    });
                }
            }
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use invariant_types::{Codec, Payload, PromiseId};

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }

    fn payload(bytes: &[u8]) -> Payload {
        Payload::new(bytes.to_vec(), Codec::Json)
    }

    fn mk_entry(sequence: u64, event: EventType) -> JournalEntry {
        JournalEntry {
            sequence,
            timestamp: std::time::SystemTime::UNIX_EPOCH.into(),
            event,
        }
    }

    #[test]
    fn cf1_timer_fired_without_scheduled_reports_timer_fired_without_scheduled() {
        let p = pid(1);
        let state = InvariantState::default();
        let entry = mk_entry(
            2,
            EventType::TimerFired {
                promise_id: p.clone(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::TimerFiredWithoutScheduled {
                promise_id: p,
                fired_seq: 2,
            }
        );
    }

    #[test]
    fn cf1_timer_fired_with_prior_scheduled_passes() {
        let p = pid(2);
        let state = InvariantState {
            scheduled_timer_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(3, EventType::TimerFired { promise_id: p });

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn cf2_signal_received_without_delivery_reports_signal_received_without_delivery() {
        let recv_pid = pid(3);
        let state = InvariantState::default();
        let entry = mk_entry(
            4,
            EventType::SignalReceived {
                promise_id: recv_pid,
                signal_name: "sig".to_string(),
                payload: payload(b"p"),
                delivery_id: 7,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SignalReceivedWithoutDelivery {
                signal_name: "sig".to_string(),
                delivery_id: 7,
                received_seq: 4,
            }
        );
    }

    #[test]
    fn cf2_signal_received_with_payload_mismatch_reports_signal_received_without_delivery() {
        let recv_pid = pid(4);
        let state = InvariantState {
            delivered_signals: std::iter::once((("sig".to_string(), 8), payload(b"expected")))
                .collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            5,
            EventType::SignalReceived {
                promise_id: recv_pid,
                signal_name: "sig".to_string(),
                payload: payload(b"actual"),
                delivery_id: 8,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SignalReceivedWithoutDelivery {
                signal_name: "sig".to_string(),
                delivery_id: 8,
                received_seq: 5,
            }
        );
    }

    #[test]
    fn cf2_signal_received_with_matching_delivery_passes() {
        let recv_pid = pid(5);
        let state = InvariantState {
            delivered_signals: std::iter::once((("sig".to_string(), 9), payload(b"ok"))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            6,
            EventType::SignalReceived {
                promise_id: recv_pid,
                signal_name: "sig".to_string(),
                payload: payload(b"ok"),
                delivery_id: 9,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn cf3_signal_consumed_twice_reports_signal_consumed_twice() {
        let recv_pid = pid(6);
        let state = InvariantState {
            delivered_signals: std::iter::once((("sig".to_string(), 10), payload(b"ok"))).collect(),
            consumed_signal_deliveries: std::iter::once(("sig".to_string(), 10)).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            7,
            EventType::SignalReceived {
                promise_id: recv_pid,
                signal_name: "sig".to_string(),
                payload: payload(b"ok"),
                delivery_id: 10,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SignalConsumedTwice {
                signal_name: "sig".to_string(),
                delivery_id: 10,
                second_seq: 7,
            }
        );
    }

    #[test]
    fn precedence_cf2_over_cf3_when_delivery_missing_and_already_consumed() {
        let recv_pid = pid(7);
        let state = InvariantState {
            consumed_signal_deliveries: std::iter::once(("sig".to_string(), 11)).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            8,
            EventType::SignalReceived {
                promise_id: recv_pid,
                signal_name: "sig".to_string(),
                payload: payload(b"ok"),
                delivery_id: 11,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SignalReceivedWithoutDelivery {
                signal_name: "sig".to_string(),
                delivery_id: 11,
                received_seq: 8,
            }
        );
    }

    #[test]
    fn precedence_cf2_over_cf3_when_payload_mismatched_and_already_consumed() {
        let recv_pid = pid(8);
        let state = InvariantState {
            delivered_signals: std::iter::once((("sig".to_string(), 12), payload(b"expected")))
                .collect(),
            consumed_signal_deliveries: std::iter::once(("sig".to_string(), 12)).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            9,
            EventType::SignalReceived {
                promise_id: recv_pid,
                signal_name: "sig".to_string(),
                payload: payload(b"actual"),
                delivery_id: 12,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SignalReceivedWithoutDelivery {
                signal_name: "sig".to_string(),
                delivery_id: 12,
                received_seq: 9,
            }
        );
    }

    #[test]
    fn cf4_await_signal_with_zero_waiting_on_reports_await_signal_inconsistent() {
        let state = InvariantState::default();
        let entry = mk_entry(
            10,
            EventType::ExecutionAwaiting {
                waiting_on: vec![],
                kind: AwaitKind::Signal {
                    name: "sig".to_string(),
                    promise_id: pid(100),
                },
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitSignalInconsistent {
                awaiting_seq: 10,
                waiting_on_count: 0,
            }
        );
    }

    #[test]
    fn cf4_await_signal_with_multiple_waiting_on_reports_await_signal_inconsistent() {
        let state = InvariantState::default();
        let entry = mk_entry(
            11,
            EventType::ExecutionAwaiting {
                waiting_on: vec![pid(9), pid(10)],
                kind: AwaitKind::Signal {
                    name: "sig".to_string(),
                    promise_id: pid(101),
                },
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitSignalInconsistent {
                awaiting_seq: 11,
                waiting_on_count: 2,
            }
        );
    }

    #[test]
    fn cf4_await_signal_with_single_waiting_on_passes() {
        let state = InvariantState::default();
        let entry = mk_entry(
            12,
            EventType::ExecutionAwaiting {
                waiting_on: vec![pid(11)],
                kind: AwaitKind::Signal {
                    name: "sig".to_string(),
                    promise_id: pid(11),
                },
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn cf4_non_signal_await_does_not_apply_signal_cardinality_rule() {
        let state = InvariantState::default();
        let entry = mk_entry(
            13,
            EventType::ExecutionAwaiting {
                waiting_on: vec![],
                kind: AwaitKind::Any,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn waiting_on_duplicate_for_any_reports_await_waiting_on_duplicate() {
        let dup = pid(14);
        let state = InvariantState::default();
        let entry = mk_entry(
            15,
            EventType::ExecutionAwaiting {
                waiting_on: vec![dup.clone(), dup.clone()],
                kind: AwaitKind::Any,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitWaitingOnDuplicate {
                awaiting_seq: 15,
                promise_id: dup,
            }
        );
    }

    #[test]
    fn waiting_on_duplicate_for_all_reports_await_waiting_on_duplicate() {
        let dup = pid(15);
        let state = InvariantState::default();
        let entry = mk_entry(
            16,
            EventType::ExecutionAwaiting {
                waiting_on: vec![dup.clone(), dup.clone()],
                kind: AwaitKind::All,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitWaitingOnDuplicate {
                awaiting_seq: 16,
                promise_id: dup,
            }
        );
    }

    #[test]
    fn waiting_on_duplicate_precedes_signal_shape_check() {
        let dup = pid(16);
        let state = InvariantState::default();
        let entry = mk_entry(
            17,
            EventType::ExecutionAwaiting {
                waiting_on: vec![dup.clone(), dup.clone()],
                kind: AwaitKind::Signal {
                    name: "sig".to_string(),
                    promise_id: pid(99),
                },
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitWaitingOnDuplicate {
                awaiting_seq: 17,
                promise_id: dup,
            }
        );
    }

    #[test]
    fn cf4_await_signal_with_mismatched_promise_id_reports_await_signal_inconsistent() {
        let state = InvariantState::default();
        let entry = mk_entry(
            14,
            EventType::ExecutionAwaiting {
                waiting_on: vec![pid(12)],
                kind: AwaitKind::Signal {
                    name: "sig".to_string(),
                    promise_id: pid(13),
                },
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitSignalInconsistent {
                awaiting_seq: 14,
                waiting_on_count: 1,
            }
        );
    }
}
