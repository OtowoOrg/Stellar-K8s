//! Registry module for the TTL auto-bump maintenance contract.
//!
//! Tracks all (contract_address, storage_key) pairs that require periodic TTL
//! extension.  Each entry records:
//!   - `contract_id`   – the Soroban contract whose key needs bumping
//!   - `key`           – the `Val`-serialised storage key within that contract
//!   - `target_ttl`    – desired live-until ledger (relative extension amount)
//!   - `threshold`     – ledgers-before-expiry at which a bump is needed
//!   - `owner`         – address that registered the entry and can remove it
//!
//! Storage layout (Persistent):
//!   `DataKey::Entries`  →  `Vec<RegistryEntry>` – ordered list of all entries
//!   `DataKey::Count`    →  `u32`                – running counter for IDs
//!   `DataKey::Admin`    →  `Address`            – privileged admin address
//!   `DataKey::BountyBalance` → `i128`           – XLM balance held for bounties
//!   `DataKey::BountyPerKey` → `u128`            – stroops paid per key bumped

use soroban_sdk::{contracttype, Address, Env, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of entries allowed in the registry to bound gas usage.
pub const MAX_REGISTRY_SIZE: u32 = 256;

/// Default threshold: trigger a bump when a key has fewer than this many
/// ledgers remaining before expiration.
pub const DEFAULT_THRESHOLD_LEDGERS: u32 = 4_320; // ~6 hours at ~5s/ledger

/// Default TTL extension: extend by this many ledgers when bumped.
pub const DEFAULT_EXTENSION_LEDGERS: u32 = 17_280; // ~24 hours

/// Minimum bounty per key (1 stroop) to prevent dust payouts.
pub const MIN_BOUNTY_PER_KEY: i128 = 1;

// ---------------------------------------------------------------------------
// Storage key enum
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    /// Ordered list of all registered entries.
    Entries,
    /// Monotonically increasing ID counter.
    Count,
    /// Admin address – allowed to set bounty config and force-remove entries.
    Admin,
    /// Total XLM bounty balance deposited into the contract (in stroops).
    BountyBalance,
    /// Stroops awarded to keeper for each successfully bumped key.
    BountyPerKey,
}

// ---------------------------------------------------------------------------
// Entry type
// ---------------------------------------------------------------------------

/// A single registry entry describing one contract key that needs TTL bumping.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryEntry {
    /// Unique numeric identifier for this entry.
    pub id: u32,
    /// The contract address whose storage key we are watching.
    pub contract_id: Address,
    /// Ledgers-before-expiry threshold: bump is eligible once TTL ≤ threshold.
    pub threshold_ledgers: u32,
    /// How many additional ledgers to extend when bumping.
    pub extension_ledgers: u32,
    /// Owner of this entry – the only address allowed to remove it.
    pub owner: Address,
    /// Whether this entry is active.
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Registry helpers
// ---------------------------------------------------------------------------

/// Initialise the registry state.  Must be called exactly once during `init`.
pub fn init(env: &Env, admin: &Address) {
    let empty: Vec<RegistryEntry> = Vec::new(env);
    env.storage().persistent().set(&DataKey::Entries, &empty);
    env.storage().persistent().set(&DataKey::Count, &0u32);
    env.storage().persistent().set(&DataKey::Admin, admin);
    env.storage()
        .persistent()
        .set(&DataKey::BountyBalance, &0i128);
    env.storage()
        .persistent()
        .set(&DataKey::BountyPerKey, &0i128);
}

/// Retrieve the admin address.
pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .persistent()
        .get(&DataKey::Admin)
        .expect("admin not set")
}

/// Retrieve all registry entries.
pub fn get_entries(env: &Env) -> Vec<RegistryEntry> {
    env.storage()
        .persistent()
        .get(&DataKey::Entries)
        .unwrap_or_else(|| Vec::new(env))
}

/// Persist an updated entries list.
pub fn set_entries(env: &Env, entries: &Vec<RegistryEntry>) {
    env.storage().persistent().set(&DataKey::Entries, entries);
}

