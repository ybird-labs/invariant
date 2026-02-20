//! Side-effect invariants (SE-1 through SE-4).
//!
//! These checks enforce the three-phase invoke lifecycle:
//! Scheduled → Started → Completed. Each phase is gated on its predecessor,
//! and `InvokeCompleted` is a terminal absorbing state — no further Started,
//! Retrying, or Completed events may reference the same promise after it.
//!
//! SE-3 is intentionally stricter than the Quint spec: it checks the
//! `(promise_id, failed_attempt)` pair rather than just `promise_id`,
//! ensuring that a retry references the exact attempt that was started.

use invariant_types::{ErrorKind, EventType, ExecutionError, JournalEntry};

use crate::error::JournalViolation;

use super::InvariantState;

/// Validate side-effect invariants against the current accumulated state.
///
/// Within each event arm, SE-4 (completed finality) is checked before the
/// predecessor checks (SE-1, SE-2, SE-3). This precedence prevents
/// misleading "missing predecessor" errors when the real problem is that
/// the promise lifecycle has already terminated.
pub(crate) fn check(state: &InvariantState, entry: &JournalEntry) -> Result<(), JournalViolation> {
    match &entry.event {
        // InvokeStarted: SE-4 (finality) then SE-1 (requires prior Scheduled).
        EventType::InvokeStarted { promise_id, .. } => {
            // SE-4: reject if this promise already completed.
            if state.completed_pids.contains(promise_id) {
                return Err(JournalViolation::EventAfterCompleted {
                    promise_id: promise_id.clone(),
                    offending_seq: entry.sequence,
                    offending_event: entry.event.name().to_string(),
                });
            }
            // SE-1: Started requires a preceding Scheduled for the same promise.
            if !state.scheduled_pids.contains(promise_id) {
                return Err(JournalViolation::StartedWithoutScheduled {
                    promise_id: promise_id.clone(),
                    started_seq: entry.sequence,
                });
            }
        }
        // InvokeCompleted: SE-2 (requires prior Started) then SE-4 (no duplicate).
        // Note: SE-2 is checked first here because a Completed without any
        // Started is a more fundamental violation than a second Completed.
        EventType::InvokeCompleted { promise_id, .. } => {
            // SE-2: Completed requires a preceding Started for the same promise.
            if !state.started_pids.contains(promise_id) {
                return Err(JournalViolation::CompletedWithoutStarted {
                    promise_id: promise_id.clone(),
                    completed_seq: entry.sequence,
                });
            }
            // SE-4: reject duplicate Completed for an already-completed promise.
            if state.completed_pids.contains(promise_id) {
                return Err(JournalViolation::EventAfterCompleted {
                    promise_id: promise_id.clone(),
                    offending_seq: entry.sequence,
                    offending_event: entry.event.name().to_string(),
                });
            }
        }
        // InvokeRetrying: SE-4 (finality) then SE-3 (requires matching Started attempt).
        EventType::InvokeRetrying {
            promise_id,
            failed_attempt,
            ..
        } => {
            // SE-4: reject if this promise already completed.
            if state.completed_pids.contains(promise_id) {
                return Err(JournalViolation::EventAfterCompleted {
                    promise_id: promise_id.clone(),
                    offending_seq: entry.sequence,
                    offending_event: entry.event.name().to_string(),
                });
            }
            // SE-3: Retrying requires a Started with the exact (promise_id, attempt) pair.
            // Stricter than Quint (which checks promise_id only) — ensures the
            // retry references the specific attempt that was actually started.
            if !state
                .started_attempts
                .contains(&(promise_id.clone(), *failed_attempt))
            {
                return Err(JournalViolation::RetryingWithoutStarted {
                    promise_id: promise_id.clone(),
                    failed_attempt: *failed_attempt,
                    retrying_seq: entry.sequence,
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
    use crate::error::JournalViolation;
    use chrono::Utc;
    use invariant_types::{Codec, EventType, JournalEntry, Payload, PromiseId};

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
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
    fn precedence_se4_over_se1_for_started() {
        let p = pid(1);
        let state = InvariantState {
            completed_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            3,
            EventType::InvokeStarted {
                promise_id: p.clone(),
                attempt: 1,
            },
        );
        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::EventAfterCompleted {
                promise_id: p,
                offending_seq: 3,
                offending_event: "InvokeStarted".to_string(),
            }
        );
    }
    #[test]
    fn precedence_se4_over_se3_for_retrying() {
        let p = pid(2);
        let state = InvariantState {
            completed_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            4,
            EventType::InvokeRetrying {
                promise_id: p.clone(),
                failed_attempt: 1,
                error: ExecutionError::new(ErrorKind::Uncategorized, "boom"),
                retry_at: Utc::now(),
            },
        );
        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::EventAfterCompleted {
                promise_id: p,
                offending_seq: 4,
                offending_event: "InvokeRetrying".to_string(),
            }
        );
    }

    #[test]
    fn precedence_se2_over_se4_for_completed() {
        let p = pid(9);
        let state = InvariantState {
            completed_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            4,
            EventType::InvokeCompleted {
                promise_id: p.clone(),
                result: payload(),
                attempt: 1,
            },
        );
        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::CompletedWithoutStarted {
                promise_id: p,
                completed_seq: 4,
            }
        );
    }

    #[test]
    fn se1_started_without_scheduled_reports_started_without_scheduled() {
        let p = pid(10);
        let state = InvariantState::default();
        let entry = mk_entry(
            2,
            EventType::InvokeStarted {
                promise_id: p.clone(),
                attempt: 1,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::StartedWithoutScheduled {
                promise_id: p,
                started_seq: 2,
            }
        );
    }

    #[test]
    fn se1_started_with_prior_scheduled_passes() {
        let p = pid(11);
        let state = InvariantState {
            scheduled_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            3,
            EventType::InvokeStarted {
                promise_id: p,
                attempt: 1,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn se2_completed_without_started_reports_completed_without_started() {
        let p = pid(12);
        let state = InvariantState::default();
        let entry = mk_entry(
            4,
            EventType::InvokeCompleted {
                promise_id: p.clone(),
                result: payload(),
                attempt: 1,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::CompletedWithoutStarted {
                promise_id: p,
                completed_seq: 4,
            }
        );
    }

    #[test]
    fn se2_completed_with_prior_started_passes() {
        let p = pid(13);
        let state = InvariantState {
            started_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            5,
            EventType::InvokeCompleted {
                promise_id: p,
                result: payload(),
                attempt: 1,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn se4_duplicate_completed_reports_event_after_completed() {
        let p = pid(16);
        let state = InvariantState {
            started_pids: std::iter::once(p.clone()).collect(),
            completed_pids: std::iter::once(p.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            6,
            EventType::InvokeCompleted {
                promise_id: p.clone(),
                result: payload(),
                attempt: 1,
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::EventAfterCompleted {
                promise_id: p,
                offending_seq: 6,
                offending_event: "InvokeCompleted".to_string(),
            }
        );
    }

    #[test]
    fn se4_completed_other_pid_does_not_block_started_for_this_pid() {
        let blocked = pid(14);
        let allowed = pid(15);
        let state = InvariantState {
            completed_pids: std::iter::once(blocked).collect(),
            scheduled_pids: std::iter::once(allowed.clone()).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            6,
            EventType::InvokeStarted {
                promise_id: allowed,
                attempt: 1,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn se4_completed_other_pid_does_not_block_completed_for_this_pid() {
        let blocked = pid(17);
        let allowed = pid(18);
        let state = InvariantState {
            started_pids: std::iter::once(allowed.clone()).collect(),
            completed_pids: std::iter::once(blocked).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            7,
            EventType::InvokeCompleted {
                promise_id: allowed,
                result: payload(),
                attempt: 1,
            },
        );

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn se3_retrying_with_mismatched_attempt_reports_retrying_without_started() {
        let p = pid(3);
        let state = InvariantState {
            started_pids: std::iter::once(p.clone()).collect(),
            started_attempts: std::iter::once((p.clone(), 2)).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            7,
            EventType::InvokeRetrying {
                promise_id: p.clone(),
                failed_attempt: 1,
                error: ExecutionError::new(ErrorKind::Uncategorized, "boom"),
                retry_at: Utc::now(),
            },
        );

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::RetryingWithoutStarted {
                promise_id: p,
                failed_attempt: 1,
                retrying_seq: 7,
            }
        );
    }

    #[test]
    fn se3_retrying_with_matching_attempt_passes() {
        let p = pid(4);
        let state = InvariantState {
            started_pids: std::iter::once(p.clone()).collect(),
            started_attempts: std::iter::once((p.clone(), 2)).collect(),
            ..Default::default()
        };
        let entry = mk_entry(
            8,
            EventType::InvokeRetrying {
                promise_id: p,
                failed_attempt: 2,
                error: ExecutionError::new(ErrorKind::Uncategorized, "boom"),
                retry_at: Utc::now(),
            },
        );

        assert!(check(&state, &entry).is_ok());
    }
}
