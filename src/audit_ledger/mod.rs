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

//! # Immutable audit event pipeline with tamper-evident chain hashing (#1958)
//!
//! This module turns mutating Kubernetes API audit events into an append-only,
//! SHA-256 hash-chained ledger that can be verified offline by a single binary
//! (`stellar-audit-verify`) without trusting the producer.
//!
//! ```text
//! kube-apiserver --(audit webhook backend, batch mode)--> stellar-audit-sink
//!                                                            │ validate / filter / dedupe
//!                                                            │ assign seq, chain hash
//!                                                            ▼
//!                                              append-only ledger (NDJSON, fsync before ack)
//!                                                            │
//!                       ┌────────────────────────────────────┴───────────────┐
//!                       ▼                                                    ▼
//!              ChainMonitor (gap alert < 60s)                  stellar-audit-verify (offline)
//! ```
//!
//! ## Hash definition (normative, version 1)
//!
//! All hashes are SHA-256. `||` is byte concatenation, `be64` is an 8-byte
//! big-endian integer.
//!
//! ```text
//! genesis      = SHA256("stellar-k8s/audit-ledger/v1/genesis\0" || chain_id_utf8)
//! content_hash = SHA256("stellar-k8s/audit-ledger/v1/content\0" || canonical_json(event))
//! entry_hash   = SHA256("stellar-k8s/audit-ledger/v1/entry\0"   || be64(seq) || prev_hash || content_hash)
//! ```
//!
//! * The first entry has `seq = 0` and `prev_hash = genesis`.
//! * Every following entry has `seq = previous.seq + 1` and
//!   `prev_hash = previous.entry_hash`.
//! * `canonical_json` is defined in [`canonical`].
//!
//! Splitting `content_hash` from `entry_hash` lets a verifier hash event bodies
//! on many cores and run only the cheap 104-byte link hash sequentially.
//!
//! ## Archive format
//!
//! UTF-8 NDJSON, one JSON object per line:
//!
//! ```text
//! {"type":"header","format":"stellar-audit-ledger/v1","chainId":"prod","genesisHash":"<hex>"}
//! {"seq":0,"prevHash":"<genesis>","hash":"<hex>","event":{...}}
//! {"seq":1,"prevHash":"<hash of 0>","hash":"<hex>","event":{...}}
//! {"type":"footer","count":2,"headSeq":1,"headHash":"<hex>"}      (optional, written by `export`)
//! ```
//!
//! Ordering is defined solely by `seq`; timestamps inside the event are
//! informational and never used for ordering.

pub mod canonical;
pub mod monitor;
pub mod sink;
pub mod store;
pub mod verify;

use sha2::{Digest, Sha256};

/// Archive format identifier written to (and required in) the header line.
pub const FORMAT: &str = "stellar-audit-ledger/v1";

const GENESIS_DOMAIN: &[u8] = b"stellar-k8s/audit-ledger/v1/genesis\0";
const CONTENT_DOMAIN: &[u8] = b"stellar-k8s/audit-ledger/v1/content\0";
const ENTRY_DOMAIN: &[u8] = b"stellar-k8s/audit-ledger/v1/entry\0";

/// A raw SHA-256 digest.
pub type Hash = [u8; 32];

/// Genesis value for a chain: the `prev_hash` of the entry with `seq = 0`.
pub fn genesis_hash(chain_id: &str) -> Hash {
    let mut h = Sha256::new();
    h.update(GENESIS_DOMAIN);
    h.update(chain_id.as_bytes());
    h.finalize().into()
}

/// Hash of the canonical event bytes.
pub fn content_hash(canonical_event: &[u8]) -> Hash {
    let mut h = Sha256::new();
    h.update(CONTENT_DOMAIN);
    h.update(canonical_event);
    h.finalize().into()
}

/// Link hash binding sequence number, predecessor and event content.
pub fn entry_hash(seq: u64, prev_hash: &Hash, content: &Hash) -> Hash {
    let mut h = Sha256::new();
    h.update(ENTRY_DOMAIN);
    h.update(seq.to_be_bytes());
    h.update(prev_hash);
    h.update(content);
    h.finalize().into()
}

/// Encode one ledger entry as a single NDJSON line (including trailing `\n`).
///
/// Returns the line and the new head hash.
pub fn encode_entry(
    seq: u64,
    prev_hash: &Hash,
    event: &serde_json::Value,
) -> Result<(Vec<u8>, Hash), canonical::CanonicalError> {
    let canon = canonical::to_vec(event)?;
    let hash = entry_hash(seq, prev_hash, &content_hash(&canon));
    let mut line = Vec::with_capacity(canon.len() + 180);
    line.extend_from_slice(b"{\"seq\":");
    line.extend_from_slice(seq.to_string().as_bytes());
    line.extend_from_slice(b",\"prevHash\":\"");
    line.extend_from_slice(hex::encode(prev_hash).as_bytes());
    line.extend_from_slice(b"\",\"hash\":\"");
    line.extend_from_slice(hex::encode(hash).as_bytes());
    line.extend_from_slice(b"\",\"event\":");
    line.extend_from_slice(&canon);
    line.extend_from_slice(b"}\n");
    Ok((line, hash))
}

/// Header line for a ledger starting at genesis.
pub fn encode_header(chain_id: &str) -> Vec<u8> {
    // Fixed key order: readers detect header/footer lines by their `{"type":` prefix.
    let chain = serde_json::to_string(chain_id).expect("string serializes");
    format!(
        "{{\"type\":\"header\",\"format\":\"{FORMAT}\",\"chainId\":{chain},\"genesisHash\":\"{}\"}}\n",
        hex::encode(genesis_hash(chain_id))
    )
    .into_bytes()
}

/// Footer line sealing `count` entries ending at `head_seq` / `head_hash`.
pub fn encode_footer(count: u64, head_seq: Option<u64>, head_hash: &str) -> Vec<u8> {
    let seq = head_seq
        .map(|s| s.to_string())
        .unwrap_or_else(|| "null".into());
    format!("{{\"type\":\"footer\",\"count\":{count},\"headSeq\":{seq},\"headHash\":\"{head_hash}\"}}\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn genesis_is_domain_separated_per_chain() {
        assert_ne!(genesis_hash("a"), genesis_hash("b"));
        assert_eq!(genesis_hash("a"), genesis_hash("a"));
    }

    /// Known-answer vector computed independently with `sha256sum`/Python so
    /// other implementations can check themselves against the spec.
    #[test]
    fn known_answer_vector() {
        let g = genesis_hash("prod");
        assert_eq!(
            hex::encode(g),
            "919a48ce11415cb2b2e4d466785b26178a045f9e30c51250212afb23f9c6ced9"
        );
        let (line, head) = encode_entry(0, &g, &json!({"b": 1, "a": 2})).unwrap();
        assert_eq!(
            hex::encode(head),
            "cb39fa7a3f347c4909599dd5855a89691581cee60dd7beedd6641bd3a82de7cc"
        );
        let s = String::from_utf8(line).unwrap();
        assert!(s.contains("\"event\":{\"a\":2,\"b\":1}"), "{s}");
        assert_eq!(s.matches('\n').count(), 1);
    }

    #[test]
    fn entry_hash_binds_seq_prev_and_content() {
        let g = genesis_hash("c");
        let c = content_hash(b"{}");
        let base = entry_hash(0, &g, &c);
        assert_ne!(base, entry_hash(1, &g, &c));
        assert_ne!(base, entry_hash(0, &content_hash(b"x"), &c));
        assert_ne!(base, entry_hash(0, &g, &content_hash(b"{ }")));
    }
}
