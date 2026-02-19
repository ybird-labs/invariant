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
