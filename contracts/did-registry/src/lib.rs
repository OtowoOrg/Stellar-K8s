//! # Decentralized Identity (DID) and Verifiable Credentials Registry
//!
//! A high-performance Soroban smart contract providing W3C-compliant Decentralized
//! Identity (DID) management and zero-knowledge / privacy-preserving verifiable credential
//! anchoring.
//!
//! ## Architecture & Features
//!
//! - **W3C DID Document Resolution**: Direct mapping between Stellar / Soroban addresses
//!   and W3C-compliant DID documents (`did:stellar:<address>`), supporting verification
//!   methods, controllers, and service endpoints.
//! - **Authorized Issuer Engine**: Designated and authorized identity/KYC issuers (e.g. KYC providers,
//!   reputation authorities) can append verifiable credential hashes and cryptographic claim
//!   commitments directly to a subject's DID.
//! - **Privacy-Preserving On-Chain Verification**: Third-party protocols and DeFi dApps can
//!   verify credential validity, claim schema compliance, and expiration without exposing
//!   underlying Personally Identifiable Information (PII) on-chain.
//! - **User-Centric Immediate Revocation**: DID owners retain absolute sovereignty to revoke
//!   individual credentials or claim types at will, permanently invalidating them. Issuers can
//!   also revoke credentials they issued.

#![no_std]
#![allow(deprecated)]

pub mod resolver;

#[cfg(test)]
mod test;

use resolver::{
    build_did_document, DidDocument, KeyType, ServiceEndpoint, VerificationMethod,
};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, symbol_short, Address,
    Bytes, BytesN, Env, String, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

const INSTANCE_ADMIN: Symbol = symbol_short!("_admin");
const INSTANCE_FEE_VAULT: Symbol = symbol_short!("_vault");
const INSTANCE_DID_COUNT: Symbol = symbol_short!("_did_cnt");
const INSTANCE_CRED_COUNT: Symbol = symbol_short!("_c_cnt");

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Mapping of (Issuer Address) -> Authorized Issuer Info
    Issuer(Address),
    /// Mapping of (Subject Address) -> DID State
    DidRecord(Address),
    /// Mapping of (Subject Address, Credential ID) -> Verifiable Credential
    Credential(Address, String),
    /// Mapping of (Subject Address, Credential Topic / Schema Symbol) -> List of Credential IDs
    TopicCredentials(Address, Symbol),
    /// Mapping of (Subject Address, Credential ID) -> Revocation Status
    Revocation(Address, String),
}

// ---------------------------------------------------------------------------
// Error Codes
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DidError {
    Unauthorized = 1,
    DidAlreadyRegistered = 2,
    DidNotFound = 3,
    DidDeactivated = 4,
    IssuerAlreadyRegistered = 5,
    IssuerNotFound = 6,
    IssuerRevoked = 7,
    CredentialAlreadyExists = 8,
    CredentialNotFound = 9,
    CredentialRevoked = 10,
    CredentialExpired = 11,
    InvalidCredentialHash = 12,
    InvalidSchema = 13,
    InvalidExpiration = 14,
    InvalidParameters = 15,
}

// ---------------------------------------------------------------------------
// Data Models
// ---------------------------------------------------------------------------

/// Issuer record metadata.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuerInfo {
    pub issuer_address: Address,
    pub name: String,
    pub active: bool,
    pub registered_at: u64,
}

/// Internal DID registration record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DidRecord {
    pub did_uri: String,
    pub controller: Address,
    pub verification_methods: Vec<VerificationMethod>,
    pub authentication: Vec<String>,
    pub assertion_methods: Vec<String>,
    pub services: Vec<ServiceEndpoint>,
    pub created_at: u64,
    pub updated_at: u64,
    pub deactivated: bool,
}

