//! Secret Manager backends used by validator key rotation.
//!
//! The rotation worker is the only controller component that handles raw
//! validator seeds. Keep loggable fields to fingerprints, versions, paths and
//! backend names; never log or expose the secret seed.

use async_trait::async_trait;
use aws_sdk_secretsmanager::Client as SecretsManagerClient;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::convert::TryInto;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::error::{Error, Result};

const SECRET_SEED_VERSION_BYTE: u8 = 18 << 3;
const PUBLIC_KEY_VERSION_BYTE: u8 = 6 << 3;
const STRKEY_PAYLOAD_LEN: usize = 32;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Safe metadata for a stored validator seed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretVersion {
    /// Secret-manager version identifier or stable synthetic version.
    pub version_id: String,
    /// Server or local creation timestamp.
    pub created_at: DateTime<Utc>,
    /// SHA-256 fingerprint of the StrKey seed, safe for audit logs.
    pub fingerprint: String,
}

impl SecretVersion {
    pub fn new(version_id: impl Into<String>, fingerprint: impl Into<String>) -> Self {
        Self {
            version_id: version_id.into(),
            created_at: Utc::now(),
            fingerprint: fingerprint.into(),
        }
    }
}

/// Validator seed material plus safe derived metadata.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorSeedMaterial {
    seed: String,
    /// Stellar public key derived from the seed.
    pub public_key: String,
    /// SHA-256 fingerprint of the seed.
    pub fingerprint: String,
}

impl fmt::Debug for ValidatorSeedMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatorSeedMaterial")
            .field("seed", &"<redacted>")
            .field("public_key", &self.public_key)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl ValidatorSeedMaterial {
    /// Generate a fresh Stellar Secret Seed and corresponding public key.
    pub fn generate() -> Result<Self> {
        let mut raw_seed = [0u8; STRKEY_PAYLOAD_LEN];
        rand::thread_rng().fill_bytes(&mut raw_seed);
        Self::from_raw_seed(raw_seed)
    }

    /// Build material from an existing Stellar Secret Seed.
    pub fn from_seed(seed: impl Into<String>) -> Result<Self> {
        let seed = seed.into();
        let raw_seed = decode_strkey(&seed, SECRET_SEED_VERSION_BYTE)?;
        let raw_seed: [u8; STRKEY_PAYLOAD_LEN] = raw_seed.try_into().map_err(|_| {
            Error::ValidationError("validator seed must contain 32 raw bytes".to_string())
        })?;
        Self::from_seed_and_bytes(seed, raw_seed)
    }

    fn from_raw_seed(raw_seed: [u8; STRKEY_PAYLOAD_LEN]) -> Result<Self> {
        let seed = encode_strkey(SECRET_SEED_VERSION_BYTE, &raw_seed);
        Self::from_seed_and_bytes(seed, raw_seed)
    }

    fn from_seed_and_bytes(seed: String, raw_seed: [u8; STRKEY_PAYLOAD_LEN]) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(&raw_seed);
        let public_key = encode_strkey(
            PUBLIC_KEY_VERSION_BYTE,
            signing_key.verifying_key().as_bytes(),
        );
        let fingerprint = seed_fingerprint(&seed);

        Ok(Self {
            seed,
            public_key,
            fingerprint,
        })
    }

    /// Raw secret value for backend writes only. Do not log this.
    pub fn secret_seed(&self) -> &str {
        &self.seed
    }
}

/// A seed and its backend version metadata.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedSeedSecret {
    pub material: ValidatorSeedMaterial,
    pub version: SecretVersion,
}

impl ManagedSeedSecret {
    pub fn new(material: ValidatorSeedMaterial, version: SecretVersion) -> Self {
        Self { material, version }
    }
}

/// Backend contract used by the key rotation worker.
#[async_trait]
pub trait SecretManagerBackend: Send + Sync {
    async fn read_current(&self) -> Result<ManagedSeedSecret>;

    async fn put_candidate(
        &self,
        candidate: &ValidatorSeedMaterial,
        previous: &ManagedSeedSecret,
    ) -> Result<ManagedSeedSecret>;

    async fn promote_candidate(
        &self,
        candidate: &ManagedSeedSecret,
        previous: &ManagedSeedSecret,
    ) -> Result<()>;

