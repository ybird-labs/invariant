use invariant_types::{EventType, JournalEntry};

use crate::error::JournalViolation;

use super::InvariantState;
pub(crate) fn check(state: &InvariantState, entry: &JournalEntry) -> Result<(), JournalViolation> {
    match &entry.event {
        EventType::InvokeStarted { promise_id, .. } => {
            if !state.scheduled_pids.contains(promise_id) {
                return Err(JournalViolation::StartedWithoutScheduled {
                    promise_id: promise_id.clone(),
                    started_seq: entry.sequence,
                });
            }
            if state.completed_pids.contains(promise_id) {
                return Err(JournalViolation::EventAfterCompleted {
                    promise_id: promise_id.clone(),
                    offending_seq: entry.sequence,
                    offending_event: entry.event.name().to_string(),
                });
            }
        }
        EventType::InvokeCompleted { promise_id, .. } => {
            if !state.started_pids.contains(promise_id) {
                return Err(JournalViolation::CompletedWithoutStarted {
                    promise_id: promise_id.clone(),
                    completed_seq: entry.sequence,
                });
            }
        }
        EventType::InvokeRetrying { promise_id, .. } => {
            if !state.started_pids.contains(promise_id) {
                return Err(JournalViolation::RetryingWithoutStarted {
                    promise_id: promise_id.clone(),
                    retrying_seq: entry.sequence,
                });
            }
            if state.completed_pids.contains(promise_id) {
                return Err(JournalViolation::EventAfterCompleted {
                    promise_id: promise_id.clone(),
                    offending_seq: entry.sequence,
                    offending_event: entry.event.name().to_string(),
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
    use invariant_types::{EventType, JournalEntry, PromiseId};
    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }
    fn mk_entry(sequence: u64, event: EventType) -> JournalEntry {
        JournalEntry {
            sequence,
            timestamp: std::time::SystemTime::UNIX_EPOCH.into(),
            event,
        }
    }
    #[test]
    fn precedence_se1_over_se4_for_started() {
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
            JournalViolation::StartedWithoutScheduled {
                promise_id: p,
                started_seq: 3
            }
        );
    }
    #[test]
    fn precedence_se3_over_se4_for_retrying() {
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
                error: "boom".to_string(),
                retry_at: Utc::now(),
            },
        );
        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::RetryingWithoutStarted {
                promise_id: p,
                retrying_seq: 4
            }
        );
    }
}
