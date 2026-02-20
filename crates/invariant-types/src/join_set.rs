use serde::{Deserialize, Serialize};
use std::fmt;

use crate::promise_id::PromiseId;

/// Identifies a JoinSet within an execution.
///
/// PromiseId — `join_set()` allocates a child position
/// via `nextChildSeq++`, consistent with the identity model.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JoinSetId(pub PromiseId);

impl fmt::Display for JoinSetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "js({})", self.0)
    }
}