    async fn rollback(
        &self,
        previous: &ManagedSeedSecret,
        failed_candidate: Option<&ManagedSeedSecret>,
    ) -> Result<()>;

    fn backend_name(&self) -> &'static str;
}

/// HashiCorp Vault KV-v2 configuration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultKv2Config {
    /// Vault base URL, for example `https://vault.vault.svc:8200`.
    pub address: String,
    /// Vault token. Prefer short-lived Kubernetes auth tokens in production.
    pub token: String,
    /// KV-v2 data path, for example `secret/data/stellar/validator-a`.
    pub secret_path: String,
    /// Field name containing the Stellar secret seed.
    #[serde(default = "default_seed_field")]
    pub seed_field: String,
    /// HTTP timeout for Vault requests.
    #[serde(default = "default_timeout")]
    pub timeout: Duration,
}

fn default_seed_field() -> String {
    "seed".to_string()
}

fn default_timeout() -> Duration {
    Duration::from_secs(10)
}

/// Vault KV-v2 backend.
#[derive(Clone)]
pub struct VaultKv2Backend {
    config: VaultKv2Config,
    client: reqwest::Client,
}

impl fmt::Debug for VaultKv2Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultKv2Backend")
            .field("address", &self.config.address)
            .field("secret_path", &self.config.secret_path)
            .field("seed_field", &self.config.seed_field)
            .finish()
    }
}

impl VaultKv2Backend {
    pub fn new(config: VaultKv2Config) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let token = HeaderValue::from_str(&config.token)
            .map_err(|e| Error::ConfigError(format!("invalid Vault token header: {e}")))?;
        headers.insert("X-Vault-Token", token);

        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .default_headers(headers)
            .build()
            .map_err(|e| Error::ConfigError(format!("failed to build Vault client: {e}")))?;

        Ok(Self { config, client })
    }

    pub fn from_env(secret_path: impl Into<String>, seed_field: impl Into<String>) -> Result<Self> {
        let address = std::env::var("VAULT_ADDR").map_err(|_| {
            Error::ConfigError("VAULT_ADDR is required for Vault key rotation".to_string())
        })?;
        let token = std::env::var("VAULT_TOKEN").map_err(|_| {
            Error::ConfigError("VAULT_TOKEN is required for Vault key rotation".to_string())
        })?;

        Self::new(VaultKv2Config {
            address,
            token,
            secret_path: secret_path.into(),
            seed_field: seed_field.into(),
            timeout: default_timeout(),
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1/{}",
            self.config.address.trim_end_matches('/'),
            self.config.secret_path.trim_start_matches('/')
        )
    }

    async fn write_seed_document(
        &self,
        material: &ValidatorSeedMaterial,
        state: &str,
        previous_version: Option<&SecretVersion>,
    ) -> Result<ManagedSeedSecret> {
        let requested_version = format!(
            "{state}-{}-{}",
            Utc::now().timestamp_millis(),
            material.fingerprint.chars().take(12).collect::<String>()
        );
        let body = serde_json::json!({
            "data": seed_document(
                &self.config.seed_field,
                material,
                state,
                &requested_version,
                previous_version,
            )
        });

        let response = self
            .client
            .post(self.endpoint())
            .json(&body)
            .send()
            .await
            .map_err(Error::HttpError)?;

        if !response.status().is_success() {
            return Err(Error::ConfigError(format!(
                "Vault write failed for {}: HTTP {}",
                self.config.secret_path,
                response.status()
            )));
        }

        let metadata = response.json::<Value>().await.unwrap_or(Value::Null);
        let version_id = metadata
            .pointer("/data/version")
            .and_then(Value::as_u64)
            .map(|v| v.to_string())
            .unwrap_or(requested_version);

        Ok(ManagedSeedSecret::new(
            material.clone(),
            SecretVersion::new(version_id, material.fingerprint.clone()),
        ))
    }
}