/// A privacy-preserving Verifiable Credential anchored on-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiableCredential {
    /// Unique credential identifier (e.g., "cred-kyc-101", UUID, or W3C URI)
    pub id: String,
    /// DID Subject (Soroban address)
    pub subject: Address,
    /// Issuing authority address
    pub issuer: Address,
    /// Semantic topic / schema identifier (e.g., `is_over_18`, `kyc_tier_1`, `not_us_resident`)
    pub topic: Symbol,
    /// Cryptographic commitment / hash of the zero-knowledge proof or off-chain credential
    pub credential_hash: BytesN<32>,
    /// Optional claim value commitment / payload hash for selective disclosure
    pub claim_value: BytesN<32>,
    /// Issuance timestamp (Unix seconds)
    pub issued_at: u64,
    /// Optional expiration timestamp (Unix seconds, 0 = never expires)
    pub expires_at: u64,
    /// Revocation flag
    pub revoked: bool,
    /// Revocation timestamp (0 if not revoked)
    pub revoked_at: u64,
}

/// Verification result summary for consumer dApps.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationResult {
    pub valid: bool,
    pub subject: Address,
    pub issuer: Address,
    pub topic: Symbol,
    pub credential_hash: BytesN<32>,
    pub claim_value: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub is_revoked: bool,
    pub is_expired: bool,
}

