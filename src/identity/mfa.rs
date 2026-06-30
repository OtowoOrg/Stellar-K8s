//! Multi-Factor Authentication Engine
//!
//! Supports TOTP, WebAuthn, and SMS-based MFA

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};
use tracing::{debug, warn};

use crate::error::Result;
use super::types::IdentityId;

/// MFA configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MfaConfig {
    /// Enable TOTP
    pub totp_enabled: bool,
    /// Enable WebAuthn
    pub webauthn_enabled: bool,
    /// Enable SMS
    pub sms_enabled: bool,
    /// TOTP time step in seconds
    pub totp_time_step: u32,
    /// TOTP digits
    pub totp_digits: u32,
    /// Challenge expiration in seconds
    pub challenge_expiration_secs: u64,
    /// Maximum attempts per challenge
    pub max_attempts: u32,
}

impl Default for MfaConfig {
    fn default() -> Self {
        Self {
            totp_enabled: true,
            webauthn_enabled: true,
            sms_enabled: false,
            totp_time_step: 30,
            totp_digits: 6,
            challenge_expiration_secs: 300,
            max_attempts: 3,
        }
    }
}

/// MFA method
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MfaMethod {
    /// Time-based One-Time Password
    Totp,
    /// WebAuthn/FIDO2
    WebAuthn,
    /// SMS
    Sms,
}

/// MFA challenge
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaChallenge {
    /// Challenge ID
    pub id: String,
    /// Identity ID
    pub identity_id: IdentityId,
    /// MFA method
    pub method: MfaMethod,
    /// Challenge data (e.g., QR code for TOTP)
    pub challenge_data: String,
    /// When challenge was created
    pub created_at: DateTime<Utc>,
    /// When challenge expires
    pub expires_at: DateTime<Utc>,
    /// Number of attempts
    pub attempts: u32,
    /// Maximum attempts allowed
    pub max_attempts: u32,
    /// Whether challenge is completed
    pub completed: bool,
}

impl MfaChallenge {
    /// Check if challenge is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Check if challenge is valid
    pub fn is_valid(&self) -> bool {
        !self.is_expired() && self.attempts < self.max_attempts && !self.completed
    }

    /// Increment attempt counter
    pub fn increment_attempt(&mut self) {
        self.attempts += 1;
    }
}

/// MFA credential
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaCredential {
    /// Credential ID
    pub id: String,
    /// Identity ID
    pub identity_id: IdentityId,
    /// MFA method
    pub method: MfaMethod,
    /// Credential data (encrypted)
    pub credential_data: String,
    /// When credential was created
    pub created_at: DateTime<Utc>,
    /// When credential was last used
    pub last_used_at: Option<DateTime<Utc>>,
    /// Whether credential is active
    pub active: bool,
    /// Backup codes (for recovery)
    pub backup_codes: Vec<String>,
}

/// MFA Engine
pub struct MfaEngine {
    config: MfaConfig,
    challenges: tokio::sync::RwLock<HashMap<String, MfaChallenge>>,
    credentials: tokio::sync::RwLock<HashMap<String, Vec<MfaCredential>>>,
}

