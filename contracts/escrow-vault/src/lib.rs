//! # Non-custodial Escrow & Collateral Vault
//!
//! A collateral vault for validator node operators who want to stake assets
//! as **performance guarantees** for automated, cross-chain node provisioning
//! and relaying.
//!
//! ## Design notes
//!
//! * **Soroban / Stellar aligned.** Balances are `i128` amounts and transfers
//!   are modelled on the Soroban token interface (a [`TokenLedger`] in‑memory
//!   stand‑in). Collateral held in the vault behaves like a Stellar
//!   *claimable balance*: it is a claim owed to a claimant, and release is
//!   initiated by the claimant pulling the funds (see *pull‑over‑push*).
//! * **Pull‑over‑push.** The vault never probes an unknown recipient's `receive`
//!   hook first. Every payout — collateral release, slashed‑funds collection,
//!   yield collection — is a *pull* operation invoked by the intended recipient
//!   (or the authorized caller) in a single transaction. The vault transfers
//!   funds only to well‑known, validated addresses (the operator, the notifier,
//!   or a keeper) so a malformed recipient contract cannot lock up execution
//!   or strand funds.
//! * **Slashing only on valid proofs.** Slashes are applied exclusively through
//!   [`slashing::verify_double_sign`] / [`slashing::verify_reporter_signature`],
//!   which require genuine Ed25519 signatures (authorized reporter for downtime,
//!   the node's own consensus key for double‑signing) plus freshness checks.
//! * **No stuck funds.** Every liability is claimable: operators can release
//!   their (non‑slashed) collateral once the lockup expires, slashed collateral
//!   can be pulled by the notifier, yield can be pulled by operators, and an
//!   idle yield pool can be reclaimed by the admin. [`Vault::assert_solvent`]
//!   is the total‑solvency invariant, covered by property tests in
//!   `tests/solvency_properties.rs`.

pub mod slashing;

use std::collections::HashMap;
use std::fmt;

pub use slashing::{DoubleSignMaterial, Proof, ProofType};

/// Collateral amount in stroop‑like units (i128, matching the Soroban ABI).
pub type Amount = i128;

/// Ledger/binary time used for lockups and proof freshness.
pub type LedgerTime = u64;

/// A 256‑bit identifier for an address (account or contract) or a node key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address([u8; 32]);

impl Address {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Address(bytes)
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0
    }
}

impl From<[u8; 32]> for Address {
    fn from(bytes: [u8; 32]) -> Self {
        Address(bytes)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.hex())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl Address {
    #[inline]
    fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A deliberately sparse address used to hold the vault's own token balance.
pub fn vault_address() -> Address {
    Address([0u8; 32])
}

/// Errors surfaced by the vault. All are recoverable; a failed call never
/// mutates the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    Unauthorized,
    InsufficientBalance,
    Overflow,
    InvalidProof(String),
    StaleProof,
    PositionNotFound,
    WrongStatus(String),
    LockupNotExpired,
    LockupExpired,
    NotFresh,
    ZeroAmount,
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::Unauthorized => write!(f, "caller is not authorized"),
            VaultError::InsufficientBalance => write!(f, "insufficient balance"),
            VaultError::Overflow => write!(f, "arithmetic overflow"),
            VaultError::InvalidProof(s) => write!(f, "invalid slashing proof: {s}"),
            VaultError::StaleProof => write!(f, "slashing proof is stale"),
            VaultError::PositionNotFound => write!(f, "position not found"),
            VaultError::WrongStatus(s) => write!(f, "position in wrong state: {s}"),
            VaultError::LockupNotExpired => write!(f, "lockup has not expired yet"),
            VaultError::LockupExpired => write!(f, "lockup has already expired"),
            VaultError::NotFresh => write!(f, "observation is not fresh"),
            VaultError::ZeroAmount => write!(f, "amount must be positive"),
        }
    }
}

impl std::error::Error for VaultError {}

/// Lifecycle state of a single collateral position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionStatus {
    /// Collateral is deposited and held until `lockup_until`.
    Locked,
    /// A dispute is open; release is frozen until it is resolved.
    Disputed,
    /// A fault was confirmed; `slashed` collateral is reserved for the notifier
    /// (any remainder stays releasable by the operator after expiry).
    Slashed,
    /// The operator pulled their (full/remaining) collateral.
    Released,
    /// 100% slashed; nothing remains for the operator.
    Forfeited,
}

