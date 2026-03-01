use thiserror;

#[derive(Clone, Debug, thiserror::Error)]
pub enum DomainError {
    #[error("max call depth of {max} exceeded")]
    MaxCallDepthExceeded { max: usize },

    /// `max` is `u32` to match the child-sequence counter width used by `ChildSeqCounter`.
    #[error("max children of {max} exceeded")]
    MaxChildrenExceeded { max: u32 },
}
