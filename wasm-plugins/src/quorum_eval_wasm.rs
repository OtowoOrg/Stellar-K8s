//! Wasm ABI for Quorum Set Validation
//!
//! Provides extern C functions for validating quorum sets from Wasm hosts.
//! All functions use simple memory semantics to work across language boundaries.
//!
//! Return values:
//! - Positive: Success (size of output data)
//! - Zero: Validation failed, use error buffer for details
//! - Negative: Fatal error, host should bypass validation

#![cfg(target_arch = "wasm32")]

use std::sync::{Mutex, OnceLock};

use crate::quorum_eval::{QuorumSetConfig, QuorumValidator, ValidationPolicy};

static VALIDATOR: OnceLock<Mutex<Option<QuorumValidator>>> = OnceLock::new();

fn validator() -> &'static Mutex<Option<QuorumValidator>> {
    VALIDATOR.get_or_init(|| {
        let validator = QuorumValidator::new(ValidationPolicy::default());
        Mutex::new(Some(validator))
    })
}

/// Initialize the quorum validator with a custom policy
///
/// # Arguments
/// - `policy_ptr`: Pointer to serialized ValidationPolicy JSON (UTF-8)
/// - `policy_len`: Length of policy data
///
/// # Returns
/// - 0 on success
/// - -1 on invalid policy or initialization error
#[no_mangle]
pub unsafe extern "C" fn quorum_validator_init(policy_ptr: *const u8, policy_len: usize) -> i32 {
    if policy_ptr.is_null() || policy_len == 0 {
        // Initialize with default policy
        let new_validator = QuorumValidator::new(ValidationPolicy::default());
        if let Ok(mut slot) = validator().lock() {
            *slot = Some(new_validator);
            return 0;
        }
        return -1;
    }

    let policy_data = std::slice::from_raw_parts(policy_ptr, policy_len);
    let Ok(policy_json) = std::str::from_utf8(policy_data) else {
        return -1;
    };

    let Ok(policy) = serde_json::from_str::<ValidationPolicy>(policy_json) else {
        return -1;
    };

    let new_validator = QuorumValidator::new(policy);
    if let Ok(mut slot) = validator().lock() {
        *slot = Some(new_validator);
        return 0;
    }
    -1
}

/// Validate a quorum set configuration
///
/// # Arguments
/// - `quorum_ptr`: Pointer to serialized QuorumSetConfig JSON (UTF-8)
/// - `quorum_len`: Length of quorum data
/// - `result_ptr`: Pointer to output buffer for serialized ValidationResult
/// - `result_capacity`: Capacity of output buffer
///
/// # Returns
/// - Positive: Number of bytes written to result buffer
/// - 0: Validation failed (data written to result buffer)
/// - -1: Fatal error (host should bypass validation)
#[no_mangle]
pub unsafe extern "C" fn quorum_validate(
    quorum_ptr: *const u8,
    quorum_len: usize,
    result_ptr: *mut u8,
    result_capacity: usize,
) -> i32 {
    if quorum_ptr.is_null() || result_ptr.is_null() {
        return -1;
    }

    if quorum_len == 0 || result_capacity == 0 {
        return -1;
    }

    // Parse quorum set configuration
    let quorum_data = std::slice::from_raw_parts(quorum_ptr, quorum_len);
    let Ok(quorum_json) = std::str::from_utf8(quorum_data) else {
        return -1;
    };

    let Ok(quorum) = serde_json::from_str::<QuorumSetConfig>(quorum_json) else {
        return -1;
    };

    // Get validator and run validation
    let Ok(slot) = validator().lock() else {
        return -1;
    };

    let Some(ref validator) = *slot else {
        return -1;
    };

    let result = validator.validate(&quorum);

    // Serialize result to JSON
    let Ok(result_json) = serde_json::to_string(&result) else {
        return -1;
    };

    if result_json.len() > result_capacity {
        return -1; // Buffer too small
    }

    // Copy result to output buffer
    let result_bytes = result_json.as_bytes();
    std::ptr::copy_nonoverlapping(result_bytes.as_ptr(), result_ptr, result_bytes.len());

    if result.is_valid {
        result_bytes.len() as i32
    } else {
        0 // Validation failed, but we provided error details
    }
}