impl fmt::Display for PositionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            PositionStatus::Locked => "locked",
            PositionStatus::Disputed => "disputed",
            PositionStatus::Slashed => "slashed",
            PositionStatus::Released => "released",
            PositionStatus::Forfeited => "forfeited",
        };
        f.write_str(s)
    }
}

/// A collateral position binding an operator to a node for a lockup period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub id: u64,
    pub operator: Address,
    /// Public key of the secured node (used to match slashing proofs).
    pub node_id: Address,
    /// The node's consensus verifying key (bytes) for double-sign proofs.
    pub node_vk: [u8; 32],
    /// Total collateral originally deposited.
    pub deposit: Amount,
    /// Collateral carved out by confirmed slashes (reserved for, but not yet
    /// claimed by, the notifier).
    pub slashed: Amount,
    /// Collateral already pulled back by the notifier (fully accounted).
    pub claimed: Amount,
    /// Collateral already pulled back by the operator (fully accounted).
    pub released: Amount,
    /// Unclaimed proportional yield owed to the operator.
    pub unclaimed_yield: Amount,
    pub locked_at: LedgerTime,
    pub lockup_until: LedgerTime,
    pub status: PositionStatus,
}

impl Position {
    /// Collateral still owed to the operator (never negative). Accounts for the
    /// three liability buckets: `slashed` (notifier), `claimed` (already handed
    /// to the notifier) and `released` (already handed to the operator).
    pub fn remainder(&self) -> Amount {
        self.deposit
            .saturating_sub(self.slashed)
            .saturating_sub(self.claimed)
            .saturating_sub(self.released)
            .max(0)
    }

    /// True when the position still holds releasable collateral.
    fn has_remainder(&self) -> bool {
        self.remainder() > 0
    }
}

/// Vault-wide parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultConfig {
    pub admin: Address,
    /// Recipient of slashed collateral (insurance fund / DA).
    pub notifier: Address,
    /// Lockup duration after deposit (ledger/time units).
    pub lockup_window: LedgerTime,
    /// Window during which a dispute may be opened (after deposit).
    pub dispute_window: LedgerTime,
    /// Maximum age (in time units) of an accepted downtime proof.
    pub slashing_staleness: LedgerTime,
    /// Basis points of collateral slashed for a downtime fault (10000 = 100%).
    pub downtime_slash_bps: u32,
    /// Basis points of collateral slashed for a double-sign fault.
    pub double_sign_slash_bps: u32,
    /// Authorized reporters: `Address` of the reporter -> their Ed25519 key.
    pub reporters: HashMap<Address, [u8; 32]>,
}

impl VaultConfig {
    /// Validates configuration invariants.
    pub fn validate(&self) -> Result<(), VaultError> {
        if self.downtime_slash_bps > 10_000 || self.double_sign_slash_bps > 10_000 {
            return Err(VaultError::InvalidProof(
                "slash basis points must be in [0, 10000]".into(),
            ));
        }
        Ok(())
    }
}

/// Minimal in-memory token ledger modelling the Soroban token interface
/// (`balance`, `transfer`). In production this is a real Soroban token client;
/// the vault only ever transfers to/from well-known addresses (pull-over-push).
#[derive(Debug, Clone, Default)]
pub struct TokenLedger {
    balances: HashMap<Address, Amount>,
}

impl TokenLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn balance_of(&self, addr: &Address) -> Amount {
        self.balances.get(addr).copied().unwrap_or(0)
    }

    /// Mints `amount` to `addr` (used by tests/keepers to fund operators).
    pub fn mint(&mut self, addr: Address, amount: Amount) -> Result<(), VaultError> {
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let bal = self.balances.entry(addr).or_insert(0);
        *bal = bal.checked_add(amount).ok_or(VaultError::Overflow)?;
        Ok(())
    }

    /// Pulls `amount` from `from` to `to`, enforcing the `from` balance cap.
    /// This is the only primitive the vault uses to move collateral.
    pub fn transfer(
        &mut self,
        from: &Address,
        to: &Address,
        amount: Amount,
    ) -> Result<(), VaultError> {
        if amount < 0 {
            return Err(VaultError::Overflow);
        }
        let from_bal = self.balances.get_mut(from).copied().unwrap_or(0);
        if from_bal < amount {
            return Err(VaultError::InsufficientBalance);
        }
        *self.balances.entry(*from).or_insert(0) = from_bal - amount;
        let to_bal = self.balances.entry(*to).or_insert(0);
        *to_bal = to_bal.checked_add(amount).ok_or(VaultError::Overflow)?;
        Ok(())
    }
}

