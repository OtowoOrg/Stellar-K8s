//! Security automation for validator identity material and certificate support.

pub mod ca;
pub mod mtls;
pub mod rotation;
pub mod vault;

pub use rotation::{
    ConsensusSnapshot, HttpStellarCoreAdmin, KeyRotationDaemon, KeyRotationReport,
    KeyRotationWorker, RotationLogEntry, RotationOutcome, RotationStage, SeedGenerator,
    StellarCoreAdmin, StellarSeedGenerator, ValidatorKeyRotationConfig,
};
pub use vault::{
    AwsSecretsManagerBackend, AwsSecretsManagerConfig, ManagedSeedSecret, MemorySecretManager,
    SecretManagerBackend, SecretVersion, ValidatorSeedMaterial, VaultKv2Backend, VaultKv2Config,
};

pub fn rotate() -> Result<(), String> {
    Ok(())
}
