//! Fail-Open Wasm Caching Plugin (partial implementation of issue #4)
//!
//! Wraps [`cache::LruTtlCache`] behind the same host-function ABI used by
//! the existing Custom Validation Plugin system (see
//! `examples/plugins/example-validator`): the host writes a JSON request
//! into the plugin's memory, calls an exported function, and reads a JSON
//! response back out.
//!
//! **What this demonstrates:** the cache engine (LRU eviction, TTL expiry,
//! bounded capacity) running for real inside a Wasm sandbox, reached
//! through the actual `read_input`/`write_output` host-function boundary,
//! with a genuine Rust panic inside a cache operation caught and turned
//! into a "bypass" response instead of trapping the whole call.
//!
//! **What this does not demonstrate:** state persisting *across* separate
//! admission-webhook invocations. `WasmRuntime::execute_sync` (see
//! `src/webhook/runtime.rs`) creates a brand-new `Store` and instantiates
//! the module fresh for every single call, so an in-Wasm cache has no
//! memory to persist between one RPC read and the next under that
//! execution model as it exists today. This plugin's request format
//! therefore accepts a *batch* of operations replayed against one
//! freshly-built cache within a single call, which is the most this
//! architecture can prove without also changing `WasmRuntime` to keep a
//! `Store`/`Instance` alive across calls (out of scope for this slice —
//! see the crate README).
//! Build with: `cargo build --target wasm32-wasip1 --release`

// Outside the wasm32 build, the ABI layer below (which is the only
// production caller of `cache`'s public API and of `handle_request`) is
// entirely `#[cfg]`'d out so `cargo test` can run natively without a Wasm
// host, which is what makes these `dead_code` lints fire on that build —
// the code is live on the target that actually ships.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

mod cache;

use std::panic::{self, AssertUnwindSafe};

use serde::{Deserialize, Serialize};

// Host functions provided by the runtime (same ABI as example-validator).
// Only declared/linked for the actual Wasm target: the host only ever
// supplies these to a Wasm guest, and gating them lets the portable cache
// logic below (and its unit tests) build and run on the native host
// target with plain `cargo test`, with no Wasm runtime involved at all.
#[cfg(target_arch = "wasm32")]
extern "C" {
    fn get_input_len() -> i32;
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    fn write_output(ptr: *const u8, len: i32) -> i32;
    fn log_message(ptr: *const u8, len: i32);
}