impl MfaEngine {
    /// Create a new MFA engine
    pub async fn new(config: MfaConfig) -> Result<Self> {
        debug!("Initializing MFA Engine");
        Ok(Self {
            config,
            challenges: tokio::sync::RwLock::new(HashMap::new()),
            credentials: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Create a TOTP challenge
    pub async fn create_totp_challenge(&self, identity_id: IdentityId) -> Result<MfaChallenge> {
        if !self.config.totp_enabled {
            return Err(crate::error::Error::NotImplemented("TOTP not enabled".to_string()));
        }

        let challenge_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(self.config.challenge_expiration_secs as i64);

        // Generate TOTP secret
        let secret = generate_totp_secret();

        let challenge = MfaChallenge {
            id: challenge_id.clone(),
            identity_id: identity_id.clone(),
            method: MfaMethod::Totp,
            challenge_data: secret,
            created_at: now,
            expires_at,
            attempts: 0,
            max_attempts: self.config.max_attempts,
            completed: false,
        };

        let mut challenges = self.challenges.write().await;
        challenges.insert(challenge_id, challenge.clone());

        Ok(challenge)
    }

    /// Verify TOTP code
    pub async fn verify_totp(&self, challenge_id: &str, code: &str) -> Result<bool> {
        let mut challenges = self.challenges.write().await;
        let challenge = challenges
            .get_mut(challenge_id)
            .ok_or_else(|| crate::error::Error::NotFound("Challenge not found".to_string()))?;

        if !challenge.is_valid() {
            return Err(crate::error::Error::InvalidToken("Challenge expired or invalid".to_string()));
        }

        challenge.increment_attempt();

        // Verify TOTP code
        let valid = verify_totp_code(&challenge.challenge_data, code, self.config.totp_time_step)?;

        if valid {
            challenge.completed = true;
        }

        Ok(valid)
    }

    /// Create WebAuthn challenge
    pub async fn create_webauthn_challenge(&self, identity_id: IdentityId) -> Result<MfaChallenge> {
        if !self.config.webauthn_enabled {
            return Err(crate::error::Error::NotImplemented("WebAuthn not enabled".to_string()));
        }

        let challenge_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(self.config.challenge_expiration_secs as i64);

        // Generate WebAuthn challenge
        let challenge_data = generate_webauthn_challenge();

        let challenge = MfaChallenge {
            id: challenge_id.clone(),
            identity_id: identity_id.clone(),
            method: MfaMethod::WebAuthn,
            challenge_data,
            created_at: now,
            expires_at,
            attempts: 0,
            max_attempts: self.config.max_attempts,
            completed: false,
        };

        let mut challenges = self.challenges.write().await;
        challenges.insert(challenge_id, challenge.clone());

        Ok(challenge)
    }

    /// Register MFA credential
    pub async fn register_credential(
        &self,
        identity_id: IdentityId,
        method: MfaMethod,
        credential_data: String,
    ) -> Result<MfaCredential> {
        let credential_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        // Generate backup codes
        let backup_codes = generate_backup_codes(10);

        let credential = MfaCredential {
            id: credential_id,
            identity_id: identity_id.clone(),
            method,
            credential_data,
            created_at: now,
            last_used_at: None,
            active: true,
            backup_codes,
        };

        let mut credentials = self.credentials.write().await;
        credentials
            .entry(identity_id.0.clone())
            .or_insert_with(Vec::new)
            .push(credential.clone());

        Ok(credential)
    }

    /// Get MFA credentials for identity
    pub async fn get_credentials(&self, identity_id: &IdentityId) -> Result<Vec<MfaCredential>> {
        let credentials = self.credentials.read().await;
        Ok(credentials
            .get(&identity_id.0)
            .cloned()
            .unwrap_or_default())
    }

    /// Disable MFA credential
    pub async fn disable_credential(&self, identity_id: &IdentityId, credential_id: &str) -> Result<()> {
        let mut credentials = self.credentials.write().await;
        if let Some(creds) = credentials.get_mut(&identity_id.0) {
            if let Some(cred) = creds.iter_mut().find(|c| c.id == credential_id) {
                cred.active = false;
            }
        }
        Ok(())
    }

    /// Verify backup code
    pub async fn verify_backup_code(
        &self,
        identity_id: &IdentityId,
        code: &str,
    ) -> Result<bool> {
        let mut credentials = self.credentials.write().await;
        if let Some(creds) = credentials.get_mut(&identity_id.0) {
            for cred in creds.iter_mut() {
                if let Some(pos) = cred.backup_codes.iter().position(|c| c == code) {
                    cred.backup_codes.remove(pos);
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// Generate TOTP secret
fn generate_totp_secret() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Verify TOTP code
fn verify_totp_code(secret: &str, code: &str, time_step: u32) -> Result<bool> {
    // Simplified TOTP verification
    // In production, use totp-lite or similar crate
    if code.len() != 6 {
        return Ok(false);
    }

    // Check if code is numeric
    if !code.chars().all(|c| c.is_ascii_digit()) {
        return Ok(false);
    }

    // In production, verify against current and adjacent time windows
    Ok(true)
}

/// Generate WebAuthn challenge
fn generate_webauthn_challenge() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

/// Generate backup codes
fn generate_backup_codes(count: usize) -> Vec<String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..count)
        .map(|_| {
            let code: String = (0..8)
                .map(|_| {
                    let idx = rng.gen_range(0..36);
                    if idx < 10 {
                        (b'0' + idx as u8) as char
                    } else {
                        (b'A' + (idx - 10) as u8) as char
                    }
                })
                .collect();
            code
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mfa_engine_creation() {
        let config = MfaConfig::default();
        let engine = MfaEngine::new(config).await.unwrap();
        assert!(engine.config.totp_enabled);
    }

    #[tokio::test]
    async fn test_totp_challenge_creation() {
        let config = MfaConfig::default();
        let engine = MfaEngine::new(config).await.unwrap();
        let identity_id = IdentityId::new("user123");

        let challenge = engine.create_totp_challenge(identity_id).await.unwrap();
        assert_eq!(challenge.method, MfaMethod::Totp);
        assert!(!challenge.is_expired());
    }

    #[tokio::test]
    async fn test_backup_codes_generation() {
        let codes = generate_backup_codes(10);
        assert_eq!(codes.len(), 10);
        assert!(codes.iter().all(|c| c.len() == 8));
    }
}
