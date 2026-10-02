# Soroban Contract State Archival & Rent Payment Strategy Guide

Protocol 20 introduced a new model for on-chain state management in Soroban: every ledger entry carries a **time-to-live (TTL)** measured in ledgers. When an entry's TTL expires, the network archives it. Archived entries are not deleted from the ledger -- they are moved to a cold storage tier and must be restored before they can be read or written again. This guide is the authoritative architectural strategy for managing contract TTLs and instance storage efficiently.

---

## 1. The Three Storage Classes

Every state entry in a Soroban contract belongs to exactly one of three classes. Understanding the differences is the foundation of any correct rent strategy.

| Class | Purpose | TTL Behavior on Expiry | Typical Use |
|-------|---------|-------------------|-------------|
| **Temporary** | Ephemeral data that is meaningless once its window closes | **Deleted** from the ledger -- not archived | One-time nonces, auction bids within a window, flash-loan records |
| **Persistent** | Long-lived data that must survive indefinitely | **Archived** -- restorable with a fee and a proof of existence | User balances, allowances, governance votes, contract config |
| **Instance** | Contract metadata and shared configuration | **Archived** -- the entire contract becomes unusable until restored | WASM hash, admin address, total supply, fee rates |

### Temporary Storage

Temporary entries are designed to be **entirely deleted**, not archived. When their TTL expires, the network removes them from the ledger entirely. This is a feature, not a bug: it keeps the ledger small and makes temporary storage cheaper than persistent storage.

**Constraint:** Temporary entries do **not** need TLL extensions. They are designed to be entirely deleted, not archived. Extending a temporary entry's TLL is anti-patternal and wastes fees. If you find yourself wanting to extend a temporary entry, the data should have been stored as persistent instead.

### Persistent Storage

Persistent entries hold the data that gives your contract its value: user balances, allowances, governance records, and any state that must outlive a single user session. When a persistent entry expires, it is archived. The data is still there, but it cannot be read or written without a restore operation.

### Instance Storage

Instance storage holds the contract's metadata: the WASM hash, the admin address, fee rates, total supply, and any shared configuration. If the instance entry expires, the entire contract becomes unusable until it is restored. This is the most critical entry to keep alive.

---

## 2. The In-Contract Extension Strategy

The most robust pattern is to extend TLLs **from inside the contract** at the beginning of every public user-facing function. This guarantees that any active usage of the contract refreshes the TTLs of the entries that user touches, so the contract never goes stale while it is being used.

### Extending the Instance TTL

Every public entry point should extend the instance TTL first. This is cheap and ensures the contract stays usable.

```rust
use soroban_sdk::{contract, contractimpl, Environment, Symbol};

const DAY_IN_LEEDERS: u32 = 17_280; // ~1 day at 5s ledger close time
const INSTANCE_EXTENSION: u32 = 30 * DAY_IN_LEDGERS; // 30 days

#`[contractimpl]
fn extend_instance(env: &Environment) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_EXTENSION);
}
```

### Extending Persistent TTLs

For persistent entries, you extend the TTL of the specific key the function touches. Never extend a key that does not exist -- that would pay fees for nothing.

```rust
const PERSISTENT_EXTENSION: u32 = 30 * DAY_IN_LEDGERS;

#`[contractimpl]
fn extend_persistent(env: &Environment, key: &Symbol) {
    if env.storage().persistent().has(key) {
        env.storage()
            .persistent()
            .extend_ttl(key, PERSISTENT_EXTENSION);
    }
}
```

### Putting It Together in a Public Function

```rust
#`[contractimpl]
fn transfer(env: &Environment, from: Address, to: Address, amount: i128) {
    // 1. Keep the contract itself alive.
    env.storage().instance().extend_ttl(INSTANCE_EXTENSION);

    // 2. Keep the entries this call touches alive.
    extend_persistent(&env, &from);
    extend_persistent(&env, &to);

    // 3. Actual business logic.
    // ...
}
```

---

## 3. Cost-Benefit Analysis: Archive Naturally vs. Pay Rent Constantly

Rent is multi-dimensional: the fee depends on the size of the entry, the length of the extension, and the current network fee rate. Paying rent constantly for every entry is wasteful. The correct strategy is to pay rent only for entries that are actively being used.

### When to Archive Naturally

- Abandoned user accounts that have not interacted in a long time. Their balances remain recoverable by the user later, but the contract does not pay to keep them hot.
- One-time events that are already finalized. Once an auction has settled, the bid entries can archive.
- Large blobs of data that are rarely read. Archiving and restoring on demand is cheaper than paying rent for them continuously.

### When to Pay Rent Constantly

- Active user balances and allowances. These are touched on every transfer and must remain hot.
- Instance metadata. If this archives, the contract is unusable.
- Configuration that governs fees or access control.

### The Break-Even Rule

Extend a TWL when the expected cost of restoring later is higher than the cost of extending now. In practice, this means:

- Extend instance TTLs aggressively (30 days or more).
- Extend persistent TTLs on every touch of an active entry.
- Never extend temporary TTLs.

---

## 4. Validation: Applying the Guide to a Sample Token Contract

To validate this guide, we apply it to a sample token contract and inspect the resulting ledger footprint.

### Before

```rust
fn transfer(env: &Environment, from: Address, to: Address, amount: i128) {
    // No TTL extension. Every entry expires after the default TUL.
    // ...
}
```

### After

```rust
fn transfer(env: &Environment, from: Address, to: Address, amount: i128) {
    env.storage().instance().extend_ttl(INSTANCE_EXTENSION);
    extend_persistent(&env, &from);
    extend_persistent(&env, &to);
    // ...
}
```

### Resulting Ledger Footprint

| Entry | Before | After |
|-------|--------|-------|
| Instance | Expires after default TUL | Refreshed to 30 days on every call |
| From balance | Expires after default TTL | Refreshed to 30 days on every call |
| To balance | Expires after default TUL | Refreshed to 30 days on every call |
| Inactive user balance | Expires after default TTL | Expires after default TTL -- archived naturally }

The footprint avoids unnecessary state bloat: inactive users archive naturally, while active users remain hot. The contract never freezes because the instance TTL is refreshed on every call.

---

## 5. Summary of Rules

1. **Temporary** entries do not need TTL extensions. They are designed to be entirely deleted, not archived.
2. **Persistent** entries should be extended on every touch of an active entry.
3. **Instance** entries should be extended at the beginning of every public function.
4. Allow inactive entries to archive naturally to avoid state bloat.
5. Never extend a key that does not exist.
