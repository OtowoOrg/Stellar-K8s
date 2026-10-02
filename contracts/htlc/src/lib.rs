//! # Cross-Chain Hash Time-Locked Contract (HTLC) — Soroban / Stellar
//!
//! Implements a trustless atomic-swap vault for Stellar-native assets (XLM
//! and SAC tokens).  The contract is designed for cross-chain swaps between
//! Stellar and external networks (Bitcoin, Ethereum, etc.) where the
//! counterparty is expected to reveal a cryptographic pre-image to claim
//! locked funds.
//!
//! ## Protocol overview
//!
//! ```text
//!  Initiator (Alice)            Contract            Responder (Bob)
//!       │  lock(hash, ttl)  ───────▶ │                    │
//!       │                             │ ◀─── claim(preimage)  │
//!       │                             │ transfer to Bob        │
//!       │                             │                    │
//!       │  (or ttl elapsed)           │                    │
//!       │  refund() ──────────────────▶ │                    │
//!       │  transfer back to Alice     │                    │
//! ```
//!
//! ## Design decisions
//!
//! - **SHA-256 only**: the hash function is the Soroban host's native `sha256`
//!   built-in.  Using a host function rather than a pure-WASM SHA-256
//!   implementation is 10–50× cheaper on multi-dimensional fee metering.
//! - **Persistent storage keyed on `hashlock`**: each HTLC is addressed by its
//!   32-byte SHA-256 digest.  Duplicate hashlocks are rejected, which prevents
//!   replay attacks.
//! - **Ledger sequence TTL**: the expiration is expressed as an absolute
//!   *ledger sequence number* (not a Unix timestamp) to avoid dependence on
//!   the validator-set clock drift.
//! - **No floating-point maths**: all amounts are `i128` stroops.
//! - **No re-entrancy surface**: no callbacks or cross-contract invocations
//!   are made after the state is finalised.
//!
//! ## Hash-collision security
//!
//! See [`crypto`] for a full analysis.  In summary: finding a second pre-image
//! for a 256-bit digest requires ≈ 2^256 work.  The birthday bound for two
//! simultaneous collisions is ≈ 2^128 — well beyond practical attack budgets.

#![no_std]

pub mod crypto;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Bytes,
    BytesN, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Error catalogue
// ---------------------------------------------------------------------------

/// All terminal error conditions surfaced by the HTLC contract.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtlcError {
    /// The hashlock is already bound to an existing HTLC.  Re-using the same
    /// hash for two concurrent swaps is rejected to prevent replay attacks.
    HashlockAlreadyExists = 1,
    /// No HTLC entry was found for the supplied hashlock.
    EscrowNotFound = 2,
    /// The HTLC has already been settled (claimed or refunded).
    AlreadySettled = 3,
    /// The caller is not the designated receiver of this HTLC.
    UnauthorizedReceiver = 4,
    /// The caller is not the original sender / initiator of this HTLC.
    UnauthorizedSender = 5,
    /// `claim` was called but the absolute ledger-sequence TTL has already
    /// elapsed.  The initiator may now call `refund`.
    TimelockExpired = 6,
    /// `refund` was called but the ledger-sequence TTL has not yet elapsed.
    TimelockNotExpired = 7,
    /// The supplied pre-image does not SHA-256 hash to the stored hashlock.
    InvalidPreimage = 8,
    /// The requested lock `amount` must be positive (> 0).
    InvalidAmount = 9,
    /// The `expiry_ledger` must be strictly greater than the current ledger
    /// sequence when `lock` is invoked.
    InvalidExpiry = 10,
}

// ---------------------------------------------------------------------------
// Storage keys & data types
// ---------------------------------------------------------------------------

/// Persistent-storage discriminant.  Each HTLC is keyed on its 32-byte
/// `hashlock` digest.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Maps a 32-byte `hashlock → EscrowEntry`.
    Htlc(BytesN<32>),
}

