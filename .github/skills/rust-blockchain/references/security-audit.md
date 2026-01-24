# Security Audit Checklist for Blockchain

Comprehensive security checklist for auditing blockchain applications in Rust.

## Smart Contract Security

### Integer Overflow/Underflow

```rust
// ❌ BAD: Unchecked arithmetic
pub fn transfer(&mut self, amount: u128) {
    self.balance = self.balance - amount;  // Can underflow!
}

// ✅ GOOD: Checked arithmetic
pub fn transfer(&mut self, amount: u128) -> Result<()> {
    self.balance = self.balance
        .checked_sub(amount)
        .ok_or(Error::InsufficientBalance)?;
    Ok(())
}

// ✅ GOOD: Saturating arithmetic (when overflow should cap)
pub fn add_reward(&mut self, amount: u128) {
    self.balance = self.balance.saturating_add(amount);
}
```

### Reentrancy Attacks

```rust
// ❌ BAD: State updated after external call
pub fn withdraw(&mut self, amount: u128) -> Result<()> {
    let balance = self.balances.get(&sender)?;
    
    // External call before state update
    self.call_external(sender, amount)?;
    
    // State update after - vulnerable to reentrancy!
    self.balances.insert(sender, balance - amount);
    Ok(())
}

// ✅ GOOD: Checks-Effects-Interactions pattern
pub fn withdraw(&mut self, amount: u128) -> Result<()> {
    // Checks
    let balance = self.balances.get(&sender)?;
    if balance < amount {
        return Err(Error::InsufficientBalance);
    }
    
    // Effects
    self.balances.insert(sender, balance - amount);
    
    // Interactions
    self.call_external(sender, amount)?;
    Ok(())
}

// ✅ BEST: Reentrancy guard
pub struct ReentrancyGuard {
    locked: Cell<bool>,
}

impl ReentrancyGuard {
    pub fn lock(&self) -> Result<Guard> {
        if self.locked.get() {
            return Err(Error::Reentrant);
        }
        self.locked.set(true);
        Ok(Guard { guard: self })
    }
}

pub fn withdraw(&mut self, amount: u128) -> Result<()> {
    let _guard = self.reentrancy_guard.lock()?;
    
    let balance = self.balances.get(&sender)?;
    self.balances.insert(sender, balance - amount);
    self.call_external(sender, amount)?;
    Ok(())
}
```

### Front-Running Protection

```rust
// ❌ BAD: Vulnerable to front-running
pub fn buy_nft(&mut self, nft_id: u64, max_price: u128) -> Result<()> {
    let price = self.get_price(nft_id);
    if price <= max_price {
        self.transfer_nft(nft_id, sender)?;
        self.transfer_payment(price)?;
    }
    Ok(())
}

// ✅ GOOD: Commit-reveal scheme
pub fn commit_bid(&mut self, commitment: [u8; 32]) {
    self.commitments.insert(sender, Commitment {
        hash: commitment,
        timestamp: self.current_time(),
    });
}

pub fn reveal_bid(&mut self, nft_id: u64, price: u128, nonce: [u8; 32]) -> Result<()> {
    let commitment = self.commitments.get(&sender)?;
    
    // Verify commitment
    let expected_hash = blake3::hash(&bincode::serialize(&(nft_id, price, nonce))?);
    if commitment.hash != expected_hash {
        return Err(Error::InvalidCommitment);
    }
    
    // Process bid
    self.process_bid(nft_id, price)?;
    Ok(())
}
```

### Access Control

```rust
// ❌ BAD: No access control
pub fn set_config(&mut self, config: Config) {
    self.config = config;
}

// ✅ GOOD: Role-based access control
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Owner,
    Admin,
    User,
}

pub struct AccessControl {
    roles: HashMap<Address, Role>,
}

impl AccessControl {
    pub fn only_role(&self, address: &Address, role: Role) -> Result<()> {
        let user_role = self.roles.get(address).ok_or(Error::NoRole)?;
        if *user_role != role {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}

pub fn set_config(&mut self, sender: Address, config: Config) -> Result<()> {
    self.access_control.only_role(&sender, Role::Admin)?;
    self.config = config;
    Ok(())
}

// ✅ BETTER: Multi-sig for critical operations
pub struct MultiSig {
    threshold: usize,
    signers: Vec<Address>,
    proposals: HashMap<[u8; 32], Proposal>,
}

pub fn propose_config_change(&mut self, config: Config) -> Result<[u8; 32]> {
    let proposal_id = blake3::hash(&bincode::serialize(&config)?).into();
    self.proposals.insert(proposal_id, Proposal {
        data: config,
        approvals: vec![sender],
        executed: false,
    });
    Ok(proposal_id)
}

pub fn approve_proposal(&mut self, proposal_id: [u8; 32]) -> Result<()> {
    let proposal = self.proposals.get_mut(&proposal_id)?;
    
    if !self.signers.contains(&sender) {
        return Err(Error::NotSigner);
    }
    
    proposal.approvals.push(sender);
    
    if proposal.approvals.len() >= self.threshold {
        self.execute_proposal(proposal_id)?;
    }
    
    Ok(())
}
```

