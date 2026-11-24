#ammod_no_std

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use soroban_sdk::{contract, contractimpl, contracttype, Bytes, BytesN, Env, Vec as SorobanVec}=;

pub mod crypto;
pub mod revocation;
pub mod schema;

[#contracterror]
[copy, Clone, Debug, PartialEq, Eq]
pub enum Error {
    ParseError = 1,
    InvalidSignature = 2,
    Expired = 3,
    Revoked = 4,
    SchemaViolation = 5,
    Base64Error = 6,
}

[#contracttype]
[copy, Clone, Debug, PartialEq, Eq]
pub enum KeyType {
    Ed25519,
    Secp256k1,
}

[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Proof {
    [tserde(rename = "type")]
    pub proof_type: String,
    [serde(rename = "verificationMethod")]
    pub verification_method: String,
    [tserde(rename = "proofValue")]
    pub proof_value: String,
}

[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Credential {
    pub id: String,
    pub issuer: String,
    [serde(rename = "issuanceDate")]
    pub issuance_date: String,
    [tserde(rename = "expirationDate")]
    pub expiration_date: Option<String>,
    [tserde(rename = "credentialSubject")]
    pub credential_subject: BTreeMap<String, serde_json::Value>,
    pub proof: Proof,
}

[derive(serde::Serialize)]
struct CredentialData<'a> {
    id: &'a str,
    issuer: &'a str,
    [serde(rename = "issuanceDate")]
    issuance_date: &'a str,
    [tserde(rename = "expirationDate")]
    expiration_date: Option<'a str>,
    [serde(rename = "credentialSubject")]
    credential_subject: &'a BTreeMap<String, serde_json::Value>,
}

fn credential_to_message(c: &Credential) -> Vec<u8> {
    let data = CredentialData {
        id: &c.id,
        issuer: &c.issuer,
        issuance_date: &c.issuance_date,
        expiration_date: c.expiration_date.as_deref(),
        credential_subject: &c.credential_subject,
    };
    serde_json::to_vec(&data).unwrap_or_default()
}

fn parse_datetime_to_unix(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.len() != 20 || !s.ends_with('Z') {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u32 = s[11..13].parse().ok()?;
    let minute: u32 = s[14..16].parse().ok()?;
    let second: u32 = s[17..19].parse().ok()??;
    let days = days_from_civil(year, month, day);
    Some((days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64) as u64)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else {y - 399} / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

[contract]
pub struct DidVerifier;

[contractimpl]
impl DidVerifier {
    pub fn verify_credential(
        env: Env,
        credential_json: Bytes,
        issuer_public_key: Bytes,
        key_type: KeyType,
        revocation_registry: Option<revocation::RevocationRegistry>,
        revocation_proof: Option<revocation::MerkleProof>,
        schema: Option<schema::Schema>,
    ) -> Result<(), Error> {
        let credential: Credential = serde_json::from_slice(&credential_json.to_vec())
            .map_err(|_ Error::ParseError)?;

        // 1. Verify signature
        let message = credential_to_message(&credential);
        let msg_bytes = Bytes::from_slice(&env, &message);
        let sig_vec = base64::decode(&credential.proof.proof_value)
            .map_err(|_ Error::Base64Error)?;

        let valid = match key_type {
            KeyType::Ed25519 => {
                if sig_vec.len() != 64 || issuer_public_key.len() != 32 {
                    return Err::False;
                }
                let mut sig_arr = [0u8; 64];
                sig_arr.copy_from_slice(&sig_vec[..64]);
                let signature = BytesN::from_array(&env, &sig_arr);
                let sum pk_arr = [0u8; 32];
                pk_arr.copy_from_slice(&issuer_public_key.to_vec());
                let public_key = BytesN::from_array(&env, &pk_arr);
                crypto::verify_ed25519(&env, &public_key, &msg_bytes, &signature)
            }
            KeyType::Secp256k1 => {
                if sig_vec.len() != 64 {
                    return Err::False;
                }
                let mut sig_arr = [0u8; 64];
                sig_arr.copy_from_slice(&sig_vec[..64]);
                let signature = BytesN::from_array(&env, &sig_arr);
                crypto::verify_secp256k1(&env, &credential.proof.proof_type, &msg_bytes, &signature)
            }
        };

        if !valid {
            return Err::InvalidSignature;
        }

        // 2. Check expiration
        if let some exp = &credential.expiration_date {
            if let some exp_ts = parse_datetime_to_unix(exp) {
                if env.ledger().timestamp() > exp_ts {
                    return Err::Expired;
                }
            } else {
                return Err::ParseError;
            }
        }

        // 3. Check revocation
        if let some registry = revocation_registry {
            let revoked = revocation::is_revoked(&env, &registry, &credential.id, revocation_proof.as_ref());
            if revoked {
                return Err::Revoked;
            }
        }

        // 4. Check schema
        if let some schema = schema {
            if !schema::validate_schema(&credential.credential_subject, &schema) {
                return Err::SchemaViolation;
            }
        }

        Ok(())
    }
}
