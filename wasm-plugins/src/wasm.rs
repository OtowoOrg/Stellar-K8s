//! Minimal Wasm ABI for hosts that want to embed the cache directly.
//!
//! Return values are deliberately simple: positive values indicate success,
//! zero means a miss, and negative values mean the host should bypass the cache.

#![cfg(target_arch = "wasm32")]

use std::sync::{Mutex, OnceLock};

use crate::{CacheConfig, StateCache, MAX_CACHE_BYTES};

static CACHE: OnceLock<Mutex<Option<StateCache>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<StateCache>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Initialise or replace the bounded cache. Returns 0 on success, -1 on invalid config.
#[no_mangle]
pub extern "C" fn cache_init(ttl_secs: u64, max_entries: usize, max_bytes: usize) -> i32 {
    let config = CacheConfig {
        ttl_secs,
        max_entries,
        max_bytes,
    };
    let Ok(new_cache) = StateCache::new(config) else {
        return -1;
    };
    let Ok(mut slot) = cache().lock() else {
        return -1;
    };
    *slot = Some(new_cache);
    0
}

/// Look up a serialized response by key and copy it into the host-provided output buffer.
/// Returns the number of bytes copied, 0 for a miss, or -1 when the cache is unavailable.
#[no_mangle]
pub unsafe extern "C" fn cache_lookup(
    key_ptr: *const u8,
    key_len: usize,
    output_ptr: *mut u8,
    output_capacity: usize,
) -> i32 {
    if key_ptr.is_null() || output_ptr.is_null() {
        return -1;
    }
    let key = std::slice::from_raw_parts(key_ptr, key_len);
    let Ok(key) = std::str::from_utf8(key) else {
        return -1;
    };
    let Ok(mut slot) = cache().lock() else {
        return -1;
    };
    let Some(cache) = slot.as_mut() else {
        return -1;
    };
    let Some(value) = cache.get(key) else {
        return 0;
    };
    if value.len() > output_capacity || value.len() > i32::MAX as usize {
        return -1;
    }
    std::ptr::copy_nonoverlapping(value.as_ptr(), output_ptr, value.len());
    value.len() as i32
}

/// Store a serialized response. Returns 0 on success or -1 when the host should bypass.
#[no_mangle]
pub unsafe extern "C" fn cache_store(
    key_ptr: *const u8,
    key_len: usize,
    value_ptr: *const u8,
    value_len: usize,
) -> i32 {
    if key_ptr.is_null() || value_ptr.is_null() {
        return -1;
    }
    if key_len.saturating_add(value_len) > MAX_CACHE_BYTES {
        return -1;
    }
    let key = std::slice::from_raw_parts(key_ptr, key_len);
    let value = std::slice::from_raw_parts(value_ptr, value_len);
    let Ok(key) = std::str::from_utf8(key) else {
        return -1;
    };
    let Ok(mut slot) = cache().lock() else {
        return -1;
    };
    let Some(cache) = slot.as_mut() else {
        return -1;
    };
    let mut copied_key = String::new();
    if copied_key.try_reserve(key.len()).is_err() {
        return -1;
    }
    copied_key.push_str(key);

    let mut copied = Vec::new();
    if copied.try_reserve(value_len).is_err() {
        return -1;
    }
    copied.extend_from_slice(value);
    cache.insert(copied_key, copied).map(|_| 0).unwrap_or(-1)
}
