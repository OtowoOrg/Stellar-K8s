//! # Programmable Yield-Bearing Stablecoin
//!
//! A SEP-41–compatible token contract for Stellar Soroban that layers two
//! capabilities on top of the standard fungible-token interface:
//!
//! ## 1. Programmable Compliance (Mint/Burn Hooks)
//!
//! Every state-mutating operation (mint, burn, transfer, approve) passes
//! through the compliance gate in [`hooks`].  The gate enforces two
//! independent restrictions:
//!
//! * **Blacklist (sanctions list)** – permanently blocks an address from all
//!   inbound and outbound flows.  Removing an address requires a super-admin
//!   multi-sig quorum (≥ `unblacklist_threshold` out of `signers`).
//! * **Freeze** – temporarily halts all flows for an address; reversible by
//!   any single admin.
//!
//! ## 2. Proportional Yield Distribution
//!
//! An administrative multi-sig function [`YieldStablecoin::distribute_yield`]
//! increments each enrolled holder's balance proportionally to their share of
//! the total supply at distribution time.  Yield is funded by minting new
//! tokens (the issuer pre-funds yield reserves off-chain by holding the
//! minting authority).  The algorithm is O(n) over `recipients` but each
//! iteration is a single persistent-storage write, keeping per-ledger CPU
//! instruction consumption predictable and well below Soroban limits for
//! realistic holder set sizes.
//!
//! ## Storage Layout
//!
//! | Tier       | Key                         | Type          | Purpose                              |
//! |------------|----------------------------|---------------|--------------------------------------|
//! | Instance   | `Admin`                     | `Address`     | Primary admin                        |
//! | Instance   | `TotalSupply`               | `i128`        | Circulating supply                   |
//! | Instance   | `Decimals`                  | `u32`         | Token decimals (immutable after init)|
//! | Instance   | `Name`                      | `String`      | Token name                           |
//! | Instance   | `Symbol`                    | `String`      | Token symbol                         |
//! | Instance   | `MultiSigSigners`           | `Vec<Address>`| Authorized multi-sig signers         |
//! | Instance   | `MultiSigThreshold`         | `u32`         | Signatures required for privileged ops |
//! | Instance   | `UnblacklistThreshold`      | `u32`         | Signatures required to un-blacklist  |
//! | Persistent | `Balance(addr)`             | `i128`        | Per-address token balance            |
//! | Persistent | `Allowance(owner, spender)` | `i128`        | ERC-20-style spending allowance      |
//! | Persistent | `Blacklisted(addr)`         | `bool`        | Sanctions list membership            |
//! | Persistent | `Frozen(addr)`              | `bool`        | Temporary freeze flag                |
//!
//! Instance storage is intentionally kept small (admin config only) to ensure
//! the security-sensitive multi-sig thresholds are **segregated** from
//! standard user balance data, which lives entirely in persistent storage.

#![no_std]

pub mod hooks;
pub mod storage;
#[cfg(test)]
mod test;

use hooks::{
    assert_can_receive, assert_can_send, assert_transfer_allowed, blacklist, freeze, is_blacklisted,
    is_frozen, thaw, unblacklist,
};
use storage::DataKey;

use soroban_sdk::{contract, contracterror, contractevent, contractimpl, Address, Env, String, Vec};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Minimum TTL extension (in ledgers) applied to instance storage on every
/// write.  ~7 days at 5-second ledger close.
const INSTANCE_TTL_EXTEND: u32 = 120_960;
const INSTANCE_TTL_THRESHOLD: u32 = 60_480;

// ── Errors ────────────────────────────────────────────────────────────────────

/// Contract-level errors for the yield stablecoin.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet.
    NotInitialized = 2,
    /// Caller is not the admin.
    Unauthorized = 3,
    /// Amount must be strictly positive.
    InvalidAmount = 4,
    /// Transfer or burn exceeds available balance.
    InsufficientBalance = 5,
    /// Transfer exceeds the approved allowance.
    InsufficientAllowance = 6,
    /// Arithmetic overflow in balance or supply calculation.
    Overflow = 7,
    /// Provided signer list is empty or threshold is zero / out of range.
    InvalidMultiSig = 8,
    /// Insufficient distinct signers provided for a multi-sig operation.
    MultiSigQuorumNotMet = 9,
    /// Recipient list and amount list lengths do not match.
    LengthMismatch = 10,
    /// A signer appeared more than once in the provided signature set.
    DuplicateSigner = 11,
}

