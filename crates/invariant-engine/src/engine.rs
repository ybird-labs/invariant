use std::sync::Arc;
use std::thread;
use std::time::Duration;
use wasmtime::{Config, Engine};

#[derive(Clone, Debug)]
pub struct WasmEngine {
    engine: Arc<Engine>,
}

impl WasmEngine {
    pub fn get_engine(&self) -> &Arc<Engine> {
        &self.engine
    }
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    epoch_interval_ms: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            epoch_interval_ms: 1000,
        }
    }
}

impl EngineConfig {
    pub fn epoch_interval_ms(mut self, ms: u64) -> Self {
        self.epoch_interval_ms = ms;
        self
    }

    pub fn build_engine(&self) -> Result<WasmEngine, wasmtime::Error> {
        let mut engine_config = Config::default();
        engine_config
            .wasm_component_model(true)
            .async_support(true)
            .cranelift_nan_canonicalization(true)
            .relaxed_simd_deterministic(true)
            .epoch_interruption(true);

        let engine = Engine::new(&engine_config)?;
        let engine_wrapper = Arc::new(engine);
        let engine_weak = engine_wrapper.weak();
        let timeout = Duration::from_millis(self.epoch_interval_ms);
        std::thread::spawn(move || {
            loop {
                thread::sleep(timeout);
                match engine_weak.upgrade() {
                    Some(engine) => engine.increment_epoch(),
                    None => break,
                }
            }
        });
        Ok(WasmEngine {
            engine: engine_wrapper,
        })
    }
}
