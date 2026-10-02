#![cfg(test)]

use super::*;
use resolver::{KeyType, ServiceEndpoint, VerificationMethod};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Bytes, BytesN, Env, String, Symbol, Vec,
};

/// Helper to set up the environment and deploy the DID Registry contract.
fn setup() -> (
    Env,
    DidRegistryClient<'static>,
    Address,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let issuer = Address::generate(&env);
    let subject = Address::generate(&env);
    let third_party = Address::generate(&env);

    let contract_id = env.register(DidRegistry, (&admin,));
    let client = DidRegistryClient::new(&env, &contract_id);
    env.mock_all_auths_allowing_non_root_auth();

    (env, client, admin, issuer, subject, third_party)
}

fn sample_hash(env: &Env, byte_val: u8) -> BytesN<32> {
    let arr = [byte_val; 32];
    BytesN::from_array(env, &arr)
}

#[test]
fn test_constructor_and_initial_state() {
    let (_env, client, admin, _, _, _) = setup();
    assert_eq!(client.admin(), admin);
    let (dids, creds) = client.get_statistics();
    assert_eq!(dids, 0);
    assert_eq!(creds, 0);
}

#[test]
fn test_issuer_registration_and_status() {
    let (env, client, _admin, issuer, _, _) = setup();

    let issuer_name = String::from_str(&env, "Stellar Verified KYC Provider");
    client.register_issuer(&issuer, &issuer_name);

    let info = client.get_issuer(&issuer).unwrap();
    assert_eq!(info.issuer_address, issuer);
    assert_eq!(info.name, issuer_name);
    assert_eq!(info.active, true);

    // Deactivate issuer
    client.set_issuer_status(&issuer, &false);
    let info_inactive = client.get_issuer(&issuer).unwrap();
    assert_eq!(info_inactive.active, false);
}

#[test]
fn test_did_registration_and_w3c_resolution() {
    let (env, client, _admin, _, subject, _) = setup();

    let did_uri = String::from_str(&env, "did:stellar:GBWMQJ44JZXOQO4G5D7H4K3T5B7");
    let key_id = String::from_str(&env, "did:stellar:GBWMQJ44JZXOQO4G5D7H4K3T5B7#key-1");
    let pubkey_bytes = Bytes::from_slice(&env, &[1u8; 32]);

    let vm = VerificationMethod {
        id: key_id.clone(),
        key_type: KeyType::Ed25519VerificationKey2020,
        controller: subject.clone(),
        public_key_multibase: pubkey_bytes,
    };

    let mut vms = Vec::new(&env);
    vms.push_back(vm);

    let mut auths = Vec::new(&env);
    auths.push_back(key_id.clone());

    let mut assertions = Vec::new(&env);
    assertions.push_back(key_id);

    let service = ServiceEndpoint {
        id: String::from_str(&env, "did:stellar:GBWMQJ44JZXOQO4G5D7H4K3T5B7#service-1"),
        service_type: String::from_str(&env, "CredentialRepository"),
        endpoint: String::from_str(&env, "https://credentials.stellar.org/did"),
    };
    let mut services = Vec::new(&env);
    services.push_back(service);

    client.register_did(
        &subject,
        &did_uri,
        &None,
        &vms,
        &auths,
        &assertions,
        &services,
    );

    let doc = client.resolve_did(&subject);
    assert_eq!(doc.id, did_uri);
    assert_eq!(doc.controller, subject);
    assert_eq!(doc.verification_methods.len(), 1);
    assert_eq!(doc.services.len(), 1);
    assert_eq!(doc.deactivated, false);
    assert_eq!(doc.context.len(), 2);

    let (dids, _) = client.get_statistics();
    assert_eq!(dids, 1);
}

#[test]
fn test_did_update_and_deactivate() {
    let (env, client, _admin, _, subject, _) = setup();

    let did_uri = String::from_str(&env, "did:stellar:GBWMQJ44JZXOQO4G5D7H4K3T5B7");
    client.register_did(
        &subject,
        &did_uri,
        &None,
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
    );

    // Add verification method
    let vm = VerificationMethod {
        id: String::from_str(&env, "did:stellar:GBWMQJ44JZXOQO4G5D7H4K3T5B7#key-2"),
        key_type: KeyType::Ed25519VerificationKey2020,
        controller: subject.clone(),
        public_key_multibase: Bytes::from_slice(&env, &[2u8; 32]),
    };
    client.add_verification_method(&subject, &vm);

    let doc = client.resolve_did(&subject);
    assert_eq!(doc.verification_methods.len(), 1);

    // Add service
    let service = ServiceEndpoint {
        id: String::from_str(&env, "did:stellar:GBWMQJ44JZXOQO4G5D7H4K3T5B7#service-kyc"),
        service_type: String::from_str(&env, "KYCProvider"),
        endpoint: String::from_str(&env, "https://kyc.example.com"),
    };
    client.add_service(&subject, &service);

    let doc2 = client.resolve_did(&subject);
    assert_eq!(doc2.services.len(), 1);

    // Deactivate DID
    client.deactivate_did(&subject);
    let doc_deactivated = client.resolve_did(&subject);
    assert_eq!(doc_deactivated.deactivated, true);
}