/// Lifecycle state of a single HTLC.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    /// Funds are locked; the responder may `claim` before `expiry_ledger`.
    Active = 0,
    /// Pre-image was supplied and verified; funds transferred to `receiver`.
    Claimed = 1,
    /// TTL elapsed without a claim; funds returned to `sender`.
    Refunded = 2,
}

/// On-chain HTLC record stored in `DataKey::Htlc(hashlock)`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowEntry {
    /// Original initiator who locked the funds.
    pub sender: Address,
    /// Intended beneficiary who must reveal the pre-image to claim.
    pub receiver: Address,
    /// SAC token contract address (or native XLM wrapper).
    pub token: Address,
    /// Locked amount in the token's base units (stroops for XLM).
    pub amount: i128,
    /// SHA-256 digest of the secret pre-image.
    pub hashlock: BytesN<32>,
    /// Absolute ledger sequence after which the initiator may call `refund`.
    /// Expressed as `u32` matching `ledger().sequence()`.
    pub expiry_ledger: u32,
    /// Current settlement status.
    pub status: EscrowStatus,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Event topics emitted on successful HTLC transitions.
const TOPIC_LOCK: Symbol = symbol_short!("htlc_lock");
const TOPIC_CLAIM: Symbol = symbol_short!("htlc_clm");
const TOPIC_REFUND: Symbol = symbol_short!("htlc_ref");

#[contract]
pub struct HtlcContract;

#[contractimpl]
impl HtlcContract {
    // -----------------------------------------------------------------------
    // lock
    // -----------------------------------------------------------------------