/// Return the current entry count (number of times `register` was called).
pub fn get_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::Count)
        .unwrap_or(0u32)
}

/// Increment and return the next unique ID.
pub fn next_id(env: &Env) -> u32 {
    let id = get_count(env) + 1;
    env.storage().persistent().set(&DataKey::Count, &id);
    id
}

/// Return the current bounty balance (stroops).
pub fn get_bounty_balance(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::BountyBalance)
        .unwrap_or(0i128)
}

/// Set the bounty balance.
pub fn set_bounty_balance(env: &Env, balance: i128) {
    env.storage()
        .persistent()
        .set(&DataKey::BountyBalance, &balance);
}

/// Return the per-key bounty amount (stroops).
pub fn get_bounty_per_key(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::BountyPerKey)
        .unwrap_or(0i128)
}

/// Set the per-key bounty amount (stroops).
pub fn set_bounty_per_key(env: &Env, amount: i128) {
    env.storage()
        .persistent()
        .set(&DataKey::BountyPerKey, &amount);
}

/// Add an entry to the registry and return its assigned ID.
///
/// Panics if:
///   - the registry has reached `MAX_REGISTRY_SIZE`
///   - `threshold_ledgers` is 0
///   - `extension_ledgers` is 0
pub fn register_entry(
    env: &Env,
    contract_id: Address,
    threshold_ledgers: u32,
    extension_ledgers: u32,
    owner: Address,
) -> u32 {
    let mut entries = get_entries(env);
    assert!(
        entries.len() < MAX_REGISTRY_SIZE,
        "registry is full (max {})",
        MAX_REGISTRY_SIZE
    );
    assert!(threshold_ledgers > 0, "threshold_ledgers must be > 0");
    assert!(extension_ledgers > 0, "extension_ledgers must be > 0");

    let id = next_id(env);
    let entry = RegistryEntry {
        id,
        contract_id,
        threshold_ledgers,
        extension_ledgers,
        owner,
        active: true,
    };
    entries.push_back(entry);
    set_entries(env, &entries);
    id
}

/// Deactivate an entry by ID.  Only the entry owner or the admin may call this.
///
/// Returns `true` if the entry was found and deactivated, `false` if not found.
pub fn deregister_entry(env: &Env, id: u32, caller: &Address) -> bool {
    let admin = get_admin(env);
    let entries = get_entries(env);
    let mut found = false;

    // Rebuild the list with the target entry marked inactive.
    let mut updated: Vec<RegistryEntry> = Vec::new(env);
    for entry in entries.iter() {
        if entry.id == id {
            assert!(
                entry.owner == *caller || admin == *caller,
                "only the owner or admin can deregister"
            );
            let mut e = entry;
            e.active = false;
            updated.push_back(e);
            found = true;
        } else {
            updated.push_back(entry);
        }
    }

    if found {
        set_entries(env, &updated);
    }
    found
}

/// Return only the active entries whose `threshold_ledgers` requirement
/// indicates they need attention at the current ledger.
///
/// Because we cannot read another contract's storage TTL from within a
/// contract call in this architecture (TTL checks happen on-chain per-key),
/// the bump logic in `lib.rs` is responsible for checking liveness.  This
/// function simply returns all active entries so the batch executor can
/// iterate over them efficiently.
pub fn active_entries(env: &Env) -> Vec<RegistryEntry> {
    let all = get_entries(env);
    let mut active: Vec<RegistryEntry> = Vec::new(env);
    for entry in all.iter() {
        if entry.active {
            active.push_back(entry);
        }
    }
    active
}

/// Return an active entry by ID, or panic if not found / inactive.
pub fn get_active_entry(env: &Env, id: u32) -> RegistryEntry {
    for entry in get_entries(env).iter() {
        if entry.id == id && entry.active {
            return entry;
        }
    }
    panic!("entry {} not found or inactive", id);
}

/// Count how many active entries are in the registry.
pub fn active_entry_count(env: &Env) -> u32 {
    let entries = get_entries(env);
    let mut count = 0u32;
    for entry in entries.iter() {
        if entry.active {
            count += 1;
        }
    }
    count
}