### Gas Limit DoS

```rust
// ❌ BAD: Unbounded loop
pub fn distribute_rewards(&mut self, recipients: Vec<Address>) {
    for recipient in recipients {
        self.transfer(recipient, self.reward_amount);
    }
}

// ✅ GOOD: Bounded operations with pagination
pub fn distribute_rewards(
    &mut self,
    start_index: usize,
    batch_size: usize,
) -> Result<bool> {
    let end_index = (start_index + batch_size).min(self.recipients.len());
    
    for i in start_index..end_index {
        let recipient = self.recipients[i];
        self.transfer(recipient, self.reward_amount)?;
    }
    
    Ok(end_index >= self.recipients.len())  // Returns true if done
}

// ✅ BETTER: Pull payment pattern
pub fn claim_reward(&mut self) -> Result<()> {
    let reward = self.pending_rewards.get(&sender)?;
    self.pending_rewards.remove(&sender);
    self.transfer(sender, reward)?;
    Ok(())
}
```

## Cryptographic Security

### Secure Random Number Generation

```rust
// ❌ BAD: Predictable randomness
pub fn random(&self, seed: u64) -> u64 {
    seed.wrapping_mul(1103515245).wrapping_add(12345)
}

// ✅ GOOD: VRF for on-chain randomness
use vrf::openssl::{CipherSuite, ECVRF};

pub fn generate_random(&self, seed: &[u8]) -> Result<([u8; 32], [u8; 80])> {
    let mut vrf = ECVRF::from_suite(CipherSuite::SECP256K1_SHA256_TAI)?;
    let (output, proof) = vrf.prove(seed)?;
    Ok((output, proof))
}

pub fn verify_random(
    &self,
    seed: &[u8],
    output: &[u8; 32],
    proof: &[u8; 80],
    public_key: &[u8],
) -> Result<bool> {
    let mut vrf = ECVRF::from_suite(CipherSuite::SECP256K1_SHA256_TAI)?;
    Ok(vrf.verify(public_key, seed, output, proof)?)
}
```

### Signature Verification

```rust
// ❌ BAD: No replay protection
pub fn execute_transaction(&mut self, tx: Transaction, sig: Signature) -> Result<()> {
    self.verify_signature(&tx, &sig)?;
    self.apply_transaction(tx)?;
    Ok(())
}

// ✅ GOOD: With nonce and chain ID
#[derive(Serialize)]
pub struct SignedTransaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub from: Address,
    pub to: Address,
    pub value: u128,
    pub data: Vec<u8>,
}

pub fn execute_transaction(&mut self, tx: SignedTransaction, sig: Signature) -> Result<()> {
    // Verify chain ID
    if tx.chain_id != self.chain_id {
        return Err(Error::WrongChain);
    }
    
    // Verify nonce
    let expected_nonce = self.get_nonce(&tx.from)?;
    if tx.nonce != expected_nonce {
        return Err(Error::InvalidNonce);
    }
    
    // Verify signature
    self.verify_signature(&tx, &sig)?;
    
    // Increment nonce to prevent replay
    self.set_nonce(&tx.from, expected_nonce + 1)?;
    
    self.apply_transaction(tx)?;
    Ok(())
}
```

### Constant-Time Comparisons

```rust
// ❌ BAD: Timing attack vulnerable
pub fn verify_secret(&self, input: &[u8]) -> bool {
    input == self.secret.as_slice()
}

// ✅ GOOD: Constant-time comparison
use subtle::ConstantTimeEq;

pub fn verify_secret(&self, input: &[u8]) -> bool {
    if input.len() != self.secret.len() {
        return false;
    }
    input.ct_eq(&self.secret).into()
}
```

## Network Security

### Rate Limiting

```rust
use std::time::{Duration, Instant};

pub struct RateLimiter {
    requests: HashMap<PeerId, VecDeque<Instant>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn check_rate_limit(&mut self, peer: PeerId) -> Result<()> {
        let now = Instant::now();
        let requests = self.requests.entry(peer).or_default();
        
        // Remove old requests outside window
        while requests.front()
            .map(|t| now.duration_since(*t) > self.window)
            .unwrap_or(false)
        {
            requests.pop_front();
        }
        
        // Check limit
        if requests.len() >= self.max_requests {
            return Err(Error::RateLimitExceeded);
        }
        
        requests.push_back(now);
        Ok(())
    }
}
```

### Input Validation

```rust
// Validate all external inputs
pub fn handle_network_message(&mut self, msg: NetworkMessage) -> Result<()> {
    match msg {
        NetworkMessage::NewBlock(block) => {
            // Size limits
            if block.transactions.len() > MAX_TXS_PER_BLOCK {
                return Err(Error::TooManyTransactions);
            }
            
            if bincode::serialize(&block)?.len() > MAX_BLOCK_SIZE {
                return Err(Error::BlockTooLarge);
            }
            
            // Validate block header
            if block.height == 0 || block.height > self.current_height + 1 {
                return Err(Error::InvalidBlockHeight);
            }
            
            // Verify signatures
            self.verify_block_signature(&block)?;
            
            self.process_block(block)?;
        }
        _ => {}
    }
    Ok(())
}
```