    /// Lock `amount` tokens from `sender` against a SHA-256 `hashlock`.
    ///
    /// The funds are held by the contract until either:
    /// - the `receiver` calls [`claim`] with a valid pre-image before
    ///   `expiry_ledger`, or
    /// - the `sender` calls [`refund`] once `expiry_ledger` has passed.
    ///
    /// # Arguments
    /// * `sender`        – Address that owns the tokens and initiates the swap.
    ///                     Must authorise this call.
    /// * `receiver`      – Counterparty that will receive the funds upon a
    ///                     successful reveal.
    /// * `token`         – SAC token contract (or XLM native wrapper).
    /// * `amount`        – Positive number of token base units to escrow.
    /// * `hashlock`      – SHA-256 digest of the secret pre-image.
    /// * `expiry_ledger` – Absolute ledger sequence at which the timelock
    ///                     expires.  Must be strictly greater than the current
    ///                     ledger sequence at the time of invocation.
    ///
    /// # Errors
    /// * [`HtlcError::InvalidAmount`]         – `amount ≤ 0`.
    /// * [`HtlcError::InvalidExpiry`]         – `expiry_ledger ≤ current`.
    /// * [`HtlcError::HashlockAlreadyExists`] – duplicate hashlock.
    pub fn lock(
        env: Env,
        sender: Address,
        receiver: Address,
        token: Address,
        amount: i128,
        hashlock: BytesN<32>,
        expiry_ledger: u32,
    ) -> Result<(), HtlcError> {
        sender.require_auth();

        // Validate amount.
        if amount <= 0 {
            return Err(HtlcError::InvalidAmount);
        }

        // Validate expiry is in the future.
        let current_seq = env.ledger().sequence();
        if expiry_ledger <= current_seq {
            return Err(HtlcError::InvalidExpiry);
        }

        // Reject duplicate hashlocks.
        let key = DataKey::Htlc(hashlock.clone());
        if env.storage().persistent().has(&key) {
            return Err(HtlcError::HashlockAlreadyExists);
        }

        // Pull tokens from sender into the contract's own address.
        let tok = token::Client::new(&env, &token);
        tok.transfer(&sender, &env.current_contract_address(), &amount);

        // Persist the HTLC record.
        let entry = EscrowEntry {
            sender: sender.clone(),
            receiver: receiver.clone(),
            token,
            amount,
            hashlock: hashlock.clone(),
            expiry_ledger,
            status: EscrowStatus::Active,
        };
        env.storage().persistent().set(&key, &entry);

        // Emit lock event.
        env.events().publish(
            (TOPIC_LOCK, hashlock),
            (sender, receiver, amount, expiry_ledger),
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // claim
    // -----------------------------------------------------------------------

    /// Release escrowed funds to `receiver` by revealing the `preimage`.
    ///
    /// The contract hashes `preimage` with SHA-256 and compares the result
    /// with the stored `hashlock`.  On success the funds are transferred to
    /// `receiver` and the HTLC is marked [`EscrowStatus::Claimed`].
    ///
    /// # Arguments
    /// * `receiver` – Must match the `receiver` stored in the HTLC.
    ///                Must authorise this call.
    /// * `hashlock` – Identifies the specific HTLC to claim.
    /// * `preimage` – Raw byte string whose SHA-256 matches `hashlock`.
    ///
    /// # Errors
    /// * [`HtlcError::EscrowNotFound`]     – no HTLC for `hashlock`.
    /// * [`HtlcError::AlreadySettled`]     – HTLC already claimed or refunded.
    /// * [`HtlcError::UnauthorizedReceiver`] – caller ≠ stored receiver.
    /// * [`HtlcError::TimelockExpired`]    – current ledger ≥ `expiry_ledger`.
    /// * [`HtlcError::InvalidPreimage`]    – sha256(preimage) ≠ hashlock.
    pub fn claim(
        env: Env,
        receiver: Address,
        hashlock: BytesN<32>,
        preimage: Bytes,
    ) -> Result<(), HtlcError> {
        receiver.require_auth();

        let key = DataKey::Htlc(hashlock.clone());
        let mut entry: EscrowEntry = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(HtlcError::EscrowNotFound)?;

        // Guard: not yet settled.
        if entry.status != EscrowStatus::Active {
            return Err(HtlcError::AlreadySettled);
        }

        // Guard: correct receiver.
        if entry.receiver != receiver {
            return Err(HtlcError::UnauthorizedReceiver);
        }

        // Guard: within the timelock window.
        let current_seq = env.ledger().sequence();
        if current_seq >= entry.expiry_ledger {
            return Err(HtlcError::TimelockExpired);
        }

        // Guard: pre-image is valid.
        if !crypto::verify_preimage(&env, &preimage, &entry.hashlock) {
            return Err(HtlcError::InvalidPreimage);
        }

        // Mark as claimed before any external calls (reentrancy safety).
        entry.status = EscrowStatus::Claimed;
        env.storage().persistent().set(&key, &entry);

        // Release funds to the receiver.
        let tok = token::Client::new(&env, &entry.token);
        tok.transfer(&env.current_contract_address(), &receiver, &entry.amount);

        // Emit claim event.
        env.events().publish((TOPIC_CLAIM, hashlock), (receiver, entry.amount));

        Ok(())
    }

    // -----------------------------------------------------------------------
    // refund
    // -----------------------------------------------------------------------

    /// Return escrowed funds to the `sender` after the timelock has expired.
    ///
    /// May only be called once `current_ledger_sequence ≥ expiry_ledger`.
    /// On success the HTLC is marked [`EscrowStatus::Refunded`] and the
    /// funds are transferred back to the original initiator.
    ///
    /// # Arguments
    /// * `sender`   – Must match the `sender` stored in the HTLC.
    ///                Must authorise this call.
    /// * `hashlock` – Identifies the specific HTLC to refund.
    ///
    /// # Errors
    /// * [`HtlcError::EscrowNotFound`]   – no HTLC for `hashlock`.
    /// * [`HtlcError::AlreadySettled`]   – HTLC already claimed or refunded.
    /// * [`HtlcError::UnauthorizedSender`] – caller ≠ stored sender.
    /// * [`HtlcError::TimelockNotExpired`] – `expiry_ledger` not yet reached.
    pub fn refund(
        env: Env,
        sender: Address,
        hashlock: BytesN<32>,
    ) -> Result<(), HtlcError> {
        sender.require_auth();

        let key = DataKey::Htlc(hashlock.clone());
        let mut entry: EscrowEntry = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(HtlcError::EscrowNotFound)?;

        // Guard: not yet settled.
        if entry.status != EscrowStatus::Active {
            return Err(HtlcError::AlreadySettled);
        }

        // Guard: correct sender / initiator.
        if entry.sender != sender {
            return Err(HtlcError::UnauthorizedSender);
        }

        // Guard: timelock window has closed.
        let current_seq = env.ledger().sequence();
        if current_seq < entry.expiry_ledger {
            return Err(HtlcError::TimelockNotExpired);
        }

        // Mark as refunded before any external calls (reentrancy safety).
        entry.status = EscrowStatus::Refunded;
        env.storage().persistent().set(&key, &entry);

        // Return funds to the initiator.
        let tok = token::Client::new(&env, &entry.token);
        tok.transfer(&env.current_contract_address(), &sender, &entry.amount);

        // Emit refund event.
        env.events().publish((TOPIC_REFUND, hashlock), (sender, entry.amount));

        Ok(())
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

    /// Retrieve the current [`EscrowEntry`] for a given `hashlock`.
    /// Returns `None` if no HTLC exists for that key.
    pub fn get_htlc(env: Env, hashlock: BytesN<32>) -> Option<EscrowEntry> {
        env.storage()
            .persistent()
            .get(&DataKey::Htlc(hashlock))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Comprehensive unit tests exercising the happy path, every unhappy-path
    //! branch, and asset-distribution correctness for both scenarios.
    //!
    //! Test structure:
    //!   - `test_happy_path_*`    — the receiver reveals the correct pre-image.
    //!   - `test_refund_*`        — the initiator reclaims after TTL expiry.
    //!   - `test_error_*`         — every `HtlcError` variant has at least one
    //!                              dedicated negative test.

    extern crate std;

    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        token::{self, StellarAssetClient},
        Address, Bytes, BytesN, Env,
    };

    use super::*;
    use crate::{HtlcContract, HtlcContractClient};

    // -----------------------------------------------------------------------
    // Test-environment helpers
    // -----------------------------------------------------------------------

    /// Raw 32-byte pre-image used throughout the tests.
    const PREIMAGE_BYTES: &[u8; 32] = b"test-preimage-exactly-32-bytes!!";

    /// Create a fresh `Env`, register the contract, and return the client
    /// together with two test addresses (initiator + responder) and a SAC
    /// token contract with initial balances.
    fn setup() -> (Env, HtlcContractClient<'static>, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        // soroban-sdk ≥ 22: use env.register(Contract, constructor_args) instead of
        // the deprecated env.register_contract(None, Contract).
        let contract_id = env.register(HtlcContract, ());
        let client = HtlcContractClient::new(&env, &contract_id);

        let sender: Address = Address::generate(&env);
        let receiver: Address = Address::generate(&env);

        // Deploy a Stellar Asset Contract and mint tokens to the sender.
        let token_admin: Address = Address::generate(&env);
        let sac_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr = sac_id.address();
        StellarAssetClient::new(&env, &token_addr).mint(&sender, &10_000_000_i128);

        (env, client, sender, receiver, token_addr)
    }

    /// Compute the SHA-256 of `PREIMAGE_BYTES` using the Soroban host and
    /// return it as a `BytesN<32>`.
    fn preimage_hash(env: &Env) -> BytesN<32> {
        let raw = Bytes::from_slice(env, PREIMAGE_BYTES);
        env.crypto().sha256(&raw)
    }

    // -----------------------------------------------------------------------
    // Happy path: successful claim
    // -----------------------------------------------------------------------

    /// Full "golden path" swap: lock → claim with valid pre-image.
    #[test]
    fn test_happy_path_claim_transfers_funds_to_receiver() {
        let (env, client, sender, receiver, token) = setup();

        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        // Ledger sequence starts at 0; set expiry well in the future.
        env.ledger().set_sequence_number(100);
        let expiry_ledger: u32 = 200;
        let amount: i128 = 1_000_000;

        // --- Lock ---
        client
            .lock(
                &sender,
                &receiver,
                &token,
                &amount,
                &hashlock,
                &expiry_ledger,
            )
            .unwrap();

        // Sender balance must have decreased by `amount`.
        let tok = token::Client::new(&env, &token);
        assert_eq!(tok.balance(&sender), 10_000_000 - amount);
        // Contract holds the escrowed funds.
        assert_eq!(tok.balance(&client.address), amount);
        // Receiver has not yet received anything.
        assert_eq!(tok.balance(&receiver), 0);

        // HTLC is Active.
        let entry = client.get_htlc(&hashlock).unwrap();
        assert_eq!(entry.status, EscrowStatus::Active);

        // --- Claim (before TTL) ---
        env.ledger().set_sequence_number(150); // within window
        client.claim(&receiver, &hashlock, &preimage).unwrap();

        // Receiver now holds the exact escrowed amount.
        assert_eq!(tok.balance(&receiver), amount);
        // Contract balance is zero.
        assert_eq!(tok.balance(&client.address), 0);
        // Sender balance is unchanged post-claim.
        assert_eq!(tok.balance(&sender), 10_000_000 - amount);

        // HTLC is Claimed.
        let entry = client.get_htlc(&hashlock).unwrap();
        assert_eq!(entry.status, EscrowStatus::Claimed);
    }

    /// Verify that the full amount (no partial transfer) is moved to `receiver`.
    #[test]
    fn test_happy_path_exact_amount_received() {
        let (env, client, sender, receiver, token) = setup();

        env.ledger().set_sequence_number(10);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);
        let amount: i128 = 5_555_555;

        client
            .lock(&sender, &receiver, &token, &amount, &hashlock, &100)
            .unwrap();

        let tok = token::Client::new(&env, &token);
        let pre_receiver_balance = tok.balance(&receiver);

        env.ledger().set_sequence_number(50);
        client.claim(&receiver, &hashlock, &preimage).unwrap();

        assert_eq!(
            tok.balance(&receiver) - pre_receiver_balance,
            amount,
            "receiver must receive exactly the escrowed amount"
        );
    }

    // -----------------------------------------------------------------------
    // Happy path: refund after TTL expiry
    // -----------------------------------------------------------------------

    /// Full refund path: lock → TTL expires → refund returns all funds to sender.
    #[test]
    fn test_refund_after_expiry_returns_funds_to_sender() {
        let (env, client, sender, receiver, token) = setup();

        env.ledger().set_sequence_number(50);
        let hashlock = preimage_hash(&env);
        let amount: i128 = 2_500_000;
        let expiry_ledger: u32 = 100;

        client
            .lock(
                &sender,
                &receiver,
                &token,
                &amount,
                &hashlock,
                &expiry_ledger,
            )
            .unwrap();

        let tok = token::Client::new(&env, &token);
        let sender_balance_after_lock = tok.balance(&sender);
        // Contract is holding the funds.
        assert_eq!(tok.balance(&client.address), amount);

        // Advance past the TTL.
        env.ledger().set_sequence_number(expiry_ledger);

        client.refund(&sender, &hashlock).unwrap();

        // Sender gets every stroop back.
        assert_eq!(tok.balance(&sender), sender_balance_after_lock + amount);
        // Contract balance is zero.
        assert_eq!(tok.balance(&client.address), 0);
        // Receiver balance unaffected.
        assert_eq!(tok.balance(&receiver), 0);

        // HTLC is Refunded.
        let entry = client.get_htlc(&hashlock).unwrap();
        assert_eq!(entry.status, EscrowStatus::Refunded);
    }

    /// Refund works at exactly `expiry_ledger` (boundary condition).
    #[test]
    fn test_refund_at_exact_expiry_boundary() {
        let (env, client, sender, _receiver, token) = setup();

        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let expiry_ledger: u32 = 10;

        client
            .lock(&sender, &_receiver, &token, &1_000_000, &hashlock, &expiry_ledger)
            .unwrap();

        // Exactly at expiry — refund must succeed.
        env.ledger().set_sequence_number(expiry_ledger);
        let res = client.try_refund(&sender, &hashlock);
        assert!(res.is_ok(), "refund at exact expiry_ledger must succeed");
    }

    // -----------------------------------------------------------------------
    // Error: InvalidAmount
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_invalid_amount_zero() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);

        let res = client.try_lock(&sender, &receiver, &token, &0, &hashlock, &100);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::InvalidAmount);
    }

    #[test]
    fn test_error_invalid_amount_negative() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);

        let res = client.try_lock(&sender, &receiver, &token, &(-1), &hashlock, &100);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::InvalidAmount);
    }

    // -----------------------------------------------------------------------
    // Error: InvalidExpiry
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_expiry_equal_to_current_ledger() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(50);
        let hashlock = preimage_hash(&env);

        // expiry_ledger == current_seq → invalid.
        let res = client.try_lock(&sender, &receiver, &token, &1_000_000, &hashlock, &50);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::InvalidExpiry);
    }

    #[test]
    fn test_error_expiry_in_the_past() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(100);
        let hashlock = preimage_hash(&env);

        let res = client.try_lock(&sender, &receiver, &token, &1_000_000, &hashlock, &99);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::InvalidExpiry);
    }

    // -----------------------------------------------------------------------
    // Error: HashlockAlreadyExists
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_duplicate_hashlock_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);

        // First lock succeeds.
        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &100)
            .unwrap();

        // Second lock with the same hashlock is rejected.
        let res = client.try_lock(&sender, &receiver, &token, &500_000, &hashlock, &200);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::HashlockAlreadyExists);
    }

    // -----------------------------------------------------------------------
    // Error: EscrowNotFound
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_claim_unknown_hashlock() {
        let (env, client, _sender, receiver, _token) = setup();
        env.ledger().set_sequence_number(0);

        let unknown_hash: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        let res = client.try_claim(&receiver, &unknown_hash, &preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::EscrowNotFound);
    }

    #[test]
    fn test_error_refund_unknown_hashlock() {
        let (env, client, sender, _receiver, _token) = setup();
        env.ledger().set_sequence_number(200);

        let unknown_hash: BytesN<32> = BytesN::from_array(&env, &[0xffu8; 32]);

        let res = client.try_refund(&sender, &unknown_hash);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::EscrowNotFound);
    }

    // -----------------------------------------------------------------------
    // Error: AlreadySettled (double-claim / double-refund)
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_double_claim_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &200)
            .unwrap();

        env.ledger().set_sequence_number(100);
        // First claim succeeds.
        client.claim(&receiver, &hashlock, &preimage).unwrap();

        // Second claim must be rejected.
        let res = client.try_claim(&receiver, &hashlock, &preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::AlreadySettled);
    }

    #[test]
    fn test_error_refund_after_claim_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &50)
            .unwrap();

        env.ledger().set_sequence_number(25);
        client.claim(&receiver, &hashlock, &preimage).unwrap();

        // Advance past TTL and attempt refund — already claimed.
        env.ledger().set_sequence_number(100);
        let res = client.try_refund(&sender, &hashlock);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::AlreadySettled);
    }

    #[test]
    fn test_error_double_refund_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &50)
            .unwrap();

        env.ledger().set_sequence_number(50);
        client.refund(&sender, &hashlock).unwrap();

        let res = client.try_refund(&sender, &hashlock);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::AlreadySettled);
    }

    #[test]
    fn test_error_claim_after_refund_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &50)
            .unwrap();

        env.ledger().set_sequence_number(50);
        client.refund(&sender, &hashlock).unwrap();

        // Attempt to claim a refunded HTLC — rejected.
        let res = client.try_claim(&receiver, &hashlock, &preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::AlreadySettled);
    }

    // -----------------------------------------------------------------------
    // Error: UnauthorizedReceiver / UnauthorizedSender
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_wrong_receiver_cannot_claim() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &200)
            .unwrap();

        let impostor: Address = Address::generate(&env);
        env.ledger().set_sequence_number(100);
        let res = client.try_claim(&impostor, &hashlock, &preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::UnauthorizedReceiver);
    }

    #[test]
    fn test_error_wrong_sender_cannot_refund() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &50)
            .unwrap();

        let impostor: Address = Address::generate(&env);
        env.ledger().set_sequence_number(50);
        let res = client.try_refund(&impostor, &hashlock);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::UnauthorizedSender);
    }

    // -----------------------------------------------------------------------
    // Error: TimelockExpired (claim after TTL)
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_claim_after_expiry_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);
        let expiry_ledger: u32 = 100;

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &expiry_ledger)
            .unwrap();

        // Advance exactly to (and past) the expiry.
        env.ledger().set_sequence_number(expiry_ledger);
        let res = client.try_claim(&receiver, &hashlock, &preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::TimelockExpired);
    }

    /// One ledger past expiry — still expired.
    #[test]
    fn test_error_claim_one_ledger_past_expiry() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &50)
            .unwrap();

        env.ledger().set_sequence_number(51);
        let res = client.try_claim(&receiver, &hashlock, &preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::TimelockExpired);
    }

    // -----------------------------------------------------------------------
    // Error: TimelockNotExpired (refund before TTL)
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_premature_refund_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let expiry_ledger: u32 = 100;

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &expiry_ledger)
            .unwrap();

        // Attempt refund before TTL.
        env.ledger().set_sequence_number(99);
        let res = client.try_refund(&sender, &hashlock);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::TimelockNotExpired);
    }

    // -----------------------------------------------------------------------
    // Error: InvalidPreimage
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_wrong_preimage_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let expiry_ledger: u32 = 200;

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &expiry_ledger)
            .unwrap();

        let wrong_preimage = Bytes::from_slice(&env, b"this-is-not-the-correct-preimage");
        env.ledger().set_sequence_number(100);
        let res = client.try_claim(&receiver, &hashlock, &wrong_preimage);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::InvalidPreimage);
    }

    #[test]
    fn test_error_empty_preimage_rejected() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);

        client
            .lock(&sender, &receiver, &token, &1_000_000, &hashlock, &200)
            .unwrap();

        let empty = Bytes::from_slice(&env, b"");
        env.ledger().set_sequence_number(100);
        let res = client.try_claim(&receiver, &hashlock, &empty);
        assert_eq!(res.unwrap_err().unwrap(), HtlcError::InvalidPreimage);
    }

    // -----------------------------------------------------------------------
    // Asset distribution invariants
    // -----------------------------------------------------------------------

    /// Conservation law: total tokens in system never change.
    /// sender_before == sender_after_lock + contract_holding
    ///               == sender_after_lock + receiver_after_claim
    #[test]
    fn test_asset_conservation_across_full_lifecycle() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let tok = token::Client::new(&env, &token);

        let total_supply: i128 = tok.balance(&sender);
        let amount: i128 = 3_000_000;
        let hashlock = preimage_hash(&env);
        let preimage = Bytes::from_slice(&env, PREIMAGE_BYTES);

        // After lock: sender decreased, contract holds.
        client
            .lock(&sender, &receiver, &token, &amount, &hashlock, &100)
            .unwrap();
        assert_eq!(
            tok.balance(&sender) + tok.balance(&client.address) + tok.balance(&receiver),
            total_supply,
            "conservation after lock"
        );

        // After claim: receiver holds, contract zeroed.
        env.ledger().set_sequence_number(50);
        client.claim(&receiver, &hashlock, &preimage).unwrap();
        assert_eq!(
            tok.balance(&sender) + tok.balance(&client.address) + tok.balance(&receiver),
            total_supply,
            "conservation after claim"
        );
        assert_eq!(tok.balance(&client.address), 0, "contract must be empty");
    }

    /// Same conservation law for the refund path.
    #[test]
    fn test_asset_conservation_refund_path() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let tok = token::Client::new(&env, &token);

        let total_supply: i128 = tok.balance(&sender);
        let amount: i128 = 7_777_777;
        let hashlock = preimage_hash(&env);

        client
            .lock(&sender, &receiver, &token, &amount, &hashlock, &50)
            .unwrap();

        env.ledger().set_sequence_number(50);
        client.refund(&sender, &hashlock).unwrap();

        assert_eq!(
            tok.balance(&sender) + tok.balance(&client.address) + tok.balance(&receiver),
            total_supply,
            "conservation after refund"
        );
        assert_eq!(tok.balance(&receiver), 0, "receiver gets nothing on refund");
        assert_eq!(tok.balance(&client.address), 0, "contract must be empty");
        assert_eq!(
            tok.balance(&sender),
            total_supply,
            "sender gets back the full original balance"
        );
    }

    // -----------------------------------------------------------------------
    // View helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_htlc_returns_correct_entry() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);
        let hashlock = preimage_hash(&env);
        let amount: i128 = 4_200_000;
        let expiry_ledger: u32 = 999;

        client
            .lock(&sender, &receiver, &token, &amount, &hashlock, &expiry_ledger)
            .unwrap();

        let entry = client.get_htlc(&hashlock).unwrap();
        assert_eq!(entry.sender, sender);
        assert_eq!(entry.receiver, receiver);
        assert_eq!(entry.token, token);
        assert_eq!(entry.amount, amount);
        assert_eq!(entry.hashlock, hashlock);
        assert_eq!(entry.expiry_ledger, expiry_ledger);
        assert_eq!(entry.status, EscrowStatus::Active);
    }

    #[test]
    fn test_get_htlc_returns_none_for_unknown_hashlock() {
        let (env, client, _sender, _receiver, _token) = setup();
        let unknown: BytesN<32> = BytesN::from_array(&env, &[0xabu8; 32]);
        assert!(client.get_htlc(&unknown).is_none());
    }

    // -----------------------------------------------------------------------
    // Concurrent / independent HTLCs
    // -----------------------------------------------------------------------

    /// Two independent HTLCs (different hashlocks) can be locked and settled
    /// independently without interference.
    #[test]
    fn test_multiple_independent_htlcs() {
        let (env, client, sender, receiver, token) = setup();
        env.ledger().set_sequence_number(0);

        // Two distinct pre-images → two distinct hashlocks.
        let preimage_a = Bytes::from_slice(&env, b"preimage-A-exactly-32-bytes-long");
        let preimage_b = Bytes::from_slice(&env, b"preimage-B-exactly-32-bytes-long");
        let hash_a: BytesN<32> = env.crypto().sha256(&preimage_a);
        let hash_b: BytesN<32> = env.crypto().sha256(&preimage_b);

        let amount_a: i128 = 1_000_000;
        let amount_b: i128 = 2_000_000;

        client
            .lock(&sender, &receiver, &token, &amount_a, &hash_a, &200)
            .unwrap();
        client
            .lock(&sender, &receiver, &token, &amount_b, &hash_b, &300)
            .unwrap();

        let tok = token::Client::new(&env, &token);
        assert_eq!(tok.balance(&client.address), amount_a + amount_b);

        // Claim HTLC-A.
        env.ledger().set_sequence_number(100);
        client.claim(&receiver, &hash_a, &preimage_a).unwrap();
        assert_eq!(tok.balance(&receiver), amount_a);
        assert_eq!(tok.balance(&client.address), amount_b);

        // Refund HTLC-B after its separate TTL.
        env.ledger().set_sequence_number(300);
        client.refund(&sender, &hash_b).unwrap();
        assert_eq!(tok.balance(&client.address), 0);
    }
}
