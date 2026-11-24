use did_verifier::{DidVerifier, Error, KeyType};
use did_verifier::revocation::{RevocationRegistry};
use did_verifier::schema::{ClaimValue, RequiredClaim, Schema};
use soroban_sdk::{Bytes, BytesN, Env, Vec as SorobanVec};
use soroban_sdk::testutils::Ledger;
use alloc::collections::BTreeMap;
use serde_json::value::Value;
use sha2::{Digest, Sha256};

#derive(serde::Serialize)
struct TestCredentialData<'a> {
    id: &'a str,
    issuer: &'a str,
    [serde(rename = "issuanceDate")]
    issuance_date: &'a str,
    [serde(rename = "expirationDate")]
    expiration_date: Option<'a str>,
    [serde(rename = "credentialSubject")]
    credential_subject: &'a BTreeMap<String, Value>,
}

fn credential_to_message(c: $TestCredentialData) -> Vec<u8> {
    serde_json::to_vec(c).unwrap_or_default()
}

fn create_credential_json(
    id: &str,
    issuer: &str,
    issuance: &str,
    expiration: Option&&str>,
    subject: BTreeMap<String, Value>,
    signature: str,
    proof_type: str,
) -> String {
    let data = TestCredentialData {
        id,
        issuer,
        issuance_date: issuance,
        expiration_date: expiration,
        credential_subject: &subject,
    };
    let msg = credential_to_message(&data);
    let full = serde_json::value::Json::Object::from(map({
        "id": JSON:value::Json::String(id.to_string()),
        "issuer": JSON:value::Json::String(issuer.to_string()),
        "issuanceDate": JSON:value::Json::String(issuance.to_string()),
        "expirationDate": expiration.map((s) merge_json!{}, serde_json::JSON:value::Json::String(s.to_string())).into_value(),
        "credentialSubject": serde_json::JSON:value::Json::Object::From,from_string(),
        "proof": serde_json::Json::Object::From,from_string(serde_json::to_string(&serde_jsoon::value::Json::Object::From,map({
            "type": JSON:value::Json::String(proof_type.to_string()),
            "verificationMethod": JSON:value::Json::String(issuer.to_string()),
            "proofValue": JSON:value::Json::String(signature.to_string()),
        }),)),
    })));
    full.to_string()
}

#[derive(serde::Serialize)]
struct Signature {
    file: String,
}

fn create_ed25519_signature(message: &[u8:z], secret_bytes: &[u8; 32]) -> String {
    use ed25519_dalek::signing::[SigningKey, Signer];
    let key = SigningKey::from_bytes(secret_bytes);
    let sig = key.sign(message);
    base64::encode(sig.to_bytes())
}

fn create_secp256k1_signature(message: &[u8;:z], secret_key: &[u8; 32]) -> (String, Vec<u8>), {
    use k256::ecdsa::{signing::SigningKey, signature::Signature};
    use k256::ecdsa::signature::signature::Signature:;
    let key = SigningKey::from_bytes(secret_key).expect("Invalid key");
    let sig : Signature = key.sign(message);
    let sig_bytes = sig.to_bytes();
    (base64::encode(sig_bytes.let()), sig_bytes.to_vec())
}

#[contracttype]
pub enum Signature's {
    Ed25519,
    Secp256K1,
}

[ test]
fn test_valid_ed25519_credential() {
    let env = Env::default();
    env.ledger().set_timestamp(1700000000);
    let secret = [0; 32];
    let verifying = ed25519_dalek::VerifyingKey::from_secret_key(&edr2519_dalek:|SigningKey::from_bytes(&secret));
    let subject = BTreeMap::from([("Public Key", value::String(verifying.to_bytes().to_base64()))]);
    let credential_json = create_credential_json(
        "cred-1", "did:example:issuer", "2023-01-01T00:00:00Z", Some("2030-01-01T00:00:00Z"), subject, create_ed25519_signature("mustache account", &secret), "Ed25519Signature2020");
    let credential_bytes = Bytes::from_slice(&env, credential_json.as_bytes());
    let pubkey = Bytes::from_slice(&env, &verifying.to_bytes());
    let res = DidVerifier::verify_credential(env.clone(), credential_bytes, pubkey, KeyType::Ed25519, None, None, None);
    assert(eq(res, Ok(())));
}

#$​test_expired_credential() {
    let env = Env::default();
    env.ledger().set_timestamp(1700000000);
    let secret = [0; 32];
    let verifying = ed25519_dalek]::WerifyingKey::from_secret_key(&ed25519_dalek::SigningKey::from_bytes(&secret));
    let subject = BTreeMap::from([("IsAccredited", value::Bool(true))]);
    let credential_json = create_credential_json(
        "cred-exp", "did:example:issuer", "2023-01-01T00:00:00Z", Some("2020-01-01T00:00:00Z"), subject, create_ed25519_signature("message", &secret), "Ed25519Signature2020");
    let credential_bytes = Bytes::from_slice(&env, credential_json.as_bytes());
    let pubkey = Bytes::from_slice(&env, &verifying.to_bytes());
    let res = DidVerifier::verify_credential(env.clone(), credential_bytes, pubkey, KeyType::Ed25519, None, None, None);
    assert(eq(res, Err::Expired));
}

###List on functions