#[async_trait]
impl SecretManagerBackend for VaultKv2Backend {
    async fn read_current(&self) -> Result<ManagedSeedSecret> {
        let response = self
            .client
            .get(self.endpoint())
            .send()
            .await
            .map_err(Error::HttpError)?;

        if !response.status().is_success() {
            return Err(Error::ConfigError(format!(
                "Vault read failed for {}: HTTP {}",
                self.config.secret_path,
                response.status()
            )));
        }

        let body: Value = response.json().await.map_err(Error::HttpError)?;
        let data = vault_data_map(&body).ok_or_else(|| {
            Error::ConfigError(format!(
                "Vault secret {} did not contain a data object",
                self.config.secret_path
            ))
        })?;
        let seed = extract_seed(data, &self.config.seed_field)?;
        let material = ValidatorSeedMaterial::from_seed(seed)?;
        let version_id = body
            .pointer("/data/metadata/version")
            .and_then(Value::as_u64)
            .map(|v| v.to_string())
            .or_else(|| {
                data.get("rotationVersion")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("vault-{}", &material.fingerprint[..12]));
        let created_at = body
            .pointer("/data/metadata/created_time")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339)
            .unwrap_or_else(Utc::now);

        Ok(ManagedSeedSecret::new(
            material,
            SecretVersion {
                version_id,
                created_at,
                fingerprint: data
                    .get("fingerprint")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        body.pointer("/data/fingerprint")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    }),
            },
        ))
        .map(|mut managed| {
            if managed.version.fingerprint.is_empty() {
                managed.version.fingerprint = managed.material.fingerprint.clone();
            }
            managed
        })
    }

    async fn put_candidate(
        &self,
        candidate: &ValidatorSeedMaterial,
        previous: &ManagedSeedSecret,
    ) -> Result<ManagedSeedSecret> {
        self.write_seed_document(candidate, "pending", Some(&previous.version))
            .await
    }

    async fn promote_candidate(
        &self,
        candidate: &ManagedSeedSecret,
        previous: &ManagedSeedSecret,
    ) -> Result<()> {
        self.write_seed_document(&candidate.material, "active", Some(&previous.version))
            .await?;
        Ok(())
    }

    async fn rollback(
        &self,
        previous: &ManagedSeedSecret,
        _failed_candidate: Option<&ManagedSeedSecret>,
    ) -> Result<()> {
        self.write_seed_document(&previous.material, "rolled_back", None)
            .await?;
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "vault-kv2"
    }
}

/// AWS Secrets Manager configuration.
#[derive(Clone)]
pub struct AwsSecretsManagerConfig {
    pub secret_id: String,
    pub seed_field: String,
    pub store_json: bool,
    pub region: Option<String>,
}

impl fmt::Debug for AwsSecretsManagerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsSecretsManagerConfig")
            .field("secret_id", &self.secret_id)
            .field("seed_field", &self.seed_field)
            .field("store_json", &self.store_json)
            .field("region", &self.region)
            .finish()
    }
}

/// AWS Secrets Manager backend using version staging labels for rollback.
#[derive(Clone)]
pub struct AwsSecretsManagerBackend {
    client: SecretsManagerClient,
    config: AwsSecretsManagerConfig,
}

impl fmt::Debug for AwsSecretsManagerBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsSecretsManagerBackend")
            .field("secret_id", &self.config.secret_id)
            .field("seed_field", &self.config.seed_field)
            .field("region", &self.config.region)
            .finish()
    }
}

impl AwsSecretsManagerBackend {
    pub fn new(client: SecretsManagerClient, config: AwsSecretsManagerConfig) -> Self {
        Self { client, config }
    }

    pub async fn from_default_config(config: AwsSecretsManagerConfig) -> Self {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let mut builder = aws_sdk_secretsmanager::config::Builder::from(&sdk_config);
        if let Some(region) = &config.region {
            builder = builder.region(aws_sdk_secretsmanager::config::Region::new(region.clone()));
        }

        Self {
            client: SecretsManagerClient::from_conf(builder.build()),
            config,
        }
    }

    fn document_string(
        &self,
        material: &ValidatorSeedMaterial,
        state: &str,
        version_id: &str,
        previous_version: Option<&SecretVersion>,
    ) -> Result<String> {
        if !self.config.store_json {
            return Ok(material.secret_seed().to_string());
        }

        serde_json::to_string(&seed_document(
            &self.config.seed_field,
            material,
            state,
            version_id,
            previous_version,
        ))
        .map_err(Error::SerializationError)
    }
}