// ---------------------------------------------------------------------------
// Contract Events
// ---------------------------------------------------------------------------

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DidRegistered {
    #[topic]
    pub subject: Address,
    pub did_uri: String,
    pub controller: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DidUpdated {
    #[topic]
    pub subject: Address,
    pub updated_at: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DidDeactivated {
    #[topic]
    pub subject: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuerRegistered {
    #[topic]
    pub issuer: Address,
    pub name: String,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuerStatusChanged {
    #[topic]
    pub issuer: Address,
    pub active: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialIssued {
    #[topic]
    pub subject: Address,
    #[topic]
    pub issuer: Address,
    #[topic]
    pub topic: Symbol,
    pub credential_id: String,
    pub credential_hash: BytesN<32>,
    pub expires_at: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialRevoked {
    #[topic]
    pub subject: Address,
    #[topic]
    pub credential_id: String,
    pub revoked_by: Address,
    pub revoked_at: u64,
}

// ---------------------------------------------------------------------------
// Contract Implementation
// ---------------------------------------------------------------------------

#[contract]
pub struct DidRegistry;

#[contractimpl]
impl DidRegistry {
    /// Initialize the DID and Verifiable Credentials Registry contract.
    pub fn __constructor(env: Env, admin: Address) {
        let store = env.storage().instance();
        store.set(&INSTANCE_ADMIN, &admin);
        store.set(&INSTANCE_DID_COUNT, &0u32);
        store.set(&INSTANCE_CRED_COUNT, &0u32);
        refresh_instance_ttl(&env);
    }

    // -----------------------------------------------------------------------
    // Admin & Issuer Management
    // -----------------------------------------------------------------------

    /// Fetch the registry admin address.
    pub fn admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&INSTANCE_ADMIN)
            .expect("Contract not initialized")
    }

    /// Set a new admin.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), DidError> {
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&INSTANCE_ADMIN)
            .ok_or(DidError::Unauthorized)?;
        current_admin.require_auth();

        env.storage().instance().set(&INSTANCE_ADMIN, &new_admin);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Register a designated issuer (e.g. KYC provider, accredited authority).
    pub fn register_issuer(env: Env, issuer: Address, name: String) -> Result<(), DidError> {
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&INSTANCE_ADMIN)
            .ok_or(DidError::Unauthorized)?;
        current_admin.require_auth();

        let issuer_key = DataKey::Issuer(issuer.clone());
        if env.storage().persistent().has(&issuer_key) {
            return Err(DidError::IssuerAlreadyRegistered);
        }

        let info = IssuerInfo {
            issuer_address: issuer.clone(),
            name: name.clone(),
            active: true,
            registered_at: env.ledger().timestamp(),
        };

        env.storage().persistent().set(&issuer_key, &info);
        refresh_instance_ttl(&env);

        IssuerRegistered { issuer, name }.publish(&env);
        Ok(())
    }

    /// Set issuer active status (enable or suspend issuer).
    pub fn set_issuer_status(env: Env, issuer: Address, active: bool) -> Result<(), DidError> {
        let current_admin: Address = env
            .storage()
            .instance()
            .get(&INSTANCE_ADMIN)
            .ok_or(DidError::Unauthorized)?;
        current_admin.require_auth();

        let issuer_key = DataKey::Issuer(issuer.clone());
        let mut info: IssuerInfo = env
            .storage()
            .persistent()
            .get(&issuer_key)
            .ok_or(DidError::IssuerNotFound)?;

        info.active = active;
        env.storage().persistent().set(&issuer_key, &info);
        refresh_instance_ttl(&env);

        IssuerStatusChanged { issuer, active }.publish(&env);
        Ok(())
    }

    /// Query issuer info.
    pub fn get_issuer(env: Env, issuer: Address) -> Option<IssuerInfo> {
        env.storage().persistent().get(&DataKey::Issuer(issuer))
    }

    // -----------------------------------------------------------------------
    // DID Lifecycle Management
    // -----------------------------------------------------------------------

    /// Create and initialize a new DID Document mapped directly to a Soroban address.
    pub fn register_did(
        env: Env,
        subject: Address,
        did_uri: String,
        controller: Option<Address>,
        verification_methods: Vec<VerificationMethod>,
        authentication: Vec<String>,
        assertion_methods: Vec<String>,
        services: Vec<ServiceEndpoint>,
    ) -> Result<(), DidError> {
        subject.require_auth();

        let did_key = DataKey::DidRecord(subject.clone());
        if env.storage().persistent().has(&did_key) {
            return Err(DidError::DidAlreadyRegistered);
        }

        let actual_controller = controller.unwrap_or_else(|| subject.clone());
        let now = env.ledger().timestamp();

        let record = DidRecord {
            did_uri: did_uri.clone(),
            controller: actual_controller.clone(),
            verification_methods,
            authentication,
            assertion_methods,
            services,
            created_at: now,
            updated_at: now,
            deactivated: false,
        };

        env.storage().persistent().set(&did_key, &record);

        let store = env.storage().instance();
        let cnt: u32 = store.get(&INSTANCE_DID_COUNT).unwrap_or(0);
        store.set(&INSTANCE_DID_COUNT, &(cnt + 1));
        refresh_instance_ttl(&env);

        DidRegistered {
            subject,
            did_uri,
            controller: actual_controller,
        }
        .publish(&env);

        Ok(())
    }

    /// Update an existing DID Document (verification methods, service endpoints, etc.).
    pub fn update_did(
        env: Env,
        subject: Address,
        verification_methods: Vec<VerificationMethod>,
        authentication: Vec<String>,
        assertion_methods: Vec<String>,
        services: Vec<ServiceEndpoint>,
    ) -> Result<(), DidError> {
        let did_key = DataKey::DidRecord(subject.clone());
        let mut record: DidRecord = env
            .storage()
            .persistent()
            .get(&did_key)
            .ok_or(DidError::DidNotFound)?;

        if record.deactivated {
            return Err(DidError::DidDeactivated);
        }

        // Require authorization from the DID controller
        record.controller.require_auth();

        let now = env.ledger().timestamp();
        record.verification_methods = verification_methods;
        record.authentication = authentication;
        record.assertion_methods = assertion_methods;
        record.services = services;
        record.updated_at = now;

        env.storage().persistent().set(&did_key, &record);
        refresh_instance_ttl(&env);

        DidUpdated {
            subject,
            updated_at: now,
        }
        .publish(&env);

        Ok(())
    }

    /// Add a verification method to an active DID.
    pub fn add_verification_method(
        env: Env,
        subject: Address,
        method: VerificationMethod,
    ) -> Result<(), DidError> {
        let did_key = DataKey::DidRecord(subject.clone());
        let mut record: DidRecord = env
            .storage()
            .persistent()
            .get(&did_key)
            .ok_or(DidError::DidNotFound)?;

        if record.deactivated {
            return Err(DidError::DidDeactivated);
        }

        record.controller.require_auth();

        record.verification_methods.push_back(method);
        record.updated_at = env.ledger().timestamp();

        env.storage().persistent().set(&did_key, &record);
        refresh_instance_ttl(&env);

        DidUpdated {
            subject,
            updated_at: record.updated_at,
        }
        .publish(&env);

        Ok(())
    }

    /// Add a service endpoint to an active DID.
    pub fn add_service(
        env: Env,
        subject: Address,
        service: ServiceEndpoint,
    ) -> Result<(), DidError> {
        let did_key = DataKey::DidRecord(subject.clone());
        let mut record: DidRecord = env
            .storage()
            .persistent()
            .get(&did_key)
            .ok_or(DidError::DidNotFound)?;

        if record.deactivated {
            return Err(DidError::DidDeactivated);
        }

        record.controller.require_auth();

        record.services.push_back(service);
        record.updated_at = env.ledger().timestamp();

        env.storage().persistent().set(&did_key, &record);
        refresh_instance_ttl(&env);

        DidUpdated {
            subject,
            updated_at: record.updated_at,
        }
        .publish(&env);

        Ok(())
    }

    /// Deactivate a DID permanently.
    pub fn deactivate_did(env: Env, subject: Address) -> Result<(), DidError> {
        let did_key = DataKey::DidRecord(subject.clone());
        let mut record: DidRecord = env
            .storage()
            .persistent()
            .get(&did_key)
            .ok_or(DidError::DidNotFound)?;

        if record.deactivated {
            return Err(DidError::DidDeactivated);
        }

        record.controller.require_auth();
        record.deactivated = true;
        record.updated_at = env.ledger().timestamp();

        env.storage().persistent().set(&did_key, &record);
        refresh_instance_ttl(&env);

        DidDeactivated { subject }.publish(&env);
        Ok(())
    }

    /// Resolve a W3C-compliant DID Document for a given Soroban subject address.
    pub fn resolve_did(env: Env, subject: Address) -> Result<DidDocument, DidError> {
        let did_key = DataKey::DidRecord(subject.clone());
        let record: DidRecord = env
            .storage()
            .persistent()
            .get(&did_key)
            .ok_or(DidError::DidNotFound)?;

        let doc = build_did_document(
            &env,
            record.did_uri,
            record.controller,
            record.verification_methods,
            record.authentication,
            record.assertion_methods,
            record.services,
            record.updated_at,
            record.deactivated,
        );

        Ok(doc)
    }

    // -----------------------------------------------------------------------
    // Verifiable Credentials & Issuance Engine
    // -----------------------------------------------------------------------

    /// Append an authorized verifiable credential hash to a user's DID.
    ///
    /// Callable by designated active issuers.
    pub fn issue_credential(
        env: Env,
        issuer: Address,
        subject: Address,
        credential_id: String,
        topic: Symbol,
        credential_hash: BytesN<32>,
        claim_value: BytesN<32>,
        expires_at: u64,
    ) -> Result<(), DidError> {
        issuer.require_auth();

        // Validate issuer authorization
        let issuer_key = DataKey::Issuer(issuer.clone());
        let issuer_info: IssuerInfo = env
            .storage()
            .persistent()
            .get(&issuer_key)
            .ok_or(DidError::IssuerNotFound)?;

        if !issuer_info.active {
            return Err(DidError::IssuerRevoked);
        }

        // Validate that subject DID exists and is not deactivated
        let did_key = DataKey::DidRecord(subject.clone());
        let did_record: DidRecord = env
            .storage()
            .persistent()
            .get(&did_key)
            .ok_or(DidError::DidNotFound)?;

        if did_record.deactivated {
            return Err(DidError::DidDeactivated);
        }

        let now = env.ledger().timestamp();
        if expires_at != 0 && expires_at <= now {
            return Err(DidError::InvalidExpiration);
        }

        let cred_key = DataKey::Credential(subject.clone(), credential_id.clone());
        if env.storage().persistent().has(&cred_key) {
            return Err(DidError::CredentialAlreadyExists);
        }

        let credential = VerifiableCredential {
            id: credential_id.clone(),
            subject: subject.clone(),
            issuer: issuer.clone(),
            topic: topic.clone(),
            credential_hash: credential_hash.clone(),
            claim_value,
            issued_at: now,
            expires_at,
            revoked: false,
            revoked_at: 0,
        };

        env.storage().persistent().set(&cred_key, &credential);

        // Maintain topic indexing for subject
        let topic_key = DataKey::TopicCredentials(subject.clone(), topic.clone());
        let mut cred_list: Vec<String> = env
            .storage()
            .persistent()
            .get(&topic_key)
            .unwrap_or_else(|| Vec::new(&env));
        cred_list.push_back(credential_id.clone());
        env.storage().persistent().set(&topic_key, &cred_list);

        let store = env.storage().instance();
        let cnt: u32 = store.get(&INSTANCE_CRED_COUNT).unwrap_or(0);
        store.set(&INSTANCE_CRED_COUNT, &(cnt + 1));
        refresh_instance_ttl(&env);

        CredentialIssued {
            subject,
            issuer,
            topic,
            credential_id,
            credential_hash,
            expires_at,
        }
        .publish(&env);

        Ok(())
    }

    /// Revoke a credential.
    ///
    /// Can be invoked by:
    /// 1. The **Subject** (user sovereignty to permanently revoke at will)
    /// 2. The original **Issuer** that created the credential
    /// 3. The **Registry Admin**
    pub fn revoke_credential(
        env: Env,
        caller: Address,
        subject: Address,
        credential_id: String,
    ) -> Result<(), DidError> {
        caller.require_auth();

        let cred_key = DataKey::Credential(subject.clone(), credential_id.clone());
        let mut credential: VerifiableCredential = env
            .storage()
            .persistent()
            .get(&cred_key)
            .ok_or(DidError::CredentialNotFound)?;

        if credential.revoked {
            return Err(DidError::CredentialRevoked);
        }

        let admin_addr: Address = env.storage().instance().get(&INSTANCE_ADMIN).unwrap();

        // Authorization check: subject, original issuer, or registry admin
        let is_subject = caller == subject;
        let is_issuer = caller == credential.issuer;
        let is_admin = caller == admin_addr;

        if !is_subject && !is_issuer && !is_admin {
            return Err(DidError::Unauthorized);
        }

        let now = env.ledger().timestamp();
        credential.revoked = true;
        credential.revoked_at = now;

        env.storage().persistent().set(&cred_key, &credential);
        refresh_instance_ttl(&env);

        CredentialRevoked {
            subject,
            credential_id,
            revoked_by: caller,
            revoked_at: now,
        }
        .publish(&env);

        Ok(())
    }

    /// Query the status of a specific credential.
    pub fn get_credential(
        env: Env,
        subject: Address,
        credential_id: String,
    ) -> Option<VerifiableCredential> {
        env.storage()
            .persistent()
            .get(&DataKey::Credential(subject, credential_id))
    }

    /// Query list of credential IDs under a specific topic / claim type for a subject.
    pub fn get_credentials_by_topic(
        env: Env,
        subject: Address,
        topic: Symbol,
    ) -> Vec<String> {
        env.storage()
            .persistent()
            .get(&DataKey::TopicCredentials(subject, topic))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // -----------------------------------------------------------------------
    // Third-Party Verification Engine
    // -----------------------------------------------------------------------

    /// Verify the validity and claims of a specific credential.
    ///
    /// Checks:
    /// - Existence of DID and credential
    /// - Issuer authorization status (active)
    /// - Hash integrity matching expected proof
    /// - Revocation state
    /// - Expiration timestamp against current ledger time
    pub fn verify_credential_by_id(
        env: Env,
        subject: Address,
        credential_id: String,
        expected_hash: Option<BytesN<32>>,
    ) -> Result<VerificationResult, DidError> {
        let cred_key = DataKey::Credential(subject.clone(), credential_id);
        let credential: VerifiableCredential = env
            .storage()
            .persistent()
            .get(&cred_key)
            .ok_or(DidError::CredentialNotFound)?;

        let now = env.ledger().timestamp();
        let is_expired = credential.expires_at != 0 && now >= credential.expires_at;
        let is_revoked = credential.revoked;

        // Verify issuer status
        let issuer_info = Self::get_issuer(env.clone(), credential.issuer.clone());
        let issuer_active = issuer_info.map(|i| i.active).unwrap_or(false);

        // Verify hash if expected_hash was provided
        let hash_matches = match expected_hash {
            Some(ref h) => *h == credential.credential_hash,
            None => true,
        };

        if !hash_matches {
            return Err(DidError::InvalidCredentialHash);
        }

        let is_valid = !is_revoked && !is_expired && issuer_active;

        Ok(VerificationResult {
            valid: is_valid,
            subject: credential.subject,
            issuer: credential.issuer,
            topic: credential.topic,
            credential_hash: credential.credential_hash,
            claim_value: credential.claim_value,
            issued_at: credential.issued_at,
            expires_at: credential.expires_at,
            is_revoked,
            is_expired,
        })
    }

    /// Verify if a subject possesses an active, valid credential for a given topic
    /// (e.g., `symbol_short!("over_18")`, `symbol_short!("non_us")`, `symbol_short!("kyc_pass")`).
    ///
    /// Returns the verified claim result if at least one valid, unrevoked, unexpired credential exists.
    pub fn verify_claim_by_topic(
        env: Env,
        subject: Address,
        topic: Symbol,
        trusted_issuer: Option<Address>,
    ) -> Result<VerificationResult, DidError> {
        let topic_key = DataKey::TopicCredentials(subject.clone(), topic.clone());
        let cred_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&topic_key)
            .ok_or(DidError::CredentialNotFound)?;

        if cred_ids.is_empty() {
            return Err(DidError::CredentialNotFound);
        }

        let now = env.ledger().timestamp();

        for cred_id in cred_ids.iter() {
            if let Some(cred) = env
                .storage()
                .persistent()
                .get::<_, VerifiableCredential>(&DataKey::Credential(subject.clone(), cred_id))
            {
                // Check trusted issuer filter if specified
                if let Some(ref trusted) = trusted_issuer {
                    if *trusted != cred.issuer {
                        continue;
                    }
                }

                // Check issuer active status
                let issuer_info = Self::get_issuer(env.clone(), cred.issuer.clone());
                if !issuer_info.map(|i| i.active).unwrap_or(false) {
                    continue;
                }

                let is_expired = cred.expires_at != 0 && now >= cred.expires_at;
                if !cred.revoked && !is_expired {
                    return Ok(VerificationResult {
                        valid: true,
                        subject: cred.subject,
                        issuer: cred.issuer,
                        topic: cred.topic,
                        credential_hash: cred.credential_hash,
                        claim_value: cred.claim_value,
                        issued_at: cred.issued_at,
                        expires_at: cred.expires_at,
                        is_revoked: false,
                        is_expired: false,
                    });
                }
            }
        }

        // If no active valid credential matched
        Err(DidError::CredentialRevoked)
    }

    /// Query global statistics: registered DIDs and issued Credentials count.
    pub fn get_statistics(env: Env) -> (u32, u32) {
        let store = env.storage().instance();
        let did_cnt = store.get(&INSTANCE_DID_COUNT).unwrap_or(0);
        let cred_cnt = store.get(&INSTANCE_CRED_COUNT).unwrap_or(0);
        (did_cnt, cred_cnt)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn refresh_instance_ttl(env: &Env) {
    let max = env.storage().max_ttl();
    env.storage().instance().extend_ttl(max, max);
}
