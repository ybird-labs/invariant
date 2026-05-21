use crate::event::{AwaitKind, EventType};
use crate::promise_id::{ExecutionId, PromiseId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single entry in the journal's append-only event log.
///
/// Sequence is 0-indexed and monotonically increasing.
/// Timestamp is wall-clock for debugging only — NOT used in replay logic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub event: EventType,
}

/// Derived execution status. Not stored independently — derived by
/// folding over journal entries. Only 7 of the 20 event types change status.
///
/// See JOURNAL_DESIGN.md State Machine section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Running,
    Blocked {
        waiting_on: Vec<PromiseId>,
        kind: AwaitKind,
    },
    /// Cancel requested, cleanup in progress.
    Cancelling,
    /// Terminal.
    Completed,
    /// Terminal.
    Failed,
    /// Terminal.
    Cancelled,
}

impl ExecutionStatus {
    /// Whether the execution has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "Running"),
            Self::Blocked { .. } => write!(f, "Blocked"),
            Self::Cancelling => write!(f, "Cancelling"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
            Self::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// The full journal for an execution. Persistence-level struct.
///
/// Version = `entries.len()`. Flat structure, simple storage, natural time ordering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionJournal {
    pub execution_id: ExecutionId,
    pub entries: Vec<JournalEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Codec, Payload};

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }

    #[test]
    fn execution_status_display_should_render_stable_names_for_all_variants() {
        let cases = [
            (ExecutionStatus::Running, "Running"),
            (
                ExecutionStatus::Blocked {
                    waiting_on: vec![pid(1)],
                    kind: AwaitKind::Single,
                },
                "Blocked",
            ),
            (ExecutionStatus::Cancelling, "Cancelling"),
            (ExecutionStatus::Completed, "Completed"),
            (ExecutionStatus::Failed, "Failed"),
            (ExecutionStatus::Cancelled, "Cancelled"),
        ];

        for (status, expected) in cases {
            assert_eq!(status.to_string(), expected);
        }
    }

    #[test]
    fn blocked_status_display_should_ignore_wait_payload_details() {
        let status = ExecutionStatus::Blocked {
            waiting_on: vec![pid(2)],
            kind: AwaitKind::Signal {
                name: "ready".into(),
                promise_id: pid(2),
            },
        };

        assert_eq!(status.to_string(), "Blocked");

        // Keep Payload imported in this test module's compilation unit alongside
        // journal types that commonly carry payloads.
        let _ = Payload::new(Vec::new(), Codec::Json);
    }
}