/// Outcome of a successful slash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashOutcome {
    pub position_id: u64,
    pub slashed_amount: Amount,
    pub remainder: Amount,
}

/// The vault contract. All mutations are atomic: on error nothing changes.
#[derive(Debug, Clone)]
pub struct Vault {
    config: VaultConfig,
    positions: HashMap<u64, Position>,
    next_id: u64,
    /// Collateral carved out by slashes, reserved for the notifier.
    slashed_reserve: Amount,
    /// Yield pool awaiting proportional distribution.
    yield_pool: Amount,
    ledger: TokenLedger,
}

impl Vault {
    /// Sets up a new vault. Mirrors a Soroban `initialize` entrypoint.
    pub fn initialize(config: VaultConfig) -> Result<Self, VaultError> {
        config.validate()?;
        Ok(Vault {
            config,
            positions: HashMap::new(),
            next_id: 1,
            slashed_reserve: 0,
            yield_pool: 0,
            ledger: TokenLedger::new(),
        })
    }

    pub fn config(&self) -> &VaultConfig {
        &self.config
    }

    pub fn position(&self, id: u64) -> Option<&Position> {
        self.positions.get(&id)
    }

    pub fn positions(&self) -> impl Iterator<Item = &Position> {
        self.positions.values()
    }

    pub fn slashed_reserve(&self) -> Amount {
        self.slashed_reserve
    }

    pub fn yield_pool(&self) -> Amount {
        self.yield_pool
    }

    pub fn token_balance(&self) -> Amount {
        self.ledger.balance_of(&vault_address())
    }

    /// Registers an authorized slashing reporter (must be `admin`).
    pub fn add_reporter(
        &mut self,
        caller: Address,
        reporter: Address,
        verifying_key: [u8; 32],
    ) -> Result<(), VaultError> {
        if caller != self.config.admin {
            return Err(VaultError::Unauthorized);
        }
        self.config.reporters.insert(reporter, verifying_key);
        Ok(())
    }

    /// Funds `addr` out of the vault's own (simulated) issuance, for tests.
    pub fn fund(&mut self, addr: Address, amount: Amount) -> Result<(), VaultError> {
        self.ledger.mint(addr, amount)
    }

    /// Deposits collateral on behalf of `operator`, creating a lockup position
    /// for `node_id`. Funds are pulled from the operator's own balance.
    pub fn deposit(
        &mut self,
        operator: Address,
        node_id: Address,
        node_vk: [u8; 32],
        amount: Amount,
        now: LedgerTime,
    ) -> Result<u64, VaultError> {
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        self.ledger.transfer(&operator, &vault_address(), amount)?;
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(VaultError::Overflow)?;
        self.positions.insert(
            id,
            Position {
                id,
                operator,
                node_id,
                node_vk,
                deposit: amount,
                slashed: 0,
                claimed: 0,
                released: 0,
                unclaimed_yield: 0,
                locked_at: now,
                lockup_until: now
                    .checked_add(self.config.lockup_window)
                    .ok_or(VaultError::Overflow)?,
                status: PositionStatus::Locked,
            },
        );
        Ok(id)
    }

    /// Fee-in stream: a keeper admits `amount` to the yield pool (pulled from
    /// the keeper's balance).
    pub fn credit_yield(&mut self, keeper: Address, amount: Amount) -> Result<(), VaultError> {
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        self.ledger.transfer(&keeper, &vault_address(), amount)?;
        self.yield_pool = self
            .yield_pool
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?;
        Ok(())
    }

