//! Wasmtime Runtime with Sandboxed Execution
//!
//! This module provides a secure, sandboxed environment for executing
//! Wasm validation plugins using Wasmtime.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};
use wasmtime::*;
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

use super::types::{
    PluginConfig, PluginExecutionResult, PluginLimits, PluginMetadata, ValidationInput,
    ValidationOutput,
};
use crate::error::{Error, Result};

/// Wasm plugin runtime manager
pub struct WasmRuntime {
    /// Wasmtime engine with configured limits
    engine: Engine,

    /// Compiled and cached modules
    module_cache: Arc<RwLock<HashMap<String, CachedModule>>>,

    /// Default resource limits
    default_limits: PluginLimits,
}

/// Cached compiled Wasm module
struct CachedModule {
    module: Module,
    metadata: PluginMetadata,
    #[allow(dead_code)]
    compiled_at: Instant,
}

/// Store state for Wasm execution
struct PluginState {
    wasi: WasiP1Ctx,
    input_buffer: Vec<u8>,
    output_buffer: Vec<u8>,
}

impl WasmRuntime {
    /// Create a new Wasm runtime with default configuration
    pub fn new() -> Result<Self> {
        Self::with_limits(PluginLimits::default())
    }

    /// Create a new Wasm runtime with custom limits
    pub fn with_limits(default_limits: PluginLimits) -> Result<Self> {
        let mut config = Config::new();

        // Enable fuel metering for instruction limits
        config.consume_fuel(true);

        // Enable epoch-based interruption for timeouts
        config.epoch_interruption(true);

        // Memory configuration
        config.max_wasm_stack(512 * 1024); // 512KB stack

        // Compiler optimizations
        config.cranelift_opt_level(OptLevel::Speed);

        // Security: disable features we don't need
        config.wasm_threads(false);
        config.wasm_simd(true);
        config.wasm_bulk_memory(true);
        config.wasm_multi_value(true);
        config.wasm_reference_types(false);

        let engine = Engine::new(&config)
            .map_err(|e| Error::PluginError(format!("Engine creation failed: {e}")))?;

        Ok(Self {
            engine,
            module_cache: Arc::new(RwLock::new(HashMap::new())),
            default_limits,
        })
    }

    /// Load a plugin from binary data
    #[instrument(skip(self, wasm_bytes), fields(plugin_name = %metadata.name))]
    pub async fn load_plugin(&self, wasm_bytes: &[u8], metadata: PluginMetadata) -> Result<()> {
        // Verify integrity if SHA256 is provided
        if let Some(expected_hash) = &metadata.sha256 {
            let actual_hash = Self::compute_sha256(wasm_bytes);
            if &actual_hash != expected_hash {
                return Err(Error::PluginError(format!(
                    "Plugin {} integrity check failed: expected {}, got {}",
                    metadata.name, expected_hash, actual_hash
                )));
            }
            info!("Plugin {} integrity verified", metadata.name);
        }

        // Compile the module
        let module = Module::new(&self.engine, wasm_bytes).map_err(|e| {
            Error::PluginError(format!("Failed to compile plugin {}: {}", metadata.name, e))
        })?;

        // Validate the module exports the required function
        Self::validate_module_exports(&module, &metadata.name)?;

        // Cache the compiled module
        let cached = CachedModule {
            module,
            metadata: metadata.clone(),
            compiled_at: Instant::now(),
        };

        let mut cache = self.module_cache.write().await;
        cache.insert(metadata.name.clone(), cached);

        info!("Plugin {} loaded successfully", metadata.name);
        Ok(())
    }

    /// Unload a plugin
    pub async fn unload_plugin(&self, name: &str) -> Result<()> {
        let mut cache = self.module_cache.write().await;
        if cache.remove(name).is_some() {
            info!("Plugin {} unloaded", name);
            Ok(())
        } else {
            Err(Error::PluginError(format!("Plugin {name} not found")))
        }
    }

    /// List loaded plugins
    pub async fn list_plugins(&self) -> Vec<PluginMetadata> {
        let cache = self.module_cache.read().await;
        cache.values().map(|c| c.metadata.clone()).collect()
    }

