use crate::error::DomainError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_CALL_DEPTH: usize = 64;

/// Encodes position in the call tree using Dewey notation.
///
/// `root` is a SHA-256 hash identifying the execution.
/// `path` encodes the sequence of child operations at each depth.
///
/// Display: `"a1b2c3d4.0.1.3"` (hex of first 4 root bytes + dot-separated path)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromiseId {
    root: [u8; 32],
    path: Vec<u32>,
}

pub type ExecutionId = PromiseId;

impl PromiseId {
    /// Root-level promise (empty path) from a pre-computed hash.
    pub fn new(root: [u8; 32]) -> Self {
        Self {
            root,
            path: Vec::new(),
        }
    }

    /// Derive a deterministic execution root from its defining inputs.
    ///
    /// Computes `SHA-256(digest_len || digest || root_len || root || path_len || path_segs... || key_len || key)`.
    /// Parent fields (root, path) are only included when `parent_id` is `Some`.
    /// Each field is length-prefixed (little-endian u32) to prevent concatenation collisions.
    pub fn promise_root(
        component_digest: &[u8],
        idempotency_key: &str,
        parent_id: Option<&PromiseId>,
    ) -> Self {
        let mut hasher = Sha256::new();

        hasher.update((component_digest.len() as u32).to_le_bytes());
        hasher.update(component_digest);

        if let Some(pid) = parent_id {
            hasher.update((pid.root.len() as u32).to_le_bytes());
            hasher.update(&pid.root);
            hasher.update((pid.path.len() as u32).to_le_bytes());
            for seg in &pid.path {
                hasher.update(seg.to_le_bytes());
            }
        }

        let key_bytes = idempotency_key.as_bytes();
        hasher.update((key_bytes.len() as u32).to_le_bytes());
        hasher.update(key_bytes);

        let hash: [u8; 32] = hasher.finalize().into();
        Self::new(hash)
    }

    /// Create a child promise by appending a sequence number to the path.
    ///
    /// The caller provides `seq` — the local operation counter at this depth.
    ///
    /// Returns `Err(MaxCallDepthExceeded)` if the path already has `MAX_CALL_DEPTH` segments.
    pub fn child(&self, seq: u32) -> Result<Self, DomainError> {
        if self.path.len() >= MAX_CALL_DEPTH {
            return Err(DomainError::MaxCallDepthExceeded {
                max: MAX_CALL_DEPTH,
            });
        }
        let mut new_path = self.path.clone();
        new_path.push(seq);
        Ok(Self {
            root: self.root,
            path: new_path,
        })
    }

    /// Return the parent promise (one level up), or `None` if this is the root.
    pub fn parent(&self) -> Option<Self> {
        if self.path.is_empty() {
            return None;
        }
        let mut parent_path = self.path.clone();
        parent_path.pop();
        Some(Self {
            root: self.root,
            path: parent_path,
        })
    }
    /// Whether this is a root-level promise (empty path, depth 0).
    pub fn is_root(&self) -> bool {
        self.path.is_empty()
    }

    /// Depth in the call tree (0 for root).
    pub fn depth(&self) -> usize {
        self.path.len()
    }

    /// The raw 32-byte root hash.
    pub fn root_bytes(&self) -> &[u8; 32] {
        &self.root
    }

    /// The path segments (sequence numbers at each depth).
    pub fn path(&self) -> &[u32] {
        &self.path
    }
}

impl fmt::Display for PromiseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.root[..4]))?;
        for seg in &self.path {
            write!(f, ".{}", seg)?;
        }
        Ok(())
    }
}