    /// Proportionally distributes the whole yield pool across active positions
    /// weighted by remaining collateral. Exact sums (largest remainder method)
    /// guarantee the solvency invariant.
    pub fn distribute_yield(&mut self) -> Result<(), VaultError> {
        let actives: Vec<u64> = self
            .positions
            .iter()
            .filter(|(_, p)| p.has_remainder())
            .map(|(id, _)| *id)
            .collect();
        if actives.is_empty() {
            return Ok(()); // yield stays claimable via reclaim_yield_pool
        }
        let total_weight = actives.iter().try_fold(0i128, |acc, id| {
            acc.checked_add(self.positions[id].remainder())
                .ok_or(VaultError::Overflow)
        })?;
        if total_weight == 0 {
            return Ok(());
        }

        // Floor shares: floor(pool * weight_i / total_weight). Because each
        // share is the floor of a real number whose sum is exactly `pool`, the
        // residual is strictly < #actives, so the largest-remainder method
        // distributes the leftover exactly and `Σshares == pool` (deterministic
        // by id).
        let mut shares: Vec<(u64, Amount)> = actives
            .iter()
            .map(|id| {
                let r = self.positions[id].remainder();
                let share =
                    r.checked_mul(self.yield_pool).ok_or(VaultError::Overflow)? / total_weight;
                Ok((*id, share))
            })
            .collect::<Result<Vec<(u64, Amount)>, VaultError>>()?;
        shares.sort_by_key(|(id, _)| *id);

        let floors_sum: Amount = shares.iter().map(|(_, s)| s).sum();
        let mut leftover = self.yield_pool - floors_sum;
        for (id, share) in shares.iter_mut() {
            if leftover > 0 {
                *share += 1;
                leftover -= 1;
            }
            let pos = self.positions.get_mut(id).ok_or(VaultError::Overflow)?;
            pos.unclaimed_yield = pos
                .unclaimed_yield
                .checked_add(*share)
                .ok_or(VaultError::Overflow)?;
        }
        debug_assert_eq!(leftover, 0);
        self.yield_pool = 0;
        Ok(())
    }

    /// Operator pulls their proportional, accrued yield.
    pub fn claim_yield(
        &mut self,
        operator: Address,
        position_id: u64,
    ) -> Result<Amount, VaultError> {
        let pos = self
            .positions
            .get_mut(&position_id)
            .ok_or(VaultError::PositionNotFound)?;
        if pos.operator != operator {
            return Err(VaultError::Unauthorized);
        }
        let amount = pos.unclaimed_yield;
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        pos.unclaimed_yield = 0;
        self.ledger.transfer(&vault_address(), &operator, amount)?;
        Ok(amount)
    }

    /// Admin pulls an idle yield pool left over when there were no active
    /// positions to distribute to (used to keep the pool from being stuck).
    pub fn reclaim_yield_pool(&mut self, caller: Address) -> Result<Amount, VaultError> {
        if caller != self.config.admin {
            return Err(VaultError::Unauthorized);
        }
        let amount = self.yield_pool;
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        self.yield_pool = 0;
        self.ledger.transfer(&vault_address(), &caller, amount)?;
        Ok(amount)
    }

    /// Applies an authenticated **downtime** slashing proof.
    ///
    /// Slashing proceeds only if: the proof targets a locked position, the
    /// proof is fresh and signed by an authorized reporter, and the lockup has
    /// not yet expired (otherwise the operator just withdraws).
    pub fn submit_downtime_proof(
        &mut self,
        proof: Proof,
        now: LedgerTime,
    ) -> Result<SlashOutcome, VaultError> {
        let reporter_vk = self
            .config
            .reporters
            .get(&proof.reporter)
            .ok_or_else(|| VaultError::InvalidProof("reporter not authorized".into()))?;
        slashing::verify_reporter_signature(
            &proof,
            reporter_vk,
            now,
            self.config.slashing_staleness,
        )?;
        let id = self.find_position_for_node(&proof.node_id)?;
        self.slash_locked(id, self.config.downtime_slash_bps, now)
    }

    /// Applies an authenticated **double-sign** slashing proof using the
    /// accused node's own consensus key.
    pub fn submit_double_sign_proof(
        &mut self,
        material: &DoubleSignMaterial,
        node_id: Address,
        slot: u64,
        now: LedgerTime,
    ) -> Result<SlashOutcome, VaultError> {
        let id = self.find_position_for_node(&node_id)?;
        let node_vk = {
            let pos = self
                .positions
                .get(&id)
                .ok_or(VaultError::PositionNotFound)?;
            pos.node_vk
        };
        if node_vk != material.node_vk_bytes {
            return Err(VaultError::InvalidProof(
                "node keys do not match the secured position".into(),
            ));
        }
        slashing::verify_double_sign(material, &node_id, slot)?;
        self.slash_locked(id, self.config.double_sign_slash_bps, now)
    }

