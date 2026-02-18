use std::path::PathBuf;

use wasmtime::component::Component;

use crate::engine::WasmEngine;
use crate::error::RuntimeError;

// TODO: Implement component loader registry
pub struct ComponentLoader {
    engine: WasmEngine,
}

pub enum ComponentSource {
    Bytes(Vec<u8>),
    FilePath(PathBuf),
    Registry(String),
}

impl ComponentLoader {
    pub fn new(engine: WasmEngine) -> Self {
        Self { engine }
    }

    pub fn load(self, source: ComponentSource) -> Result<Component, RuntimeError> {
        match source {
            ComponentSource::FilePath(path) => Component::from_file(self.engine.get_engine(), path)
                .map_err(RuntimeError::ComponentLoadError),
            ComponentSource::Bytes(bytes) => Component::new(self.engine.get_engine(), bytes)
                .map_err(RuntimeError::ComponentLoadError),
            ComponentSource::Registry(_) => unimplemented!(),
        }
    }
}
