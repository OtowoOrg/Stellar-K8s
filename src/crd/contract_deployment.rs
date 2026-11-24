use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Digest;

/// A Soroban WASM artifact and its signed upload transaction.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ContractWASM",
    namespaced,
    status = "ContractWASMStatus",
    shortname = "cwasm"
)]
#[serde(rename_all = "camelCase")]
pub struct ContractWASMSpec {
    pub rpc_url: String,
    pub wasm_uri: String,
    /// Ledger-key XDR for the WASM code entry used by getLedgerEntries.
    pub wasm_ledger_key_xdr: String,
    /// Expected SHA-256 digest as 64 lowercase hexadecimal characters.
    pub sha256: String,
    /// Signed Soroban transaction envelope that uploads the artifact.
    pub upload_transaction_xdr: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContractWASMStatus {
    pub phase: ContractDeploymentPhase,
    pub wasm_hash: Option<String>,
    pub transaction_hash: Option<String>,
    pub last_error: Option<String>,
}

/// A Soroban contract instance deployed from an uploaded WASM artifact.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ContractInstance",
    namespaced,
    status = "ContractInstanceStatus",
    shortname = "cinst"
)]
#[serde(rename_all = "camelCase")]
pub struct ContractInstanceSpec {
    pub rpc_url: String,
    pub wasm: String,
    /// Expected WASM hash; must match the referenced ContractWASM status.
    pub wasm_hash: String,
    /// Deterministic Soroban contract ID derived from the signed create transaction.
    pub contract_id: String,
    /// Ledger-key XDR for this contract instance, used for idempotent preflight.
    pub contract_ledger_key_xdr: String,
    /// Signed transaction envelope that invokes the constructor/create operation.
    pub deployment_transaction_xdr: String,
    #[serde(default)]
    pub storage: Vec<ContractStorageEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContractStorageEntry {
    pub key_xdr: String,
    pub value_xdr: String,
    pub durability: StorageDurability,
    pub ttl_ledgers: u32,
    /// Signed Soroban invocation that writes this storage entry.
    pub transaction_xdr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum StorageDurability {
    Temporary,
    Persistent,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ContractDeploymentPhase {
    #[default]
    Pending,
    Submitted,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContractInstanceStatus {
    pub phase: ContractDeploymentPhase,
    pub contract_id: Option<String>,
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub storage_index: u32,
    #[serde(default)]
    pub deployment_complete: bool,
    pub last_error: Option<String>,
}

/// Validate the user-supplied digest before downloading or submitting an artifact.
pub fn validate_sha256(expected: &str, wasm: &[u8]) -> Result<String, String> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("sha256 must contain exactly 64 hexadecimal characters".to_string());
    }

    let actual = hex::encode(sha2::Sha256::digest(wasm));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "WASM SHA-256 mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::validate_sha256;
    use sha2::{Digest, Sha256};

    #[test]
    fn validates_wasm_digest_before_upload() {
        let wasm = b"wasm-module";
        let digest = hex::encode(Sha256::digest(wasm));
        assert_eq!(validate_sha256(&digest, wasm).unwrap(), digest);
        assert!(validate_sha256(&"0".repeat(64), wasm).is_err());
        assert!(validate_sha256("not-a-digest", wasm).is_err());
    }
}