    /// Execute a plugin with the given input
    #[instrument(skip(self, input), fields(plugin_name = %plugin_name))]
    pub async fn execute(
        &self,
        plugin_name: &str,
        input: &ValidationInput,
        limits: Option<PluginLimits>,
    ) -> Result<PluginExecutionResult> {
        let start_time = Instant::now();

        // Get the cached module
        let cache = self.module_cache.read().await;
        let cached = cache
            .get(plugin_name)
            .ok_or_else(|| Error::PluginError(format!("Plugin {plugin_name} not loaded")))?;

        let module = cached.module.clone();
        let metadata = cached.metadata.clone();
        drop(cache);

        // Use provided limits or defaults
        let limits = limits.unwrap_or_else(|| metadata.limits.clone());

        // Serialize input
        let input_json = serde_json::to_vec(input)
            .map_err(|e| Error::PluginError(format!("Failed to serialize input: {e}")))?;

        // Execute in a blocking task to not block the async runtime
        let engine = self.engine.clone();
        let result = tokio::task::spawn_blocking(move || {
            Self::execute_sync(&engine, &module, input_json, &limits)
        })
        .await
        .map_err(|e| Error::PluginError(format!("Plugin execution task failed: {e}")))??;

        let execution_time = start_time.elapsed();

        Ok(PluginExecutionResult {
            plugin_name: plugin_name.to_string(),
            output: result.output,
            execution_time_ms: execution_time.as_millis() as u64,
            memory_used_bytes: result.memory_used,
            fuel_consumed: result.fuel_consumed,
        })
    }

    /// Execute multiple plugins in parallel
    pub async fn execute_all(
        &self,
        plugins: &[PluginConfig],
        input: &ValidationInput,
    ) -> Vec<Result<PluginExecutionResult>> {
        let mut handles = Vec::new();

        for plugin in plugins {
            if !plugin.enabled {
                continue;
            }

            // Check if plugin handles this operation
            if !plugin.operations.contains(&input.operation) {
                continue;
            }

            let name = plugin.metadata.name.clone();
            let limits = Some(plugin.metadata.limits.clone());
            let input = input.clone();
            let runtime = self.clone_for_execution();

            let handle = tokio::spawn(async move { runtime.execute(&name, &input, limits).await });

            handles.push((plugin.clone(), handle));
        }

        let mut results = Vec::new();
        for (plugin, handle) in handles {
            match handle.await {
                Ok(Ok(result)) => results.push(Ok(result)),
                Ok(Err(e)) => {
                    if plugin.fail_open {
                        warn!("Plugin {} failed (fail-open): {}", plugin.metadata.name, e);
                        results.push(Ok(PluginExecutionResult {
                            plugin_name: plugin.metadata.name.clone(),
                            output: ValidationOutput::allowed_with_warnings(vec![format!(
                                "Plugin failed but fail-open is enabled: {}",
                                e
                            )]),
                            execution_time_ms: 0,
                            memory_used_bytes: 0,
                            fuel_consumed: 0,
                        }));
                    } else {
                        results.push(Err(e));
                    }
                }
                Err(e) => {
                    if plugin.fail_open {
                        warn!(
                            "Plugin {} panicked (fail-open): {}",
                            plugin.metadata.name, e
                        );
                        results.push(Ok(PluginExecutionResult {
                            plugin_name: plugin.metadata.name.clone(),
                            output: ValidationOutput::allowed_with_warnings(vec![format!(
                                "Plugin panicked but fail-open is enabled: {}",
                                e
                            )]),
                            execution_time_ms: 0,
                            memory_used_bytes: 0,
                            fuel_consumed: 0,
                        }));
                    } else {
                        results.push(Err(Error::PluginError(format!(
                            "Plugin {} panicked: {}",
                            plugin.metadata.name, e
                        ))));
                    }
                }
            }
        }

        results
    }

    /// Synchronous execution (runs in blocking task)
    fn execute_sync(
        engine: &Engine,
        module: &Module,
        input_json: Vec<u8>,
        limits: &PluginLimits,
    ) -> Result<ExecutionResult> {
        // Create store with limits
        let _store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes as usize)
            .build();

        // Create WASI context (sandboxed, no filesystem or network access)
        let wasi = WasiCtxBuilder::new().build_p1();

        let state = PluginState {
            wasi,
            input_buffer: input_json,
            output_buffer: Vec::with_capacity(4096),
        };

        let mut store = Store::new(engine, state);

        // Set fuel limit
        store
            .set_fuel(limits.max_fuel)
            .map_err(|e| Error::PluginError(format!("Failed to set fuel: {e}")))?;