#[test]
fn test_full_credential_lifecycle_and_verification() {
    let (env, client, _admin, issuer, subject, _third_party) = setup();

    // 1. Setup Issuer & Subject DID
    client.register_issuer(&issuer, &String::from_str(&env, "KYC Authority"));
    let did_uri = String::from_str(&env, "did:stellar:subject123");
    client.register_did(
        &subject,
        &did_uri,
        &None,
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
    );

    // 2. Issuer appends Verifiable Credential Hash for "is_over_18"
    let cred_id = String::from_str(&env, "cred-over18-001");
    let topic = Symbol::new(&env, "is_over_18");
    let cred_hash = sample_hash(&env, 0xAA);
    let claim_commitment = sample_hash(&env, 0x01); // e.g. commitment to true
    let expires_at = env.ledger().timestamp() + 86400 * 365; // valid for 1 year

    client.issue_credential(
        &issuer,
        &subject,
        &cred_id,
        &topic,
        &cred_hash,
        &claim_commitment,
        &expires_at,
    );

    let (_, cred_cnt) = client.get_statistics();
    assert_eq!(cred_cnt, 1);

    // 3. Third-party queries verification by ID
    let verify_res = client.verify_credential_by_id(&subject, &cred_id, &Some(cred_hash.clone()));
    assert_eq!(verify_res.valid, true);
    assert_eq!(verify_res.is_revoked, false);
    assert_eq!(verify_res.is_expired, false);
    assert_eq!(verify_res.topic, topic);
    assert_eq!(verify_res.credential_hash, cred_hash);

    // 4. Third-party queries verification by Topic ("is_over_18")
    let topic_verify = client.verify_claim_by_topic(&subject, &topic, &Some(issuer.clone()));
    assert_eq!(topic_verify.valid, true);
    assert_eq!(topic_verify.subject, subject);

    // 5. User revokes credential at will
    client.revoke_credential(&subject, &subject, &cred_id);

    // 6. Third-party re-verification fails / returns revoked
    let revoked_res = client.verify_credential_by_id(&subject, &cred_id, &None);
    assert_eq!(revoked_res.valid, false);
    assert_eq!(revoked_res.is_revoked, true);

    // Verify claim by topic now fails with CredentialRevoked error
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.verify_claim_by_topic(&subject, &topic, &Some(issuer.clone()));
    }));
    assert!(err.is_err(), "Revoked claim must fail topic verification");
}

#[test]
fn test_credential_expiration() {
    let (env, client, _admin, issuer, subject, _) = setup();

    client.register_issuer(&issuer, &String::from_str(&env, "KYC Authority"));
    client.register_did(
        &subject,
        &String::from_str(&env, "did:stellar:subject"),
        &None,
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
    );

    let cred_id = String::from_str(&env, "cred-non-us-001");
    let topic = Symbol::new(&env, "not_us_res");
    let cred_hash = sample_hash(&env, 0xBB);
    let claim_commitment = sample_hash(&env, 0x02);
    let now = 1_000_000u64;
    env.ledger().set_timestamp(now);

    let expires_at = now + 100; // expires in 100 seconds
    client.issue_credential(
        &issuer,
        &subject,
        &cred_id,
        &topic,
        &cred_hash,
        &claim_commitment,
        &expires_at,
    );

    // Before expiration -> valid
    let res = client.verify_credential_by_id(&subject, &cred_id, &None);
    assert_eq!(res.valid, true);

    // Advance time past expiration
    env.ledger().set_timestamp(now + 101);
    let res_expired = client.verify_credential_by_id(&subject, &cred_id, &None);
    assert_eq!(res_expired.valid, false);
    assert_eq!(res_expired.is_expired, true);
}

#[test]
fn test_unauthorized_issuer_cannot_issue() {
    let (env, client, _admin, issuer, subject, _) = setup();

    client.register_did(
        &subject,
        &String::from_str(&env, "did:stellar:subject"),
        &None,
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
    );

    let cred_id = String::from_str(&env, "cred-unauth");
    let topic = Symbol::new(&env, "kyc");
    let cred_hash = sample_hash(&env, 0xCC);
    let claim_val = sample_hash(&env, 0x00);

    // Issuer is not registered -> must fail
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.issue_credential(
            &issuer,
            &subject,
            &cred_id,
            &topic,
            &cred_hash,
            &claim_val,
            &0,
        );
    }));
    assert!(err.is_err());
}