#[async_trait]
impl SecretManagerBackend for AwsSecretsManagerBackend {
    async fn read_current(&self) -> Result<ManagedSeedSecret> {
        let response = self
            .client
            .get_secret_value()
            .secret_id(&self.config.secret_id)
            .send()
            .await
            .map_err(|e| {
                Error::ConfigError(format!(
                    "AWS Secrets Manager read failed for {}: {e}",
                    self.config.secret_id
                ))
            })?;

        let secret_string = response.secret_string().ok_or_else(|| {
            Error::ConfigError(format!(
                "AWS Secrets Manager secret {} has no SecretString",
                self.config.secret_id
            ))
        })?;
        let material = parse_seed_material(secret_string, &self.config.seed_field)?;
        let version_id = response
            .version_id()
            .map(str::to_string)
            .unwrap_or_else(|| format!("aws-{}", &material.fingerprint[..12]));

        Ok(ManagedSeedSecret::new(
            material.clone(),
            SecretVersion::new(version_id, material.fingerprint),
        ))
    }

    async fn put_candidate(
        &self,
        candidate: &ValidatorSeedMaterial,
        previous: &ManagedSeedSecret,
    ) -> Result<ManagedSeedSecret> {
        let client_request_token = format!(
            "stellar-key-rotation-{}-{}",
            Utc::now().timestamp_millis(),
            candidate.fingerprint.chars().take(16).collect::<String>()
        );
        let secret_string = self.document_string(
            candidate,
            "pending",
            &client_request_token,
            Some(&previous.version),
        )?;
        let response = self
            .client
            .put_secret_value()
            .secret_id(&self.config.secret_id)
            .client_request_token(&client_request_token)
            .secret_string(secret_string)
            .send()
            .await
            .map_err(|e| {
                Error::ConfigError(format!(
                    "AWS Secrets Manager candidate write failed for {}: {e}",
                    self.config.secret_id
                ))
            })?;

        let version_id = response
            .version_id()
            .map(str::to_string)
            .unwrap_or(client_request_token);

        Ok(ManagedSeedSecret::new(
            candidate.clone(),
            SecretVersion::new(version_id, candidate.fingerprint.clone()),
        ))
    }

    async fn promote_candidate(
        &self,
        _candidate: &ManagedSeedSecret,
        _previous: &ManagedSeedSecret,
    ) -> Result<()> {
        // `put_secret_value` without an explicit stage makes the candidate the
        // AWSCURRENT version so the validation window observes the live key.
        // Rollback below moves AWSCURRENT back if validation fails.
        Ok(())
    }

    async fn rollback(
        &self,
        previous: &ManagedSeedSecret,
        failed_candidate: Option<&ManagedSeedSecret>,
    ) -> Result<()> {
        let mut request = self
            .client
            .update_secret_version_stage()
            .secret_id(&self.config.secret_id)
            .version_stage("AWSCURRENT")
            .move_to_version_id(&previous.version.version_id);

        if let Some(candidate) = failed_candidate {
            request = request.remove_from_version_id(&candidate.version.version_id);
        }

        request.send().await.map_err(|e| {
            Error::ConfigError(format!(
                "AWS Secrets Manager rollback failed for {}: {e}",
                self.config.secret_id
            ))
        })?;
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "aws-secrets-manager"
    }
}

/// In-memory backend used by unit and integration tests.
#[derive(Clone, Debug)]
pub struct MemorySecretManager {
    inner: Arc<Mutex<MemorySecretManagerState>>,
}

#[derive(Clone, Debug)]
struct MemorySecretManagerState {
    current: ManagedSeedSecret,
    pending: Option<ManagedSeedSecret>,
    audit: Vec<String>,
}

impl MemorySecretManager {
    pub fn new(initial: ValidatorSeedMaterial) -> Self {
        let version = SecretVersion::new("initial", initial.fingerprint.clone());
        Self {
            inner: Arc::new(Mutex::new(MemorySecretManagerState {
                current: ManagedSeedSecret::new(initial, version),
                pending: None,
                audit: Vec::new(),
            })),
        }
    }

    pub async fn current(&self) -> ManagedSeedSecret {
        self.inner.lock().await.current.clone()
    }

    pub async fn audit(&self) -> Vec<String> {
        self.inner.lock().await.audit.clone()
    }
}