    /// Opens a dispute against a locked position within the dispute window,
    /// freezing release until it is resolved.
    pub fn open_dispute(
        &mut self,
        caller: Address,
        position_id: u64,
        now: LedgerTime,
    ) -> Result<(), VaultError> {
        if caller != self.config.admin {
            return Err(VaultError::Unauthorized);
        }
        let pos = self
            .positions
            .get_mut(&position_id)
            .ok_or(VaultError::PositionNotFound)?;
        if now.saturating_sub(pos.locked_at) > self.config.dispute_window {
            return Err(VaultError::NotFresh);
        }
        if pos.status != PositionStatus::Locked {
            return Err(VaultError::WrongStatus(pos.status.to_string()));
        }
        pos.status = PositionStatus::Disputed;
        Ok(())
    }

    /// Admin resolves a dispute. `accepted == true` forfeits the operator's
    /// remaining collateral to the notifier; `false` restores the position so
    /// the operator can release after the lockup expires.
    pub fn resolve_dispute(
        &mut self,
        caller: Address,
        position_id: u64,
        accepted: bool,
        _now: LedgerTime,
    ) -> Result<(), VaultError> {
        if caller != self.config.admin {
            return Err(VaultError::Unauthorized);
        }
        let pos = self
            .positions
            .get_mut(&position_id)
            .ok_or(VaultError::PositionNotFound)?;
        if pos.status != PositionStatus::Disputed {
            return Err(VaultError::WrongStatus(pos.status.to_string()));
        }
        if accepted {
            let amount = pos.remainder();
            pos.slashed = pos
                .slashed
                .checked_add(amount)
                .ok_or(VaultError::Overflow)?;
            self.slashed_reserve = self
                .slashed_reserve
                .checked_add(amount)
                .ok_or(VaultError::Overflow)?;
            pos.status = PositionStatus::Forfeited;
        } else {
            pos.status = PositionStatus::Locked;
        }
        Ok(())
    }

    /// Pull-based release: the operator withdraws 100% of their remaining
    /// (non-slashed) collateral once the lockup has expired.
    pub fn release_collateral(
        &mut self,
        operator: Address,
        position_id: u64,
        now: LedgerTime,
    ) -> Result<Amount, VaultError> {
        let pos = self
            .positions
            .get_mut(&position_id)
            .ok_or(VaultError::PositionNotFound)?;
        if pos.operator != operator {
            return Err(VaultError::Unauthorized);
        }
        if !matches!(pos.status, PositionStatus::Locked | PositionStatus::Slashed) {
            return Err(VaultError::WrongStatus(pos.status.to_string()));
        }
        if now < pos.lockup_until {
            return Err(VaultError::LockupNotExpired);
        }
        let amount = pos.remainder();
        if amount <= 0 {
            return Err(VaultError::WrongStatus("nothing left to release".into()));
        }
        pos.released = pos
            .released
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?; // remainder -> 0
        pos.status = PositionStatus::Released;
        self.ledger.transfer(&vault_address(), &operator, amount)?;
        Ok(amount)
    }

    /// Notifier pulls slashed collateral.
    pub fn claim_slashed(
        &mut self,
        caller: Address,
        position_id: u64,
    ) -> Result<Amount, VaultError> {
        if caller != self.config.notifier {
            return Err(VaultError::Unauthorized);
        }
        let pos = self
            .positions
            .get_mut(&position_id)
            .ok_or(VaultError::PositionNotFound)?;
        let amount = pos.slashed;
        if amount <= 0 {
            return Err(VaultError::WrongStatus("nothing slashed to claim".into()));
        }
        // Move the carved collateral out of the notifier's unclaimed bucket and
        // into the already-claimed bucket; the operator's remainder is unchanged.
        pos.slashed = pos
            .slashed
            .checked_sub(amount)
            .ok_or(VaultError::Overflow)?;
        pos.claimed = pos
            .claimed
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?;
        self.slashed_reserve = self
            .slashed_reserve
            .checked_sub(amount)
            .ok_or(VaultError::Overflow)?;
        self.ledger.transfer(&vault_address(), &caller, amount)?;
        Ok(amount)
    }