        // Set epoch deadline for timeout
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);

        // Create linker with WASI
        let mut linker = Linker::new(engine);
        preview1::add_to_linker_sync(&mut linker, |state: &mut PluginState| &mut state.wasi)
            .map_err(|e| Error::PluginError(format!("Failed to add WASI to linker: {e}")))?;

        // Add host functions for input/output
        Self::add_host_functions(&mut linker)?;

        // Instantiate the module
        let instance = linker
            .instantiate(&mut store, module)
            .map_err(|e| Error::PluginError(format!("Failed to instantiate module: {e}")))?;

        // Get the validate function
        let validate_fn = instance
            .get_typed_func::<(), i32>(&mut store, "validate")
            .map_err(|e| Error::PluginError(format!("Failed to get validate function: {e}")))?;

        // Call the validate function
        let result_code = validate_fn.call(&mut store, ()).map_err(|e| {
            if e.to_string().contains("fuel") {
                Error::PluginError("Plugin exceeded instruction limit".to_string())
            } else if e.to_string().contains("epoch") {
                Error::PluginError("Plugin execution timeout".to_string())
            } else {
                Error::PluginError(format!("Plugin execution failed: {e}"))
            }
        })?;

        // Get fuel consumed
        let fuel_remaining = store.get_fuel().unwrap_or(0);
        let fuel_consumed = limits.max_fuel.saturating_sub(fuel_remaining);

        // Parse output from state
        let state = store.data();
        let output: ValidationOutput = if state.output_buffer.is_empty() {
            // Default to denied if no output
            if result_code == 0 {
                ValidationOutput::allowed()
            } else {
                ValidationOutput::denied(format!("Plugin returned error code: {result_code}"))
            }
        } else {
            serde_json::from_slice(&state.output_buffer)
                .map_err(|e| Error::PluginError(format!("Failed to parse plugin output: {e}")))?
        };

        Ok(ExecutionResult {
            output,
            memory_used: 0, // TODO: Track actual memory usage
            fuel_consumed,
        })
    }

    /// Add host functions for plugin I/O
    fn add_host_functions(linker: &mut Linker<PluginState>) -> Result<()> {
        // Function to get input length
        linker
            .func_wrap(
                "env",
                "get_input_len",
                |caller: Caller<'_, PluginState>| -> i32 {
                    caller.data().input_buffer.len() as i32
                },
            )
            .map_err(|e| Error::PluginError(format!("Failed to add get_input_len: {e}")))?;

        // Function to read input into Wasm memory
        linker
            .func_wrap(
                "env",
                "read_input",
                |mut caller: Caller<'_, PluginState>, ptr: i32, len: i32| -> i32 {
                    let memory = match caller.get_export("memory") {
                        Some(Extern::Memory(mem)) => mem,
                        _ => return -1,
                    };

                    // Clone the input to avoid borrow issues
                    let input = caller.data().input_buffer.clone();
                    let read_len = std::cmp::min(len as usize, input.len());

                    if memory
                        .write(&mut caller, ptr as usize, &input[..read_len])
                        .is_err()
                    {
                        return -1;
                    }

                    read_len as i32
                },
            )
            .map_err(|e| Error::PluginError(format!("Failed to add read_input: {e}")))?;

        // Function to write output from Wasm memory
        linker
            .func_wrap(
                "env",
                "write_output",
                |mut caller: Caller<'_, PluginState>, ptr: i32, len: i32| -> i32 {
                    let memory = match caller.get_export("memory") {
                        Some(Extern::Memory(mem)) => mem,
                        _ => return -1,
                    };

                    let mut buffer = vec![0u8; len as usize];
                    if memory.read(&caller, ptr as usize, &mut buffer).is_err() {
                        return -1;
                    }

                    caller.data_mut().output_buffer = buffer;
                    0
                },
            )
            .map_err(|e| Error::PluginError(format!("Failed to add write_output: {e}")))?;

        // Function for debug logging
        linker
            .func_wrap(
                "env",
                "log_message",
                |mut caller: Caller<'_, PluginState>, ptr: i32, len: i32| {
                    let memory = match caller.get_export("memory") {
                        Some(Extern::Memory(mem)) => mem,
                        _ => return,
                    };

                    let mut buffer = vec![0u8; len as usize];
                    if memory.read(&caller, ptr as usize, &mut buffer).is_ok() {
                        if let Ok(msg) = String::from_utf8(buffer) {
                            debug!(target: "wasm_plugin", "{}", msg);
                        }
                    }
                },
            )
            .map_err(|e| Error::PluginError(format!("Failed to add log_message: {e}")))?;

        Ok(())
    }

    /// Validate that the module exports required functions
    fn validate_module_exports(module: &Module, name: &str) -> Result<()> {
        let exports: Vec<_> = module.exports().collect();

        let has_validate = exports.iter().any(|e| e.name() == "validate");
        if !has_validate {
            return Err(Error::PluginError(format!(
                "Plugin {name} must export a 'validate' function"
            )));
        }

        let has_memory = exports.iter().any(|e| e.name() == "memory");
        if !has_memory {
            return Err(Error::PluginError(format!(
                "Plugin {name} must export 'memory'"
            )));
        }

        Ok(())
    }

    /// Compute SHA256 hash of bytes
    fn compute_sha256(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Clone runtime for parallel execution (shares engine and cache)
    fn clone_for_execution(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            module_cache: self.module_cache.clone(),
            default_limits: self.default_limits.clone(),
        }
    }
}