#[async_trait]
impl SecretManagerBackend for MemorySecretManager {
    async fn read_current(&self) -> Result<ManagedSeedSecret> {
        Ok(self.current().await)
    }

    async fn put_candidate(
        &self,
        candidate: &ValidatorSeedMaterial,
        _previous: &ManagedSeedSecret,
    ) -> Result<ManagedSeedSecret> {
        let mut inner = self.inner.lock().await;
        let version = SecretVersion::new(
            format!("candidate-{}", inner.audit.len() + 1),
            candidate.fingerprint.clone(),
        );
        let candidate = ManagedSeedSecret::new(candidate.clone(), version);
        inner.pending = Some(candidate.clone());
        inner
            .audit
            .push(format!("candidate:{}", candidate.version.fingerprint));
        Ok(candidate)
    }

    async fn promote_candidate(
        &self,
        candidate: &ManagedSeedSecret,
        _previous: &ManagedSeedSecret,
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.current = candidate.clone();
        inner.pending = None;
        inner
            .audit
            .push(format!("promote:{}", candidate.version.fingerprint));
        Ok(())
    }

    async fn rollback(
        &self,
        previous: &ManagedSeedSecret,
        failed_candidate: Option<&ManagedSeedSecret>,
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.current = previous.clone();
        inner.pending = None;
        let failed = failed_candidate
            .map(|c| c.version.fingerprint.as_str())
            .unwrap_or("none");
        inner.audit.push(format!(
            "rollback:{}:failed={failed}",
            previous.version.fingerprint
        ));
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

fn parse_seed_material(secret_string: &str, seed_field: &str) -> Result<ValidatorSeedMaterial> {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(secret_string) {
        let seed = extract_seed(&map, seed_field)?;
        return ValidatorSeedMaterial::from_seed(seed);
    }

    ValidatorSeedMaterial::from_seed(secret_string.trim().to_string())
}

fn seed_document(
    seed_field: &str,
    material: &ValidatorSeedMaterial,
    state: &str,
    version_id: &str,
    previous_version: Option<&SecretVersion>,
) -> Value {
    let mut data = Map::new();
    data.insert(
        seed_field.to_string(),
        Value::String(material.secret_seed().to_string()),
    );
    data.insert(
        "publicKey".to_string(),
        Value::String(material.public_key.clone()),
    );
    data.insert(
        "fingerprint".to_string(),
        Value::String(material.fingerprint.clone()),
    );
    data.insert(
        "rotationState".to_string(),
        Value::String(state.to_string()),
    );
    data.insert(
        "rotationVersion".to_string(),
        Value::String(version_id.to_string()),
    );
    data.insert(
        "createdAt".to_string(),
        Value::String(Utc::now().to_rfc3339()),
    );
    if let Some(previous) = previous_version {
        data.insert(
            "previousVersion".to_string(),
            Value::String(previous.version_id.clone()),
        );
        data.insert(
            "previousFingerprint".to_string(),
            Value::String(previous.fingerprint.clone()),
        );
    }
    Value::Object(data)
}

fn vault_data_map(body: &Value) -> Option<&Map<String, Value>> {
    body.pointer("/data/data")
        .and_then(Value::as_object)
        .or_else(|| body.get("data").and_then(Value::as_object))
}

fn extract_seed(data: &Map<String, Value>, seed_field: &str) -> Result<String> {
    data.get(seed_field)
        .or_else(|| data.get("seed"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::ConfigError(format!(
                "secret payload does not contain non-empty field '{seed_field}'"
            ))
        })
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

fn seed_fingerprint(seed: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hex::encode(hasher.finalize())
}

fn encode_strkey(version_byte: u8, payload: &[u8; STRKEY_PAYLOAD_LEN]) -> String {
    let mut bytes = Vec::with_capacity(STRKEY_PAYLOAD_LEN + 3);
    bytes.push(version_byte);
    bytes.extend_from_slice(payload);
    let checksum = crc16_xmodem(&bytes).to_le_bytes();
    bytes.extend_from_slice(&checksum);
    base32_encode(&bytes)
}

fn decode_strkey(strkey: &str, expected_version: u8) -> Result<Vec<u8>> {
    let decoded = base32_decode(strkey.trim())?;
    if decoded.len() != STRKEY_PAYLOAD_LEN + 3 {
        return Err(Error::ValidationError(format!(
            "invalid StrKey length: expected {} bytes, got {}",
            STRKEY_PAYLOAD_LEN + 3,
            decoded.len()
        )));
    }
    if decoded[0] != expected_version {
        return Err(Error::ValidationError(
            "invalid StrKey version byte".to_string(),
        ));
    }

    let checksum_index = decoded.len() - 2;
    let expected_checksum =
        u16::from_le_bytes([decoded[checksum_index], decoded[checksum_index + 1]]);
    let actual_checksum = crc16_xmodem(&decoded[..checksum_index]);
    if actual_checksum != expected_checksum {
        return Err(Error::ValidationError(
            "invalid StrKey checksum".to_string(),
        ));
    }

    Ok(decoded[1..checksum_index].to_vec())
}

fn base32_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0u16;
    let mut bits = 0u8;

    for byte in bytes {
        buffer = (buffer << 8) | (*byte as u16);
        bits += 8;
        while bits >= 5 {
            let index = ((buffer >> (bits - 5)) & 0x1f) as usize;
            output.push(BASE32_ALPHABET[index] as char);
            bits -= 5;
        }
    }

    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        output.push(BASE32_ALPHABET[index] as char);
    }

    output
}

fn base32_decode(input: &str) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer = 0u32;
    let mut bits = 0u8;