    /// Total outstanding liabilities of the vault. The solvency invariant is:
    /// `ledger.balance_of(&vault_address()) >= liabilities`, with equality in
    /// steady state.
    pub fn liabilities(&self) -> Amount {
        let active: Amount = self
            .positions
            .values()
            .filter(|p| p.has_remainder())
            .map(|p| p.remainder())
            .sum();
        let unclaimed_yield: Amount = self.positions.values().map(|p| p.unclaimed_yield).sum();
        active + self.slashed_reserve + self.yield_pool + unclaimed_yield
    }

    /// Total-solvency check: all deposits are backed by real token balance.
    pub fn assert_solvent(&self) -> Result<(), VaultError> {
        let balance = self.token_balance();
        let liabilities = self.liabilities();
        if balance < liabilities {
            return Err(VaultError::InsufficientBalance);
        }
        // After every operation the vault should hold exactly what it owes
        // (no surplus is created or destroyed by the state machine).
        if balance != liabilities {
            return Err(VaultError::Overflow);
        }
        Ok(())
    }

    // ---- internal helpers -------------------------------------------------

    fn find_position_for_node(&self, node_id: &Address) -> Result<u64, VaultError> {
        self.positions
            .iter()
            .find(|(_, p)| p.node_id == *node_id && p.status == PositionStatus::Locked)
            .map(|(id, _)| *id)
            .ok_or(VaultError::PositionNotFound)
    }

    /// Applies a slash to a `Locked` position before its lockup expires.
    fn slash_locked(
        &mut self,
        position_id: u64,
        bps: u32,
        now: LedgerTime,
    ) -> Result<SlashOutcome, VaultError> {
        let pos = self
            .positions
            .get_mut(&position_id)
            .ok_or(VaultError::PositionNotFound)?;
        if pos.status != PositionStatus::Locked {
            return Err(VaultError::WrongStatus(pos.status.to_string()));
        }
        if now >= pos.lockup_until {
            return Err(VaultError::LockupExpired);
        }
        let base = pos.remainder();
        let slash = slash_portion(base, bps)?;
        let remaining = base - slash;
        let outcome = SlashOutcome {
            position_id: pos.id,
            slashed_amount: slash,
            remainder: remaining,
        };
        pos.slashed = pos.slashed.checked_add(slash).ok_or(VaultError::Overflow)?;
        self.slashed_reserve = self
            .slashed_reserve
            .checked_add(slash)
            .ok_or(VaultError::Overflow)?;
        if remaining == 0 {
            pos.status = PositionStatus::Forfeited;
        } else {
            pos.status = PositionStatus::Slashed;
        }
        Ok(outcome)
    }
}

/// `bps` (basis points, 0..=10_000) of `base`, rounded down. Returns 0 for a 0
/// bps cut and `base` for 10_000 bps.
fn slash_portion(base: Amount, bps: u32) -> Result<Amount, VaultError> {
    if bps > 10_000 {
        return Err(VaultError::InvalidProof(
            "slash basis points must be in [0, 10000]".into(),
        ));
    }
    base.checked_mul(bps as Amount)
        .map(|n| n / 10_000)
        .ok_or(VaultError::Overflow)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn test_config() -> VaultConfig {
        VaultConfig {
            admin: Address::from_bytes([1u8; 32]),
            notifier: Address::from_bytes([2u8; 32]),
            lockup_window: 100,
            dispute_window: 30,
            slashing_staleness: 50,
            downtime_slash_bps: 10_000,
            double_sign_slash_bps: 10_000,
            reporters: HashMap::new(),
        }
    }

    #[test]
    fn deposit_release_roundtrip() {
        let mut v = Vault::initialize(test_config()).unwrap();
        let operator = Address::from_bytes([3u8; 32]);
        let node = Address::from_bytes([4u8; 32]);
        v.fund(operator, 5000).unwrap();
        let id = v.deposit(operator, node, [0u8; 32], 5000, 0).unwrap();
        // cannot release before expiry
        assert_eq!(
            v.release_collateral(operator, id, 99),
            Err(VaultError::LockupNotExpired)
        );
        // after expiry the non-faulty operator withdraws exactly 100%
        assert_eq!(v.release_collateral(operator, id, 100), Ok(5000));
        assert_eq!(v.ledger.balance_of(&operator), 5000);
        v.assert_solvent().unwrap();
    }
}