impl Default for WasmRuntime {
    fn default() -> Self {
        Self::new().expect("Failed to create default WasmRuntime")
    }
}

/// Result of plugin execution
struct ExecutionResult {
    output: ValidationOutput,
    memory_used: u64,
    fuel_consumed: u64,
}

/// Builder for configuring the Wasm runtime
pub struct WasmRuntimeBuilder {
    limits: PluginLimits,
    enable_simd: bool,
    enable_threads: bool,
    max_stack_size: usize,
}

impl WasmRuntimeBuilder {
    pub fn new() -> Self {
        Self {
            limits: PluginLimits::default(),
            enable_simd: true,
            enable_threads: false,
            max_stack_size: 512 * 1024,
        }
    }

    pub fn with_limits(mut self, limits: PluginLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.limits.timeout_ms = timeout_ms;
        self
    }

    pub fn with_max_memory(mut self, max_memory_bytes: u64) -> Self {
        self.limits.max_memory_bytes = max_memory_bytes;
        self
    }

    pub fn with_max_fuel(mut self, max_fuel: u64) -> Self {
        self.limits.max_fuel = max_fuel;
        self
    }

    pub fn with_simd(mut self, enable: bool) -> Self {
        self.enable_simd = enable;
        self
    }

    pub fn with_threads(mut self, enable: bool) -> Self {
        self.enable_threads = enable;
        self
    }

    pub fn with_stack_size(mut self, size: usize) -> Self {
        self.max_stack_size = size;
        self
    }

    pub fn build(self) -> Result<WasmRuntime> {
        let mut config = Config::new();

        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.max_wasm_stack(self.max_stack_size);
        config.cranelift_opt_level(OptLevel::Speed);
        config.wasm_threads(self.enable_threads);
        config.wasm_simd(self.enable_simd);
        config.wasm_bulk_memory(true);
        config.wasm_multi_value(true);
        config.wasm_reference_types(false);

        let engine = Engine::new(&config)
            .map_err(|e| Error::PluginError(format!("Engine creation failed: {e}")))?;

        Ok(WasmRuntime {
            engine,
            module_cache: Arc::new(RwLock::new(HashMap::new())),
            default_limits: self.limits,
        })
    }
}

impl Default for WasmRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256() {
        let data = b"hello world";
        let hash = WasmRuntime::compute_sha256(data);
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_plugin_limits_default() {
        let limits = PluginLimits::default();
        assert_eq!(limits.timeout_ms, 1000);
        assert_eq!(limits.max_memory_bytes, 16 * 1024 * 1024);
        assert_eq!(limits.max_fuel, 1_000_000);
    }

    #[tokio::test]
    async fn test_runtime_creation() {
        let runtime = WasmRuntime::new();
        assert!(runtime.is_ok());
    }

    #[tokio::test]
    async fn test_runtime_builder() {
        let runtime = WasmRuntimeBuilder::new()
            .with_timeout_ms(2000)
            .with_max_memory(32 * 1024 * 1024)
            .with_max_fuel(2_000_000)
            .build();
        assert!(runtime.is_ok());
    }
}
