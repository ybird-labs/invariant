//! JoinSet invariants (JS-1 through JS-7).
//!
//! These checks enforce the lifecycle and ownership rules for concurrent
//! join sets. A join set progresses through three phases: Created →
//! Submitted (one or more promises) → Awaited (consuming results).
//! Once the first `JoinSetAwaited` fires, the set is frozen — no further
//! submissions are allowed (JS-2).
//!
//! Ownership is exclusive: each promise may belong to at most one join set
//! (JS-7), and each `(join_set_id, promise_id)` pair may be consumed at
//! most once (JS-5). The global count invariant (JS-6) ensures awaits
//! never exceed submissions per set.

use invariant_types::{EventType, JournalEntry};

use crate::error::JournalViolation;

use super::InvariantState;

/// Validate join-set invariants against the current accumulated state.
///
/// The `JoinSetSubmitted` arm checks in order: JS-2 (frozen after await)
/// before JS-1 (missing create) before JS-7 (multi-owner). JS-2 takes
/// priority because submitting to a frozen set is a stronger violation
/// than a missing create.
///
/// The `JoinSetAwaited` arm checks in order: JS-3 (membership) → JS-4
/// (completion) → JS-5 (double consume) → JS-6 (count bound). Each
/// check assumes the previous invariants hold, matching the Quint spec's
/// logical dependency chain.
pub(crate) fn check(state: &InvariantState, entry: &JournalEntry) -> Result<(), JournalViolation> {
    match &entry.event {
        EventType::JoinSetSubmitted {
            join_set_id,
            promise_id,
        } => {
            // JS-2: a join set is frozen after first await.
            if state.awaited_joinsets.contains(join_set_id) {
                return Err(JournalViolation::SubmitAfterAwait {
                    join_set_id: join_set_id.clone(),
                    submitted_seq: entry.sequence,
                });
            }

            // JS-1: submit requires prior create.
            if !state.created_joinsets.contains(join_set_id) {
                return Err(JournalViolation::SubmitWithoutCreate {
                    join_set_id: join_set_id.clone(),
                    submitted_seq: entry.sequence,
                });
            }

            // JS-7: a promise may belong to only one join set.
            if let Some(first_js) = state.pid_owner.get(promise_id) {
                if first_js != join_set_id {
                    return Err(JournalViolation::PromiseInMultipleJoinSets {
                        promise_id: promise_id.clone(),
                        first_js: first_js.clone(),
                        second_js: join_set_id.clone(),
                    });
                }
            }
        }
        EventType::JoinSetAwaited {
            join_set_id,
            promise_id,
            ..
        } => {
            let pair = (join_set_id.clone(), promise_id.clone());

            // JS-3: awaited promise must be submitted to this set.
            if !state.submitted_pairs.contains(&pair) {
                return Err(JournalViolation::AwaitedNotMember {
                    join_set_id: join_set_id.clone(),
                    promise_id: promise_id.clone(),
                    awaited_seq: entry.sequence,
                });
            }

            // JS-4: awaited promise must be completed.
            if !state.completed_pids.contains(promise_id) {
                return Err(JournalViolation::AwaitedNotCompleted {
                    promise_id: promise_id.clone(),
                    awaited_seq: entry.sequence,
                });
            }

            // JS-5: the same (join_set_id, promise_id) cannot be consumed twice.
            if state.consumed_pairs.contains(&pair) {
                return Err(JournalViolation::DoubleConsume {
                    join_set_id: join_set_id.clone(),
                    promise_id: promise_id.clone(),
                    second_seq: entry.sequence,
                });
            }

            // JS-6: prospective awaited count must stay <= submitted count.
            let (submitted, awaited) = state
                .joinset_counts
                .get(join_set_id)
                .copied()
                .unwrap_or((0, 0));
            let next_awaited = awaited.saturating_add(1);
            if next_awaited > submitted {
                return Err(JournalViolation::ConsumeExceedsSubmit {
                    join_set_id: join_set_id.clone(),
                    submitted,
                    awaited: next_awaited,
                });
            }
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use invariant_types::{Codec, JoinSetId, Payload, PromiseId};

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }

    fn js(tag: u8) -> JoinSetId {
        JoinSetId(pid(tag))
    }

    fn payload() -> Payload {
        Payload::new(vec![], Codec::Json)
    }

    fn mk_entry(sequence: u64, event: EventType) -> JournalEntry {
        JournalEntry {
            sequence,
            timestamp: std::time::SystemTime::UNIX_EPOCH.into(),
            event,
        }
    }

    #[test]
    fn js1_submit_without_create_reports_submit_without_create() {
        let join_set_id = js(1);
        let promise_id = pid(10);
        let state = InvariantState::default();
        let entry = mk_entry(
            2,
            EventType::JoinSetSubmitted {
                join_set_id: join_set_id.clone(),
                promise_id,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SubmitWithoutCreate {
                join_set_id,
                submitted_seq: 2,
            }
        );
    }

    #[test]
    fn js1_submit_with_create_passes() {
        let join_set_id = js(2);
        let promise_id = pid(11);
        let state = InvariantState {
            created_joinsets: std::iter::once(join_set_id.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            3,
            EventType::JoinSetSubmitted {
                join_set_id,
                promise_id,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn js2_submit_after_await_reports_submit_after_await() {
        let join_set_id = js(3);
        let promise_id = pid(12);
        let state = InvariantState {
            created_joinsets: std::iter::once(join_set_id.clone()).collect(),
            awaited_joinsets: std::iter::once(join_set_id.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            4,
            EventType::JoinSetSubmitted {
                join_set_id: join_set_id.clone(),
                promise_id,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SubmitAfterAwait {
                join_set_id,
                submitted_seq: 4,
            }
        );
    }

    #[test]
    fn precedence_js2_over_js1_when_awaited_without_create() {
        let join_set_id = js(4);
        let promise_id = pid(13);
        let state = InvariantState {
            awaited_joinsets: std::iter::once(join_set_id.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            5,
            EventType::JoinSetSubmitted {
                join_set_id: join_set_id.clone(),
                promise_id,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SubmitAfterAwait {
                join_set_id,
                submitted_seq: 5,
            }
        );
    }

    #[test]
    fn js7_submit_same_promise_to_different_joinset_reports_promise_in_multiple_join_sets() {
        let first_js = js(5);
        let second_js = js(6);
        let promise_id = pid(14);
        let state = InvariantState {
            created_joinsets: std::iter::once(second_js.clone()).collect(),
            pid_owner: std::iter::once((promise_id.clone(), first_js.clone())).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            6,
            EventType::JoinSetSubmitted {
                join_set_id: second_js.clone(),
                promise_id: promise_id.clone(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::PromiseInMultipleJoinSets {
                promise_id,
                first_js,
                second_js,
            }
        );
    }

    #[test]
    fn js7_submit_same_promise_to_same_joinset_passes() {
        let join_set_id = js(7);
        let promise_id = pid(15);
        let state = InvariantState {
            created_joinsets: std::iter::once(join_set_id.clone()).collect(),
            pid_owner: std::iter::once((promise_id.clone(), join_set_id.clone())).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            7,
            EventType::JoinSetSubmitted {
                join_set_id,
                promise_id,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn precedence_js1_over_js7_when_missing_create_and_owned_elsewhere() {
        let first_js = js(8);
        let second_js = js(9);
        let promise_id = pid(16);
        let state = InvariantState {
            pid_owner: std::iter::once((promise_id.clone(), first_js)).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            8,
            EventType::JoinSetSubmitted {
                join_set_id: second_js.clone(),
                promise_id,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::SubmitWithoutCreate {
                join_set_id: second_js,
                submitted_seq: 8,
            }
        );
    }

    #[test]
    fn js3_awaited_not_member_reports_awaited_not_member() {
        let join_set_id = js(10);
        let promise_id = pid(20);
        let state = InvariantState::default();
        let entry = mk_entry(
            9,
            EventType::JoinSetAwaited {
                join_set_id: join_set_id.clone(),
                promise_id: promise_id.clone(),
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitedNotMember {
                join_set_id,
                promise_id,
                awaited_seq: 9,
            }
        );
    }

    #[test]
    fn js4_awaited_not_completed_reports_awaited_not_completed() {
        let join_set_id = js(11);
        let promise_id = pid(21);
        let state = InvariantState {
            submitted_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            joinset_counts: std::iter::once((join_set_id.clone(), (1, 0))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            10,
            EventType::JoinSetAwaited {
                join_set_id,
                promise_id: promise_id.clone(),
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitedNotCompleted {
                promise_id,
                awaited_seq: 10,
            }
        );
    }

    #[test]
    fn js5_double_consume_reports_double_consume() {
        let join_set_id = js(12);
        let promise_id = pid(22);
        let state = InvariantState {
            submitted_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            completed_pids: std::iter::once(promise_id.clone()).collect(),
            consumed_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            joinset_counts: std::iter::once((join_set_id.clone(), (1, 1))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            11,
            EventType::JoinSetAwaited {
                join_set_id: join_set_id.clone(),
                promise_id: promise_id.clone(),
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::DoubleConsume {
                join_set_id,
                promise_id,
                second_seq: 11,
            }
        );
    }

    #[test]
    fn js6_consume_exceeds_submit_reports_consume_exceeds_submit() {
        let join_set_id = js(13);
        let p1 = pid(23);
        let p2 = pid(24);
        let state = InvariantState {
            submitted_pairs: vec![(join_set_id.clone(), p1), (join_set_id.clone(), p2.clone())]
                .into_iter()
                .collect(),
            completed_pids: std::iter::once(p2.clone()).collect(),
            joinset_counts: std::iter::once((join_set_id.clone(), (1, 1))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            12,
            EventType::JoinSetAwaited {
                join_set_id: join_set_id.clone(),
                promise_id: p2,
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::ConsumeExceedsSubmit {
                join_set_id,
                submitted: 1,
                awaited: 2,
            }
        );
    }

    #[test]
    fn js_awaited_valid_member_completed_not_consumed_and_bounded_passes() {
        let join_set_id = js(14);
        let promise_id = pid(25);
        let state = InvariantState {
            submitted_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            completed_pids: std::iter::once(promise_id.clone()).collect(),
            joinset_counts: std::iter::once((join_set_id.clone(), (1, 0))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            13,
            EventType::JoinSetAwaited {
                join_set_id,
                promise_id,
                result: payload(),
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn precedence_js3_over_js4_when_not_member_and_not_completed() {
        let join_set_id = js(15);
        let promise_id = pid(26);
        let state = InvariantState::default();
        let entry = mk_entry(
            14,
            EventType::JoinSetAwaited {
                join_set_id: join_set_id.clone(),
                promise_id: promise_id.clone(),
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitedNotMember {
                join_set_id,
                promise_id,
                awaited_seq: 14,
            }
        );
    }

    #[test]
    fn precedence_js4_over_js5_when_member_not_completed_and_already_consumed() {
        let join_set_id = js(16);
        let promise_id = pid(27);
        let state = InvariantState {
            submitted_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            consumed_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            joinset_counts: std::iter::once((join_set_id.clone(), (1, 1))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            15,
            EventType::JoinSetAwaited {
                join_set_id,
                promise_id: promise_id.clone(),
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::AwaitedNotCompleted {
                promise_id,
                awaited_seq: 15,
            }
        );
    }

    #[test]
    fn precedence_js5_over_js6_when_double_consume_also_exceeds_submit() {
        let join_set_id = js(17);
        let promise_id = pid(28);
        let state = InvariantState {
            submitted_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            completed_pids: std::iter::once(promise_id.clone()).collect(),
            consumed_pairs: std::iter::once((join_set_id.clone(), promise_id.clone())).collect(),
            joinset_counts: std::iter::once((join_set_id.clone(), (1, 1))).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            16,
            EventType::JoinSetAwaited {
                join_set_id: join_set_id.clone(),
                promise_id: promise_id.clone(),
                result: payload(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::DoubleConsume {
                join_set_id,
                promise_id,
                second_seq: 16,
            }
        );
    }
}