### Eclipse Attack Prevention

```rust
pub struct PeerManager {
    inbound_peers: HashSet<PeerId>,
    outbound_peers: HashSet<PeerId>,
    max_inbound: usize,
    max_outbound: usize,
}

impl PeerManager {
    pub fn accept_connection(&mut self, peer: PeerId, direction: Direction) -> Result<()> {
        match direction {
            Direction::Inbound => {
                if self.inbound_peers.len() >= self.max_inbound {
                    return Err(Error::TooManyInboundPeers);
                }
                
                // Limit peers from same subnet
                let subnet = self.get_subnet(&peer);
                let same_subnet_count = self.inbound_peers.iter()
                    .filter(|p| self.get_subnet(p) == subnet)
                    .count();
                
                if same_subnet_count >= MAX_PEERS_PER_SUBNET {
                    return Err(Error::SubnetLimitExceeded);
                }
                
                self.inbound_peers.insert(peer);
            }
            Direction::Outbound => {
                // Maintain diverse outbound connections
                if self.outbound_peers.len() >= self.max_outbound {
                    return Err(Error::TooManyOutboundPeers);
                }
                self.outbound_peers.insert(peer);
            }
        }
        Ok(())
    }
}
```

## State Machine Security

### State Transition Validation

```rust
pub fn validate_state_transition(
    &self,
    old_state: &State,
    new_state: &State,
    transactions: &[Transaction],
) -> Result<()> {
    // Verify state root
    let computed_root = self.compute_state_root(new_state);
    if computed_root != new_state.root {
        return Err(Error::InvalidStateRoot);
    }
    
    // Verify all transactions were applied correctly
    let mut simulated_state = old_state.clone();
    for tx in transactions {
        self.apply_transaction(&mut simulated_state, tx)?;
    }
    
    if simulated_state != *new_state {
        return Err(Error::StateTransitionMismatch);
    }
    
    Ok(())
}
```

### Atomic State Updates

```rust
pub struct StateTransaction<'a> {
    state: &'a mut State,
    changes: Vec<StateChange>,
    committed: bool,
}

impl<'a> StateTransaction<'a> {
    pub fn new(state: &'a mut State) -> Self {
        Self {
            state,
            changes: vec![],
            committed: false,
        }
    }
    
    pub fn set(&mut self, key: Key, value: Value) {
        self.changes.push(StateChange::Set { key, value });
    }
    
    pub fn commit(mut self) -> Result<()> {
        for change in &self.changes {
            match change {
                StateChange::Set { key, value } => {
                    self.state.db.put(key, value)?;
                }
                StateChange::Delete { key } => {
                    self.state.db.delete(key)?;
                }
            }
        }
        self.committed = true;
        Ok(())
    }
}

impl<'a> Drop for StateTransaction<'a> {
    fn drop(&mut self) {
        if !self.committed && !std::thread::panicking() {
            // Rollback changes
            tracing::warn!("State transaction dropped without commit, rolling back");
        }
    }
}
```

## Audit Tools

### Automated Testing

```rust
#[cfg(test)]
mod security_tests {
    use proptest::prelude::*;
    
    proptest! {
        #[test]
        fn test_no_overflow_in_arithmetic(
            a in 0u128..u128::MAX / 2,
            b in 0u128..u128::MAX / 2,
        ) {
            let account = Account { balance: a };
            
            // All operations should either succeed or return error
            let result = account.add_balance(b);
            assert!(result.is_ok() || result.is_err());
            
            // Should never panic
        }
        
        #[test]
        fn test_signature_always_verifiable(
            nonce in any::<u64>(),
            value in any::<u128>(),
        ) {
            let keypair = Keypair::generate(&mut OsRng);
            let tx = Transaction { nonce, value, /* ... */ };
            let signature = keypair.sign(&tx);
            
            // Signature must verify
            assert!(verify_signature(&tx, &signature, &keypair.public()).is_ok());
        }
    }
}
```

### Fuzzing

```rust
// cargo-fuzz target
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(tx) = bincode::deserialize::<Transaction>(data) {
        let mut blockchain = Blockchain::new_test();
        
        // Should never panic, even with malicious input
        let _ = blockchain.validate_transaction(&tx);
    }
});
```

### Static Analysis

```toml
# Cargo.toml
[dependencies]
# Use only audited crypto libraries
ed25519-dalek = { version = "2.1", features = ["hazmat"] }
sha3 = "0.10"

[dev-dependencies]
cargo-audit = "0.18"
cargo-deny = "0.14"
```

```bash
# Run security checks
cargo audit          # Check for known vulnerabilities
cargo deny check     # Check licenses and dependencies
cargo clippy -- -D warnings  # Strict linting
```
