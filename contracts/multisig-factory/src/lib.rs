//! # Multi-Signature Wallet Factory
//!
//! A Soroban smart contract that deploys independent multi-signature wallet
//! instances for DAO treasuries and institutional custodians.
//!
//! ## Responsibilities
//!
//! - Accept a WASM blob (the compiled `MultisigWallet` contract) and register
//!   its hash for subsequent deployments.
//! - Deploy unique wallet instances via Soroban's deployer interface, each
//!   seeded with its own signer set and signing threshold.
//! - Record every deployed wallet address, indexed by a caller-provided salt,
//!   so the factory is the canonical registry of wallet addresses.
//!
//! ## Security invariants
//!
//! - Only the factory admin may register new wallet WASM.
//! - Each salt-based deployment is idempotent: re-deploying with the same salt
//!   will panic with `AlreadyDeployed` rather than silently overwriting.
//! - The factory holds no signing authority over deployed wallets; each wallet
//!   enforces its own signer/threshold policy independently.
//!
//! ## Storage layout
//!
//! | Key | Type | TTL |
//! |-----|------|-----|
//! | `Admin` | `Address` | Persistent |
//! | `WalletWasmHash` | `BytesN<32>` | Persistent |
//! | `DeployedWallet(salt)` | `Address` | Persistent |
//! | `WalletCount` | `u64` | Persistent |

#![no_std]

mod wallet;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    Address, BytesN, Env, Vec,
};

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// All error conditions surfaced by the factory contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not yet been initialized.
    NotInitialized = 2,
    /// Caller is not the factory admin.
    NotAdmin = 3,
    /// No wallet WASM hash has been registered yet.
    WasmNotRegistered = 4,
    /// A wallet with this salt has already been deployed.
    AlreadyDeployed = 5,
    /// The provided signer list is empty.
    EmptySigners = 6,
    /// The threshold must be ≥ 1 and ≤ len(signers).
    InvalidThreshold = 7,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Factory administrator address.
    Admin,
    /// Content hash of the wallet WASM registered by the admin.
    WalletWasmHash,
    /// Mapping from deployment salt → deployed wallet contract address.
    DeployedWallet(BytesN<32>),
    /// Total number of wallets deployed through this factory.
    WalletCount,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct MultisigFactory;

#[contractimpl]
impl MultisigFactory {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the factory.
    ///
    /// Must be called exactly once by the deployer of this factory contract.
    ///
    /// * `admin` — address that controls factory-level operations (WASM
    ///   registration, admin rotation).
    pub fn initialize(env: Env, admin: Address) -> Result<(), FactoryError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(FactoryError::AlreadyInitialized);
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::WalletCount, &0u64);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // WASM registration
    // -----------------------------------------------------------------------

    /// Register (or update) the wallet WASM hash to use for future deployments.
    ///
    /// Only the factory admin may call this.  The `wasm_hash` must refer to a
    /// WASM blob already uploaded to the ledger via `env.deployer().upload_contract_wasm`.
    ///
    /// Updating the hash does **not** affect already-deployed wallets.
    pub fn register_wallet_wasm(
        env: Env,
        wasm_hash: BytesN<32>,
    ) -> Result<(), FactoryError> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::WalletWasmHash, &wasm_hash);

        env.events().publish(
            (soroban_sdk::symbol_short!("wasm_reg"),),
            wasm_hash,
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Wallet deployment
    // -----------------------------------------------------------------------

    /// Deploy a new multi-signature wallet instance.
    ///
    /// * `salt`      — 32-byte unique salt; two calls with the same salt will
    ///                 panic with `AlreadyDeployed`. Use the first deployment
    ///                 address returned by this function to reference the wallet.
    /// * `signers`   — Initial set of authorised signers. Must be non-empty.
    /// * `threshold` — Minimum number of signer approvals required to execute a
    ///                 proposal. Must satisfy `1 ≤ threshold ≤ len(signers)`.
    ///
    /// Returns the `Address` of the freshly deployed wallet contract.
    pub fn deploy_wallet(
        env: Env,
        salt: BytesN<32>,
        signers: Vec<Address>,
        threshold: u32,
    ) -> Result<Address, FactoryError> {
        // Guard: factory must be initialised.
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(FactoryError::NotInitialized);
        }

        // Guard: WASM must be registered.
        let wasm_hash: BytesN<32> = env
            .storage()
            .instance()
            .get(&DataKey::WalletWasmHash)
            .ok_or(FactoryError::WasmNotRegistered)?;

        // Guard: no duplicate deployments for the same salt.
        if env.storage().persistent().has(&DataKey::DeployedWallet(salt.clone())) {
            return Err(FactoryError::AlreadyDeployed);
        }

        // Validate signer set and threshold.
        if signers.is_empty() {
            return Err(FactoryError::EmptySigners);
        }
        let n = signers.len();
        if threshold == 0 || threshold > n {
            return Err(FactoryError::InvalidThreshold);
        }

        // Deploy the wallet contract.
        let wallet_address = env
            .deployer()
            .with_current_contract(salt.clone())
            .deploy_v2(wasm_hash, (signers.clone(), threshold));

        // Record the deployment.
        env.storage()
            .persistent()
            .set(&DataKey::DeployedWallet(salt.clone()), &wallet_address);

        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::WalletCount)
            .unwrap_or(0u64);
        env.storage()
            .instance()
            .set(&DataKey::WalletCount, &(count + 1));

        env.events().publish(
            (soroban_sdk::symbol_short!("deployed"),),
            (salt, wallet_address.clone(), signers, threshold),
        );

        Ok(wallet_address)
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Return the address of a previously deployed wallet identified by `salt`.
    pub fn get_wallet(env: Env, salt: BytesN<32>) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::DeployedWallet(salt))
    }

    /// Return the total number of wallets deployed through this factory.
    pub fn wallet_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::WalletCount)
            .unwrap_or(0u64)
    }

    /// Return the current factory admin.
    pub fn admin(env: Env) -> Result<Address, FactoryError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)
    }

    /// Return the registered wallet WASM hash, if any.
    pub fn wallet_wasm_hash(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::WalletWasmHash)
    }

    // -----------------------------------------------------------------------
    // Admin rotation
    // -----------------------------------------------------------------------

    /// Transfer the factory admin role to a new address.
    pub fn rotate_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        let current_admin = Self::require_admin(&env)?;
        current_admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);

        env.events().publish(
            (soroban_sdk::symbol_short!("adm_rot"),),
            new_admin,
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn require_admin(env: &Env) -> Result<Address, FactoryError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)
    }
}