#[derive(Debug, Deserialize)]
struct CacheRequest {
    #[serde(default)]
    config: RequestConfig,
    ops: Vec<CacheOp>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestConfig {
    #[serde(default = "default_capacity")]
    capacity: usize,
    #[serde(default = "default_ttl")]
    ttl_seconds: u64,
}

fn default_capacity() -> usize {
    CacheConfig::default().capacity
}

fn default_ttl() -> u64 {
    CacheConfig::default().ttl_seconds
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            capacity: default_capacity(),
            ttl_seconds: default_ttl(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum CacheOp {

    /// Test-only op: deliberately panics inside the cache call path, to
    /// prove the fail-open wrapper survives a real panic rather than only
    /// a modeled error path.
    InjectPanic,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum OpOutcome {
    Hit { value: String },
    Miss,
    Ok,
    /// The cache path failed (panicked, or was disabled) and the caller
    /// should treat this as a cache miss and go straight to the RPC node.
    Bypass { reason: String },
}

#[derive(Debug, Serialize)]
struct CacheResponse {
    results: Vec<OpOutcome>,
    /// True if any op in this batch had to fail open.
    any_bypassed: bool,
}

#[cfg(target_arch = "wasm32")]
fn log(msg: &str) {
    unsafe {
        log_message(msg.as_ptr(), msg.len() as i32);
    }
}

/// Native stand-in for [`log`] so the fail-open logging calls in
/// `handle_request`/`run_op_fail_open` remain exercised by `cargo test`
/// on the host target, without needing the host's `log_message` import.
#[cfg(not(target_arch = "wasm32"))]
fn log(msg: &str) {
    eprintln!("[cache-plugin] {msg}");
}

#[cfg(target_arch = "wasm32")]
fn read_request() -> Result<CacheRequest, String> {
    let len = unsafe { get_input_len() };
    if len <= 0 {
        return Err("No input provided".to_string());
    }
    let mut buffer = vec![0u8; len as usize];
    let read = unsafe { read_input(buffer.as_mut_ptr(), len) };
    if read < 0 {
        return Err("Failed to read input".to_string());
    }

}

#[cfg(target_arch = "wasm32")]
fn write_response(response: &CacheResponse) -> Result<(), String> {

    let result = unsafe { write_output(json.as_ptr(), json.len() as i32) };
    if result < 0 {
        return Err("Failed to write output".to_string());
    }
    Ok(())
}

/// Run one op against `store`, catching a panic and turning it into a
/// [`OpOutcome::Bypass`] instead of letting it unwind out of this
/// function (and, in a real Wasmtime host, trap the whole call).
fn run_op_fail_open(store: &mut LruTtlCache<String, String>, op: CacheOp) -> OpOutcome {
    let result = panic::catch_unwind(AssertUnwindSafe(|| match op {
        CacheOp::Get { key, now } => match store.get(&key, now) {
            Lookup::Hit(value) => OpOutcome::Hit { value },
            Lookup::Miss => OpOutcome::Miss,
        },
        CacheOp::Put { key, value, now } => {
            store.put(key, value, now);
            OpOutcome::Ok
        }
        CacheOp::InjectPanic => panic!("cache-plugin: injected panic for fail-open test"),
    }));

    match result {
        Ok(outcome) => outcome,
        Err(_) => {
            log("cache op panicked; failing open");
            OpOutcome::Bypass {
                reason: "cache_panic".to_string(),
            }
        }
    }
}

fn handle_request(request: CacheRequest) -> CacheResponse {
    let config = CacheConfig {
        capacity: request.config.capacity,
        ttl_seconds: request.config.ttl_seconds,
    };

    // Building the cache itself is wrapped too: a pathological config
    // (e.g. one that somehow makes allocation fail) must not crash the
    // whole request either. `LruTtlCache::new` also hard-clamps capacity
    // to `MAX_CAPACITY`, which is the primary defense here.
    let built = panic::catch_unwind(AssertUnwindSafe(|| LruTtlCache::new(config)));

    let mut store = match built {
        Ok(store) => store,
        Err(_) => {
            log("cache construction panicked; every op in this batch fails open");
            let n = request.ops.len();
            return CacheResponse {
                results: (0..n)
                    .map(|_| OpOutcome::Bypass {
                        reason: "cache_init_panic".to_string(),
                    })
                    .collect(),
                any_bypassed: n > 0,
            };
        }
    };

    let mut any_bypassed = false;
    let results = request
        .ops
        .into_iter()
        .map(|op| {
            let outcome = run_op_fail_open(&mut store, op);
            if matches!(outcome, OpOutcome::Bypass { .. }) {
                any_bypassed = true;
            }
            outcome
        })
        .collect();


}

/// Entry point called by the Wasm runtime.
///
/// Returns `0` on success (including when individual ops failed open —
/// that is the *expected* graceful path, not an error) and non-zero only
/// when the request itself couldn't be read or the response couldn't be
/// written at all.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn cache_batch() -> i32 {
    let request = match read_request() {
        Ok(request) => request,
        Err(e) => {
            log(&format!("Error reading input: {e}"));
            let _ = write_response(&CacheResponse {
                results: vec![],
                any_bypassed: true,
            });
            return 1;
        }
    };

    let response = handle_request(request);

    if let Err(e) = write_response(&response) {
        log(&format!("Error writing output: {e}"));
        return 2;
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_of_put_then_get_hits() {
        let request = CacheRequest {
            config: RequestConfig::default(),
            ops: vec![
                CacheOp::Put {
                    key: "a".into(),
                    value: "1".into(),
                    now: 0,
                },
                CacheOp::Get {
                    key: "a".into(),
                    now: 1,
                },
            ],
        };
        let response = handle_request(request);
        assert!(!response.any_bypassed);
        assert!(matches!(response.results[0], OpOutcome::Ok));
        assert!(matches!(&response.results[1], OpOutcome::Hit { value } if value == "1"));
    }

    #[test]
    fn get_on_empty_cache_is_a_miss_not_a_bypass() {
        let request = CacheRequest {
            config: RequestConfig::default(),
            ops: vec![CacheOp::Get {
                key: "missing".into(),
                now: 0,
            }],
        };
        let response = handle_request(request);
        assert!(!response.any_bypassed);
        assert!(matches!(response.results[0], OpOutcome::Miss));
    }

    #[test]
    fn zero_capacity_config_fails_open_as_miss_not_a_crash() {
        let request = CacheRequest {
            config: RequestConfig {
                capacity: 0,
                ttl_seconds: 30,
            },
            ops: vec![
                CacheOp::Put {
                    key: "a".into(),
                    value: "1".into(),
                    now: 0,
                },
                CacheOp::Get {
                    key: "a".into(),
                    now: 1,
                },
            ],
        };
        let response = handle_request(request);
        assert!(matches!(response.results[0], OpOutcome::Ok));
        assert!(matches!(response.results[1], OpOutcome::Miss));
    }

    #[test]
    fn a_real_panic_inside_the_cache_path_fails_open_instead_of_unwinding_out() {
        let request = CacheRequest {
            config: RequestConfig::default(),
            ops: vec![
                CacheOp::Put {
                    key: "a".into(),
                    value: "1".into(),
                    now: 0,
                },
                CacheOp::InjectPanic,
                // The batch keeps going after the panic: one bad op fails
                // open, it does not take down every subsequent op.
                CacheOp::Get {
                    key: "a".into(),
                    now: 1,
                },
            ],
        };

        // Silence the default panic hook's stderr output for this
        // expected, caught panic so `cargo test` output stays readable.
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));
        let response = handle_request(request);
        panic::set_hook(prev_hook);

        assert!(response.any_bypassed);
        assert!(matches!(response.results[0], OpOutcome::Ok));
        assert!(matches!(&response.results[1], OpOutcome::Bypass { .. }));
        // Confirms the cache instance itself survived the panic and kept
        // serving subsequent ops in the same batch correctly.
        assert!(matches!(&response.results[2], OpOutcome::Hit { value } if value == "1"));
    }
}