// ── Events ────────────────────────────────────────────────────────────────────

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Minted {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Burned {
    #[topic]
    pub from: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transferred {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approved {
    #[topic]
    pub owner: Address,
    #[topic]
    pub spender: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YieldDistributed {
    #[topic]
    pub epoch: u32,
    pub total_yield: i128,
    pub recipient_count: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressBlacklisted {
    #[topic]
    pub addr: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressUnblacklisted {
    #[topic]
    pub addr: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressFrozen {
    #[topic]
    pub addr: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressThawed {
    #[topic]
    pub addr: Address,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct YieldStablecoin;

#[contractimpl]
impl YieldStablecoin {
    // ── Constructor ──────────────────────────────────────────────────────────

    /// Initialize the stablecoin.
    ///
    /// * `admin` – primary admin; required auth is checked.
    /// * `name` / `symbol` / `decimals` – immutable token metadata.
    /// * `signers` – initial set of multi-sig signers; must be non-empty.
    /// * `threshold` – number of signers required for privileged operations
    ///   (yield distribution, config changes); must be 1 ≤ threshold ≤
    ///   signers.len().
    /// * `unblacklist_threshold` – stricter quorum required to remove an
    ///   address from the sanctions list; must be ≥ threshold.
    pub fn __constructor(
        env: Env,
        admin: Address,
        name: String,
        symbol_str: String,
        decimals: u32,
        signers: Vec<Address>,
        threshold: u32,
        unblacklist_threshold: u32,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            env.panic_with_error(&Error::AlreadyInitialized);
        }
        admin.require_auth();

        let n = signers.len();
        if n == 0 || threshold == 0 || threshold > n || unblacklist_threshold < threshold {
            env.panic_with_error(&Error::InvalidMultiSig);
        }

        let store = env.storage().instance();
        store.set(&DataKey::Admin, &admin);
        store.set(&DataKey::Name, &name);
        store.set(&DataKey::TokenSymbol, &symbol_str);
        store.set(&DataKey::Decimals, &decimals);
        store.set(&DataKey::TotalSupply, &0i128);
        store.set(&DataKey::MultiSigSigners, &signers);
        store.set(&DataKey::MultiSigThreshold, &threshold);
        store.set(&DataKey::UnblacklistThreshold, &unblacklist_threshold);

        refresh_instance_ttl(&env);
    }

    // ── SEP-41 view functions ────────────────────────────────────────────────

    /// Returns the token name.
    pub fn name(env: Env) -> String {
        instance_get(&env, &DataKey::Name)
    }

    /// Returns the token symbol.
    pub fn symbol(env: Env) -> String {
        instance_get(&env, &DataKey::TokenSymbol)
    }

    /// Returns the number of decimal places.
    pub fn decimals(env: Env) -> u32 {
        instance_get(&env, &DataKey::Decimals)
    }

    /// Returns the total circulating supply.
    pub fn total_supply(env: Env) -> i128 {
        instance_get(&env, &DataKey::TotalSupply)
    }

    /// Returns the token balance of `id`.
    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0i128)
    }

    /// Returns the spending allowance granted by `owner` to `spender`.
    pub fn allowance(env: Env, owner: Address, spender: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Allowance(owner, spender))
            .unwrap_or(0i128)
    }

    // ── SEP-41 mutating functions ────────────────────────────────────────────

    /// Transfer `amount` tokens from `from` to `to`.
    ///
    /// Requires `from.require_auth()` and compliance clearance for both
    /// parties.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        from.require_auth();
        assert_transfer_allowed(&env, &from, &to);
        do_transfer(&env, &from, &to, amount)?;
        Transferred {
            from: from.clone(),
            to: to.clone(),
            amount,
        }
        .publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Transfer `amount` tokens from `from` to `to` using `spender`'s
    /// pre-approved allowance.
    pub fn transfer_from(
        env: Env,
        spender: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        spender.require_auth();
        assert_transfer_allowed(&env, &from, &to);
        deduct_allowance(&env, &from, &spender, amount)?;
        do_transfer(&env, &from, &to, amount)?;
        Transferred {
            from: from.clone(),
            to: to.clone(),
            amount,
        }
        .publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Approve `spender` to transfer up to `amount` on behalf of `owner`.
    pub fn approve(env: Env, owner: Address, spender: Address, amount: i128) -> Result<(), Error> {
        if amount < 0 {
            return Err(Error::InvalidAmount);
        }
        owner.require_auth();
        // The owner must not be frozen or blacklisted to grant allowances.
        assert_can_send(&env, &owner);
        env.storage()
            .persistent()
            .set(&DataKey::Allowance(owner.clone(), spender.clone()), &amount);
        Approved {
            owner,
            spender,
            amount,
        }
        .publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    // ── Mint / Burn ──────────────────────────────────────────────────────────

    /// Mint `amount` new tokens directly to `to`.
    ///
    /// Requires admin authorization and compliance clearance for the
    /// recipient.  The compliance hook is executed **before** any state
    /// mutation so that a blacklisted recipient can never receive yield or
    /// minted supply.
    pub fn mint(env: Env, admin: Address, to: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        require_admin(&env, &admin)?;
        // Compliance gate: recipient must not be blacklisted or frozen.
        assert_can_receive(&env, &to);

        let new_balance = credit_balance(&env, &to, amount)?;
        let _ = new_balance; // balance update handled inside credit_balance

        let supply: i128 = instance_get(&env, &DataKey::TotalSupply);
        let new_supply = supply.checked_add(amount).ok_or(Error::Overflow)?;
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &new_supply);

        Minted {
            to: to.clone(),
            amount,
        }
        .publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Burn `amount` tokens from `from`.
    ///
    /// Requires `from.require_auth()` and compliance send-clearance (a
    /// blacklisted address cannot liquidate their position through burn).
    pub fn burn(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        from.require_auth();
        assert_can_send(&env, &from);

        let bal: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);
        if bal < amount {
            return Err(Error::InsufficientBalance);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(bal - amount));

        let supply: i128 = instance_get(&env, &DataKey::TotalSupply);
        let new_supply = supply.checked_sub(amount).ok_or(Error::Overflow)?;
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &new_supply);

        Burned {
            from: from.clone(),
            amount,
        }
        .publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    // ── Multi-Sig Yield Distribution ─────────────────────────────────────────

    /// Distribute protocol yield proportionally to a set of holders.
    ///
    /// This function implements the core **programmable yield** mechanism.
    /// It mints `total_yield_amount` tokens and distributes them across
    /// `recipients` in proportion to each recipient's share of
    /// `total_supply_snapshot` (the total supply at the time the yield epoch
    /// was computed off-chain).
    ///
    /// ## Authorization
    ///
    /// Requires `threshold` distinct signers from the registered multi-sig
    /// signer set to each call `require_auth()`.  Any duplicate signer address
    /// in `signers_authorizing` panics with [`Error::DuplicateSigner`] to
    /// prevent a single key from fulfilling multiple slots.
    ///
    /// ## Compliance
    ///
    /// Every recipient is checked through the compliance hook before their
    /// balance is credited.  A blacklisted or frozen address is **silently
    /// skipped** (the yield for that address is not redistributed — it stays
    /// in the minted supply, effectively burning the allocation for that
    /// address from the circulating perspective until an admin corrects the
    /// situation off-chain).  This design avoids a single non-compliant
    /// address blocking the entire distribution epoch.
    ///
    /// ## Parameters
    ///
    /// * `signers_authorizing` – the subset of registered signers providing
    ///   authorization for this call; must be ≥ `threshold` distinct members.
    /// * `recipients` – addresses to receive yield.
    /// * `recipient_balances` – each recipient's balance used as the numerator
    ///   for proportional share computation.
    /// * `total_supply_snapshot` – denominator for share computation; must be
    ///   > 0.
    /// * `total_yield_amount` – gross yield to distribute (in token base units).
    /// * `epoch` – monotonic epoch identifier for event correlation.
    ///
    /// ## Yield Allocation Formula
    ///
    /// ```text
    /// allocation_i = (recipient_balance_i * total_yield_amount) / total_supply_snapshot
    /// ```
    ///
    /// Integer division truncates; any dust (rounding remainder) stays
    /// unminted to preserve `total_supply` accuracy.
    pub fn distribute_yield(
        env: Env,
        signers_authorizing: Vec<Address>,
        recipients: Vec<Address>,
        recipient_balances: Vec<i128>,
        total_supply_snapshot: i128,
        total_yield_amount: i128,
        epoch: u32,
    ) -> Result<i128, Error> {
        if total_yield_amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if recipients.len() != recipient_balances.len() {
            return Err(Error::LengthMismatch);
        }
        if total_supply_snapshot <= 0 {
            return Err(Error::InvalidAmount);
        }

        // ── Multi-sig authorization ──────────────────────────────────────────
        verify_multisig(&env, &signers_authorizing, false)?;

        // ── Per-recipient yield crediting ────────────────────────────────────
        let mut total_minted: i128 = 0i128;
        let mut credited_count: u32 = 0u32;

        let n = recipients.len();
        for i in 0..n {
            let recipient = recipients.get(i).unwrap();
            let rec_balance = recipient_balances.get(i).unwrap();

            // Non-positive balance → no yield
            if rec_balance <= 0 {
                continue;
            }

            // Compliance gate: skip restricted addresses without aborting.
            if is_blacklisted(&env, &recipient) || is_frozen(&env, &recipient) {
                continue;
            }

            // Proportional allocation: floor division, dust stays unminted.
            let allocation = rec_balance
                .checked_mul(total_yield_amount)
                .ok_or(Error::Overflow)?
                / total_supply_snapshot;

            if allocation <= 0 {
                continue;
            }

            credit_balance(&env, &recipient, allocation)?;
            total_minted = total_minted
                .checked_add(allocation)
                .ok_or(Error::Overflow)?;
            credited_count += 1;
        }

        // Update total supply by the amount actually minted.
        if total_minted > 0 {
            let supply: i128 = instance_get(&env, &DataKey::TotalSupply);
            let new_supply = supply.checked_add(total_minted).ok_or(Error::Overflow)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalSupply, &new_supply);
        }

        YieldDistributed {
            epoch,
            total_yield: total_minted,
            recipient_count: credited_count,
        }
        .publish(&env);
        refresh_instance_ttl(&env);
        Ok(total_minted)
    }

    // ── Compliance admin functions ───────────────────────────────────────────

    /// Add `addr` to the on-chain sanctions list.
    ///
    /// Requires single admin authorization.
    pub fn blacklist_address(env: Env, admin: Address, addr: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        blacklist(&env, &addr);
        AddressBlacklisted { addr }.publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Remove `addr` from the on-chain sanctions list.
    ///
    /// Requires the stricter `unblacklist_threshold` multi-sig quorum.
    pub fn unblacklist_address(
        env: Env,
        signers_authorizing: Vec<Address>,
        addr: Address,
    ) -> Result<(), Error> {
        verify_multisig(&env, &signers_authorizing, true)?;
        unblacklist(&env, &addr);
        AddressUnblacklisted { addr }.publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Freeze `addr`, halting all inbound and outbound flows immediately.
    ///
    /// Requires single admin authorization.
    pub fn freeze_address(env: Env, admin: Address, addr: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        freeze(&env, &addr);
        AddressFrozen { addr }.publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Lift the freeze on `addr`.
    ///
    /// Requires single admin authorization.
    pub fn thaw_address(env: Env, admin: Address, addr: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        thaw(&env, &addr);
        AddressThawed { addr }.publish(&env);
        refresh_instance_ttl(&env);
        Ok(())
    }

    // ── Multi-sig config ─────────────────────────────────────────────────────

    /// Replace the multi-sig signer set and thresholds.
    ///
    /// Requires the current `threshold` multi-sig quorum to prevent a
    /// compromised single admin key from silently downgrading the quorum.
    pub fn update_multisig(
        env: Env,
        signers_authorizing: Vec<Address>,
        new_signers: Vec<Address>,
        new_threshold: u32,
        new_unblacklist_threshold: u32,
    ) -> Result<(), Error> {
        verify_multisig(&env, &signers_authorizing, false)?;

        let n = new_signers.len();
        if n == 0
            || new_threshold == 0
            || new_threshold > n
            || new_unblacklist_threshold < new_threshold
        {
            return Err(Error::InvalidMultiSig);
        }

        let store = env.storage().instance();
        store.set(&DataKey::MultiSigSigners, &new_signers);
        store.set(&DataKey::MultiSigThreshold, &new_threshold);
        store.set(&DataKey::UnblacklistThreshold, &new_unblacklist_threshold);
        refresh_instance_ttl(&env);
        Ok(())
    }

    /// Transfer admin authority to `new_admin`.
    ///
    /// Requires the current admin's authorization.
    pub fn set_admin(env: Env, current_admin: Address, new_admin: Address) -> Result<(), Error> {
        require_admin(&env, &current_admin)?;
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        refresh_instance_ttl(&env);
        Ok(())
    }

    // ── Compliance view helpers ──────────────────────────────────────────────

    /// Returns `true` if `addr` is on the sanctions list.
    pub fn is_blacklisted(env: Env, addr: Address) -> bool {
        is_blacklisted(&env, &addr)
    }

    /// Returns `true` if `addr` is currently frozen.
    pub fn is_frozen(env: Env, addr: Address) -> bool {
        is_frozen(&env, &addr)
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Extend the instance storage TTL to keep admin config alive.
fn refresh_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

/// Load a value from instance storage; panics if the contract is not
/// initialized (key absent).
fn instance_get<V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> V {
    env.storage()
        .instance()
        .get(key)
        .unwrap_or_else(|| env.panic_with_error(&Error::NotInitialized))
}

/// Require `caller` to be the registered admin and call `require_auth()`.
fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin: Address = instance_get(env, &DataKey::Admin);
    if *caller != admin {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

/// Verify that `signers_authorizing` contains at least `threshold` distinct
/// members from the registered signer set, with each calling `require_auth()`.
///
/// If `use_unblacklist_threshold` is `true`, the stricter
/// `unblacklist_threshold` is used instead of the standard `threshold`.
fn verify_multisig(
    env: &Env,
    signers_authorizing: &Vec<Address>,
    use_unblacklist_threshold: bool,
) -> Result<(), Error> {
    let registered: Vec<Address> = instance_get(env, &DataKey::MultiSigSigners);
    let threshold: u32 = if use_unblacklist_threshold {
        instance_get(env, &DataKey::UnblacklistThreshold)
    } else {
        instance_get(env, &DataKey::MultiSigThreshold)
    };

    // Collect valid signers, detecting duplicates via a sorted check.
    // We can't use HashSet in no_std, so we do an O(n²) duplicate check
    // which is safe given that signer sets are small (≤ ~20 members).
    let mut valid_count: u32 = 0;

    for i in 0..signers_authorizing.len() {
        let signer = signers_authorizing.get(i).unwrap();

        // Check for duplicate within `signers_authorizing`.
        for j in 0..i {
            if signers_authorizing.get(j).unwrap() == signer {
                return Err(Error::DuplicateSigner);
            }
        }

        // Check membership in registered signer set.
        let mut is_registered = false;
        for k in 0..registered.len() {
            if registered.get(k).unwrap() == signer {
                is_registered = true;
                break;
            }
        }
        if !is_registered {
            continue; // unregistered signer — simply ignore, don't count
        }

        signer.require_auth();
        valid_count += 1;
    }

    if valid_count < threshold {
        return Err(Error::MultiSigQuorumNotMet);
    }
    Ok(())
}

/// Credit `amount` to `addr`'s persistent balance.  Returns the new balance.
fn credit_balance(env: &Env, addr: &Address, amount: i128) -> Result<i128, Error> {
    let current: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Balance(addr.clone()))
        .unwrap_or(0);
    let new_bal = current.checked_add(amount).ok_or(Error::Overflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::Balance(addr.clone()), &new_bal);
    Ok(new_bal)
}

/// Debit `amount` from `from`'s persistent balance and credit it to `to`.
fn do_transfer(env: &Env, from: &Address, to: &Address, amount: i128) -> Result<(), Error> {
    let from_bal: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Balance(from.clone()))
        .unwrap_or(0);
    if from_bal < amount {
        return Err(Error::InsufficientBalance);
    }
    env.storage()
        .persistent()
        .set(&DataKey::Balance(from.clone()), &(from_bal - amount));
    credit_balance(env, to, amount)?;
    Ok(())
}

/// Deduct `amount` from `owner`→`spender` allowance.
fn deduct_allowance(
    env: &Env,
    owner: &Address,
    spender: &Address,
    amount: i128,
) -> Result<(), Error> {
    let key = DataKey::Allowance(owner.clone(), spender.clone());
    let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    if current < amount {
        return Err(Error::InsufficientAllowance);
    }
    env.storage().persistent().set(&key, &(current - amount));
    Ok(())
}

/// Expose BPS_SCALE for tests without making the constant `pub`.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn bps_scale() -> i128 {
    10_000
}
