//! W3C-compliant DID document resolver and data types.
//!
//! Provides data structures and resolution logic for W3C Decentralized Identifiers (DIDs)
//! bound directly to Stellar/Soroban addresses under the `did:stellar:` method.

use soroban_sdk::{contracttype, Address, Bytes, Env, String, Vec};

/// Key / Verification Method type adhering to W3C standards.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyType {
    /// Ed25519VerificationKey2020 standard format
    Ed25519VerificationKey2020 = 1,
    /// EcdsaSecp256k1VerificationKey2019 format
    EcdsaSecp256k1RecoveryMethod2020 = 2,
}

/// Verification Method entry in a W3C DID Document.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationMethod {
    /// Verification method identifier (e.g., "did:stellar:<addr>#key-1")
    pub id: String,
    /// Type of verification key
    pub key_type: KeyType,
    /// DID controller address for this verification method
    pub controller: Address,
    /// Public key bytes (e.g. 32 bytes for Ed25519, 33/65 bytes for Secp256k1)
    pub public_key_multibase: Bytes,
}

/// Service endpoint entry in a W3C DID Document.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceEndpoint {
    /// Unique service identifier (e.g. "did:stellar:<addr>#kyc-service")
    pub id: String,
    /// Type of service (e.g. "KYCProvider", "CredentialRepository", "LinkedDomains")
    pub service_type: String,
    /// Target service URI / endpoint (e.g. "https://kyc.stellar.org/verify")
    pub endpoint: String,
}

/// A W3C-compliant DID Document representation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DidDocument {
    /// JSON-LD context array, e.g. ["https://www.w3.org/ns/did/v1"]
    pub context: Vec<String>,
    /// The DID identifier (e.g., "did:stellar:<addr>")
    pub id: String,
    /// DID Controller Soroban address
    pub controller: Address,
    /// Verification methods (cryptographic material)
    pub verification_methods: Vec<VerificationMethod>,
    /// Authentication relationship references (e.g. ["#key-1"])
    pub authentication: Vec<String>,
    /// Assertion method relationship references (e.g. ["#key-1"])
    pub assertion_methods: Vec<String>,
    /// Associated service endpoints
    pub services: Vec<ServiceEndpoint>,
    /// Last updated timestamp (ledger timestamp)
    pub updated: u64,
    /// Deactivation status
    pub deactivated: bool,
}

/// Pure resolution builder to construct a W3C DID Document from raw state components.
pub fn build_did_document(
    env: &Env,
    did_uri: String,
    controller: Address,
    verification_methods: Vec<VerificationMethod>,
    authentication: Vec<String>,
    assertion_methods: Vec<String>,
    services: Vec<ServiceEndpoint>,
    updated: u64,
    deactivated: bool,
) -> DidDocument {
    let mut context = Vec::new(env);
    context.push_back(String::from_str(env, "https://www.w3.org/ns/did/v1"));
    context.push_back(String::from_str(
        env,
        "https://w3id.org/security/suites/ed25519-2020/v1",
    ));

    DidDocument {
        context,
        id: did_uri,
        controller,
        verification_methods,
        authentication,
        assertion_methods,
        services,
        updated,
        deactivated,
    }
}
