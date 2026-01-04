use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Failed to load component: {0}")]
    ComponentLoadError(#[from] wasmtime::Error),
    #[error("Failed to instantiate component: {0}")]
    ComponentInstantiateError(String),
}
