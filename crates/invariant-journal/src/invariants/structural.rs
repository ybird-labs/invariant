//! Structural invariants (S-1 through S-5).
//!
//! These checks enforce the physical integrity of the journal as an
//! append-only, 0-indexed event log with well-defined lifecycle bookends.
//! They run before any domain-level checks because later invariants rely
//! on structural soundness (e.g., sequence == index).

use super::InvariantState;
use crate::error::JournalViolation;
use invariant_types::{EventType, JournalEntry};

/// Validate structural invariants against the current accumulated state.
///
/// Checks are ordered so that sequence integrity (S-1) and lifecycle start
/// (S-2) are verified before terminal-event rules (S-3/S-4/S-5), since the
/// latter depend on coherent sequence numbering. Within the terminal group,
/// S-3 (duplicate terminal) takes precedence over S-4 (post-terminal append).
pub(crate) fn check(state: &InvariantState, entry: &JournalEntry) -> Result<(), JournalViolation> {
    // S-1: Sequence numbers must equal their 0-based array index.
    // `state.len` is the count of entries already ingested, so the next
    // entry must carry `sequence == len`.
    debug_assert!(state.len <= u64::MAX as usize);
    let expected = state.len as u64;
    if entry.sequence != expected {
        return Err(JournalViolation::NonMonotonicSequence {
            entry_index: state.len,
            expected,
            actual: entry.sequence,
        });
    }

    // S-2: The very first event must be `ExecutionStarted`.
    if state.len == 0 && !matches!(entry.event, EventType::ExecutionStarted { .. }) {
        return Err(JournalViolation::MissingExecutionStarted {
            first_event: entry.event.name().to_string(),
        });
    }

    // S-3 / S-4: Terminal event finality.
    // Once a terminal event has been recorded, the journal is sealed:
    //   - Another terminal is a uniqueness violation (S-3).
    //   - A non-terminal is a "terminal not last" violation (S-4).
    if let Some(first_at) = state.terminal_seq {
        if entry.event.is_terminal() {
            return Err(JournalViolation::MultipleTerminalEvents {
                first_at,
                second_at: entry.sequence,
            });
        }
        return Err(JournalViolation::TerminalNotLast {
            terminal_seq: first_at,
            journal_len: state.len.saturating_add(1),
        });
    }

    // S-5: `ExecutionCancelled` requires a prior `CancelRequested`.
    if matches!(entry.event, EventType::ExecutionCancelled { .. }) && !state.has_cancel_requested {
        return Err(JournalViolation::CancelledWithoutRequest {
            cancelled_seq: entry.sequence,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use invariant_types::{Codec, ErrorKind, ExecutionError, Payload};

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

    fn started_event() -> EventType {
        EventType::ExecutionStarted {
            component_digest: vec![1, 2, 3],
            input: payload(),
            parent_id: None,
            idempotency_key: "k".to_string(),
        }
    }

    fn completed_event() -> EventType {
        EventType::ExecutionCompleted { result: payload() }
    }

    fn failed_event() -> EventType {
        EventType::ExecutionFailed {
            error: ExecutionError::new(ErrorKind::Uncategorized, "boom"),
        }
    }

    fn cancelled_event() -> EventType {
        EventType::ExecutionCancelled {
            reason: "cancel".to_string(),
        }
    }

    fn cancel_requested_event() -> EventType {
        EventType::CancelRequested {
            reason: "request".to_string(),
        }
    }

    #[test]
    fn s1_non_monotonic_sequence_reports_expected_actual() {
        let state = InvariantState {
            len: 1,
            ..Default::default()
        };
        let entry = mk_entry(0, started_event());

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::NonMonotonicSequence {
                entry_index: 1,
                expected: 1,
                actual: 0,
            }
        );
    }

    #[test]
    fn s2_first_event_must_be_execution_started() {
        let state = InvariantState::new();
        let entry = mk_entry(0, completed_event());

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::MissingExecutionStarted {
                first_event: "ExecutionCompleted".to_string(),
            }
        );
    }

    #[test]
    fn s3_second_terminal_reports_multiple_terminal_events() {
        let state = InvariantState {
            len: 5,
            terminal_seq: Some(3),
            ..Default::default()
        };
        let entry = mk_entry(5, failed_event());

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::MultipleTerminalEvents {
                first_at: 3,
                second_at: 5,
            }
        );
    }

    #[test]
    fn s4_non_terminal_after_terminal_reports_terminal_not_last() {
        let state = InvariantState {
            len: 4,
            terminal_seq: Some(3),
            ..Default::default()
        };
        let entry = mk_entry(4, cancel_requested_event());

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::TerminalNotLast {
                terminal_seq: 3,
                journal_len: 5,
            }
        );
    }

    #[test]
    fn s5_cancelled_without_prior_request_reports_cancelled_without_request() {
        let state = InvariantState {
            len: 2,
            has_cancel_requested: false,
            ..Default::default()
        };
        let entry = mk_entry(2, cancelled_event());

        let err = check(&state, &entry).unwrap_err();
        assert_eq!(
            err,
            JournalViolation::CancelledWithoutRequest { cancelled_seq: 2 }
        );
    }

    #[test]
    fn precedence_s1_over_s2_when_first_entry_has_wrong_seq_and_event() {
        let state = InvariantState::new();
        let entry = mk_entry(42, completed_event());

        let err = check(&state, &entry).unwrap_err();
        assert!(matches!(err, JournalViolation::NonMonotonicSequence { .. }));
    }

    #[test]
    fn precedence_s3_over_s4_for_second_terminal() {
        let state = InvariantState {
            len: 6,
            terminal_seq: Some(4),
            ..Default::default()
        };
        let entry = mk_entry(6, completed_event());

        let err = check(&state, &entry).unwrap_err();
        assert!(matches!(
            err,
            JournalViolation::MultipleTerminalEvents { .. }
        ));
    }

    #[test]
    fn precedence_s3_over_s5_for_cancelled_after_existing_terminal() {
        let state = InvariantState {
            len: 6,
            terminal_seq: Some(4),
            has_cancel_requested: false,
            ..Default::default()
        };
        let entry = mk_entry(6, cancelled_event());

        let err = check(&state, &entry).unwrap_err();
        assert!(matches!(
            err,
            JournalViolation::MultipleTerminalEvents { .. }
        ));
    }

    #[test]
    fn valid_first_execution_started_passes() {
        let state = InvariantState::new();
        let entry = mk_entry(0, started_event());

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn valid_non_terminal_before_any_terminal_passes() {
        let state = InvariantState {
            len: 1,
            ..Default::default()
        };
        let entry = mk_entry(1, cancel_requested_event());

        assert!(check(&state, &entry).is_ok());
    }

    #[test]
    fn valid_cancelled_with_prior_request_passes() {
        let state = InvariantState {
            len: 2,
            has_cancel_requested: true,
            ..Default::default()
        };
        let entry = mk_entry(2, cancelled_event());

        assert!(check(&state, &entry).is_ok());
    }
}