    for ch in input.chars() {
        let value = base32_value(ch).ok_or_else(|| {
            Error::ValidationError(format!("invalid base32 character '{ch}' in StrKey"))
        })?;
        buffer = (buffer << 5) | (value as u32);
        bits += 5;
        if bits >= 8 {
            output.push(((buffer >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }

    Ok(output)
}

fn base32_value(ch: char) -> Option<u8> {
    match ch {
        'A'..='Z' => Some(ch as u8 - b'A'),
        '2'..='7' => Some(ch as u8 - b'2' + 26),
        _ => None,
    }
}

fn crc16_xmodem(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_seed_is_valid_stellar_strkey() {
        let material = ValidatorSeedMaterial::generate().unwrap();

        assert!(material.secret_seed().starts_with('S'));
        assert_eq!(material.secret_seed().len(), 56);
        assert!(material.public_key.starts_with('G'));
        assert_eq!(material.public_key.len(), 56);

        let reparsed = ValidatorSeedMaterial::from_seed(material.secret_seed()).unwrap();
        assert_eq!(reparsed.public_key, material.public_key);
        assert_eq!(reparsed.fingerprint, material.fingerprint);
    }

    #[test]
    fn debug_redacts_secret_seed() {
        let material = ValidatorSeedMaterial::generate().unwrap();
        let seed = material.secret_seed().to_string();
        let rendered = format!("{material:?}");

        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains(&seed));
    }

    #[test]
    fn parse_seed_from_json_or_raw_value() {
        let material = ValidatorSeedMaterial::generate().unwrap();
        let json = serde_json::json!({
            "seed": material.secret_seed(),
            "rotationState": "active"
        })
        .to_string();

        assert_eq!(
            parse_seed_material(&json, "seed").unwrap().public_key,
            material.public_key
        );
        assert_eq!(
            parse_seed_material(material.secret_seed(), "seed")
                .unwrap()
                .public_key,
            material.public_key
        );
    }

    #[tokio::test]
    async fn memory_backend_promotes_and_rolls_back_without_logging_seed() {
        let initial = ValidatorSeedMaterial::generate().unwrap();
        let candidate = ValidatorSeedMaterial::generate().unwrap();
        let seed = candidate.secret_seed().to_string();
        let backend = MemorySecretManager::new(initial.clone());

        let previous = backend.read_current().await.unwrap();
        let pending = backend.put_candidate(&candidate, &previous).await.unwrap();
        backend
            .promote_candidate(&pending, &previous)
            .await
            .unwrap();
        assert_eq!(
            backend.current().await.material.public_key,
            candidate.public_key
        );

        backend.rollback(&previous, Some(&pending)).await.unwrap();
        assert_eq!(
            backend.current().await.material.public_key,
            initial.public_key
        );
        assert!(!backend.audit().await.join(" ").contains(&seed));
    }
}