/// Validate a quorum set and return just the is_valid flag
///
/// # Arguments
/// - `quorum_ptr`: Pointer to serialized QuorumSetConfig JSON (UTF-8)
/// - `quorum_len`: Length of quorum data
///
/// # Returns
/// - 1 if valid
/// - 0 if invalid
/// - -1 on fatal error
#[no_mangle]
pub unsafe extern "C" fn quorum_is_valid(quorum_ptr: *const u8, quorum_len: usize) -> i32 {
    if quorum_ptr.is_null() {
        return -1;
    }

    let quorum_data = std::slice::from_raw_parts(quorum_ptr, quorum_len);
    let Ok(quorum_json) = std::str::from_utf8(quorum_data) else {
        return -1;
    };

    let Ok(quorum) = serde_json::from_str::<QuorumSetConfig>(quorum_json) else {
        return -1;
    };

    let Ok(slot) = validator().lock() else {
        return -1;
    };

    let Some(ref validator) = *slot else {
        return -1;
    };

    let result = validator.validate(&quorum);
    if result.is_valid {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_validator_init_default() {
        unsafe {
            let result = quorum_validator_init(std::ptr::null(), 0);
            assert_eq!(result, 0, "Failed to initialize with default policy");
        }
    }

    #[test]
    fn test_wasm_validator_init_custom_policy() {
        unsafe {
            let policy_json = r#"{"max_centralization_pct":60,"check_transitive_depth":true,"check_cycles":true,"check_intersection":true,"max_transitive_depth":3,"min_intersection_ratio":0.75}"#;
            let policy_bytes = policy_json.as_bytes();
            let result = quorum_validator_init(policy_bytes.as_ptr(), policy_bytes.len());
            assert_eq!(result, 0, "Failed to initialize with custom policy");
        }
    }

    #[test]
    fn test_wasm_quorum_validation() {
        unsafe {
            // Initialize validator
            let _ = quorum_validator_init(std::ptr::null(), 0);

            let quorum_json = r#"{"t":2,"v":["v1","v2","v3"],"innerSets":[]}"#;
            let quorum_bytes = quorum_json.as_bytes();

            let mut result_buf = vec![0u8; 4096];
            let bytes_written = quorum_validate(
                quorum_bytes.as_ptr(),
                quorum_bytes.len(),
                result_buf.as_mut_ptr(),
                result_buf.len(),
            );

            assert!(
                bytes_written > 0,
                "Validation should succeed for valid quorum"
            );

            let result_str = std::str::from_utf8(&result_buf[..bytes_written as usize]).unwrap();
            let result_json: serde_json::Value = serde_json::from_str(result_str).unwrap();
            assert!(result_json["is_valid"].as_bool().unwrap());
        }
    }

    #[test]
    fn test_wasm_quorum_is_valid_true() {
        unsafe {
            // Initialize validator
            let _ = quorum_validator_init(std::ptr::null(), 0);

            let quorum_json = r#"{"t":2,"v":["v1","v2","v3"],"innerSets":[]}"#;
            let quorum_bytes = quorum_json.as_bytes();

            let result = quorum_is_valid(quorum_bytes.as_ptr(), quorum_bytes.len());
            assert_eq!(result, 1, "Valid quorum should return 1");
        }
    }

    #[test]
    fn test_wasm_quorum_is_valid_false() {
        unsafe {
            // Initialize validator
            let _ = quorum_validator_init(std::ptr::null(), 0);

            let quorum_json = r#"{"t":1,"v":["v1"],"innerSets":[]}"#;
            let quorum_bytes = quorum_json.as_bytes();

            let result = quorum_is_valid(quorum_bytes.as_ptr(), quorum_bytes.len());
            assert_eq!(result, 0, "Invalid quorum should return 0");
        }
    }

    #[test]
    fn test_wasm_invalid_quorum_json() {
        unsafe {
            // Initialize validator
            let _ = quorum_validator_init(std::ptr::null(), 0);

            let quorum_json = r#"invalid json"#;
            let quorum_bytes = quorum_json.as_bytes();

            let result = quorum_is_valid(quorum_bytes.as_ptr(), quorum_bytes.len());
            assert_eq!(result, -1, "Invalid JSON should return -1");
        }
    }

    #[test]
    fn test_wasm_null_pointers() {
        unsafe {
            let result = quorum_is_valid(std::ptr::null(), 10);
            assert_eq!(result, -1, "Null pointer should return -1");
        }
    }
}
