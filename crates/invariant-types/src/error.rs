use thiserror;

#[derive(Clone, Debug, thiserror::Error)]
pub enum DomainError {
    #[error("max call depth of {max} exceeded")]
    MaxCallDepthExceeded { max: usize },
}
