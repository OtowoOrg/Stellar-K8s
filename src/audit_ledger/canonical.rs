// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Deterministic JSON serialization used as the hash pre-image.
//!
//! Rules (stable for ledger format v1):
//!
//! * No insignificant whitespace.
//! * Object members are sorted by the raw UTF-8 bytes of their keys.
//! * Arrays keep their order.
//! * Numbers must be integers representable as `i64`/`u64`; floating point
//!   numbers are rejected because their text form is not portable.
//! * Strings escape only `"` `\` and control characters: `\b \t \n \f \r` use
//!   short forms, all other C0 controls use `\u00xx` (lowercase hex). All
//!   other characters, including non-ASCII, are emitted as raw UTF-8.

use serde_json::Value;

/// Error raised when a value cannot be canonicalized.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CanonicalError {
    #[error("non-integer number {0} is not allowed in audited events")]
    NonIntegerNumber(String),
}

/// Canonical byte representation of `value`.
pub fn to_vec(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::with_capacity(256);
    write(value, &mut out)?;
    Ok(out)
}

fn write(value: &Value, out: &mut Vec<u8>) -> Result<(), CanonicalError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.extend_from_slice(i.to_string().as_bytes());
            } else if let Some(u) = n.as_u64() {
                out.extend_from_slice(u.to_string().as_bytes());
            } else {
                return Err(CanonicalError::NonIntegerNumber(n.to_string()));
            }
        }
        Value::String(s) => write_str(s, out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push(b'{');
            for (i, (k, v)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_str(k, out);
                out.push(b':');
                write(v, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn write_str(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for ch in s.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\r' => out.extend_from_slice(b"\\r"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes())
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(v: Value) -> String {
        String::from_utf8(to_vec(&v).unwrap()).unwrap()
    }

    #[test]
    fn sorts_keys_and_strips_whitespace() {
        let a: Value =
            serde_json::from_str(r#"{ "b": [1, 2], "a": {"z": null, "y": true} }"#).unwrap();
        assert_eq!(c(a), r#"{"a":{"y":true,"z":null},"b":[1,2]}"#);
    }

    #[test]
    fn equal_values_with_different_source_order_hash_identically() {
        let a: Value = serde_json::from_str(r#"{"x":1,"y":2}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"y":2,"x":1}"#).unwrap();
        assert_eq!(to_vec(&a).unwrap(), to_vec(&b).unwrap());
    }

    #[test]
    fn escapes_controls_and_keeps_utf8() {
        assert_eq!(c(json!("a\"\\\n\u{1}é😀")), "\"a\\\"\\\\\\n\\u0001é😀\"");
    }

    #[test]
    fn rejects_floats_but_accepts_big_ints() {
        assert!(to_vec(&json!(1.5)).is_err());
        assert_eq!(c(json!(u64::MAX)), "18446744073709551615");
        assert_eq!(c(json!(-5)), "-5");
    }

    #[test]
    fn array_order_is_preserved() {
        assert_eq!(c(json!([3, 1, 2])), "[3,1,2]");
    }
}
