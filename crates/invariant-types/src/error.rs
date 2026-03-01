use thiserror;

#[derive(Clone, Debug, thiserror::Error)]
pub enum DomainError {
    #[error("max call depth of {max} exceeded")]
    MaxCallDepthExceeded { max: usize },

    #[error("max children of {max} exceeded")]
    MaxChildrenExceeded { max: u32 },
}
