use std::fmt;

use serde::{Deserialize, Serialize};

/// Canonical category for an execution or invocation failure.
///
/// This is intentionally coarse-grained: it is used for policy decisions
/// (for example retry behavior) and for observability dimensions in logs
/// and metrics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Runtime trap or host-side execution failure.
    ///
    /// Typically retryable when the failure is transient.
    Trap,
    /// Guest/business logic returned an application error.
    ///
    /// Usually a non-retryable, expected outcome.
    UserError,
    /// Execution or invocation exceeded the configured time limit.
    ///
    /// Retryability depends on caller policy and idempotency guarantees.
    Timeout,
    /// Operation was intentionally cancelled.
    ///
    /// This represents a control-flow decision, not necessarily a fault.
    Cancelled,
    /// Replay divergence (nondeterminism) was detected.
    ///
    /// Indicates a deterministic replay invariant violation.
    Nondeterminism,
    /// Catch-all bucket when no specific category applies.
    Uncategorized,
}

/// Structured payload for execution failures and invoke retries.
///
/// This replaces raw string errors with a stable shape that is easy to:
/// - classify (`kind`) for retry/policy decisions,
/// - render (`message`) for user-facing summaries,
/// - enrich (`detail`) with optional low-level diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionError {
    /// Coarse failure category used by policy and observability.
    pub kind: ErrorKind,
    /// Human-readable summary safe to display in normal logs and UIs.
    pub message: String,
    /// Optional diagnostic detail for debugging and deep triage.
    ///
    /// Prefer concise, actionable context. Omit when no extra detail exists.
    pub detail: Option<String>,
}

impl ExecutionError {
    /// Creates an [`ExecutionError`] with required fields only.
    ///
    /// Use [`Self::with_detail`] to attach optional diagnostic context.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: None,
        }
    }

    /// Creates an [`ExecutionError`] with all fields in a single call.
    ///
    /// This is a convenience constructor for call sites that always have
    /// diagnostic detail available and do not need fluent chaining.
    /// Equivalent to `Self::new(kind, message).with_detail(detail)`.
    pub fn new_with_detail(
        kind: ErrorKind,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    /// Adds or replaces the optional diagnostic detail.
    ///
    /// This is a fluent helper so callers can write:
    /// `ExecutionError::new(kind, message).with_detail(detail)`.
    ///
    /// If called multiple times, the last value wins.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)?;
        if let Some(ref detail) = self.detail {
            write!(f, " ({})", detail)?;
        }
        Ok(())
    }
}
