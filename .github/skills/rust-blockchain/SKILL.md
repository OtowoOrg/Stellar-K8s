---
name: rust-blockchain
description: Expert guidance for building sophisticated blockchain applications using Rust. Use this skill for blockchain development, smart contracts, consensus mechanisms, cryptography, distributed systems, performance optimization, security best practices, and production-ready blockchain architecture.
license: Complete terms in LICENSE.txt
---

# Rust Blockchain Development

Comprehensive guidance for building production-grade blockchain applications with Rust, covering architecture, security, performance, and industry best practices.

## When to Use This Skill

- Building blockchain infrastructure (nodes, validators, consensus engines)
- Smart contract development (Substrate, Solana, NEAR, or custom chains)
- Cryptographic primitives and signature schemes
- P2P networking and gossip protocols
- State machines and transaction processing
- Performance optimization for blockchain workloads
- Security auditing and vulnerability mitigation
- Testing strategies for distributed systems

## Core Principles

### Memory Safety & Zero-Cost Abstractions

Rust's ownership model prevents entire classes of vulnerabilities critical in blockchain:

```rust
// Ownership prevents double-spending bugs at compile time
pub struct Transaction {
    nonce: u64,
    from: Address,
    to: Address,
    value: u128,
}

// Move semantics ensure transactions can't be replayed
impl Block {
    pub fn add_transaction(&mut self, tx: Transaction) {
        self.transactions.push(tx); // tx moved, can't be reused
    }
}
```

### Type-Driven Development

Use Rust's type system to enforce protocol invariants:

```rust
// Phantom types for compile-time state verification
pub struct Unsigned;
pub struct Signed;

pub struct Transaction<State> {
    data: TxData,
    _marker: PhantomData<State>,
}

impl Transaction<Unsigned> {
    pub fn sign(self, key: &PrivateKey) -> Transaction<Signed> {
        // Transition from unsigned to signed state
    }
}

// Only signed transactions can be broadcast
impl Transaction<Signed> {
    pub fn broadcast(&self, network: &Network) { }
}
```

### Error Handling

Never panic in blockchain code. Use `Result` and `Option` with custom error types:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BlockchainError {
    #[error("Invalid signature: {0}")]
    InvalidSignature(String),
    #[error("Insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: u128, need: u128 },
    #[error("Nonce mismatch: expected {expected}, got {got}")]
    NonceMismatch { expected: u64, got: u64 },
    #[error("Block validation failed: {0}")]
    ValidationError(String),
    #[error(transparent)]
    CryptoError(#[from] ed25519_dalek::SignatureError),
}

pub type BlockchainResult<T> = Result<T, BlockchainError>;
```

## Project Architecture

### Standard Structure

```
blockchain-project/
├── Cargo.toml
├── node/              # Full node implementation
│   ├── src/
│   │   ├── main.rs
│   │   ├── cli.rs
│   │   └── config.rs
├── consensus/         # Consensus algorithm
│   ├── src/
│   │   ├── lib.rs
│   │   ├── pbft.rs    # or raft.rs, tendermint.rs, etc.
│   │   └── types.rs
├── chain/             # Core blockchain logic
│   ├── src/
│   │   ├── lib.rs
│   │   ├── block.rs
│   │   ├── transaction.rs
│   │   └── state.rs
├── crypto/            # Cryptographic primitives
│   ├── src/
│   │   ├── lib.rs
│   │   ├── hash.rs
│   │   ├── signature.rs
│   │   └── merkle.rs
├── network/           # P2P networking
│   ├── src/
│   │   ├── lib.rs
│   │   ├── peer.rs
│   │   └── protocol.rs
├── storage/           # Database layer
│   ├── src/
│   │   ├── lib.rs
│   │   ├── rocksdb.rs
│   │   └── state_db.rs
└── runtime/           # Smart contract execution
    ├── src/
        ├── lib.rs
        ├── vm.rs
        └── wasm.rs
```

### Cargo.toml Best Practices

```toml
[package]
name = "blockchain-project"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[workspace]
members = ["node", "consensus", "chain", "crypto", "network", "storage", "runtime"]

[dependencies]
# Async runtime - Tokio is standard for blockchain
tokio = { version = "1.35", features = ["full"] }
# Serialization - bincode for binary, serde_json for RPC
serde = { version = "1.0", features = ["derive"] }
bincode = "1.3"
# Cryptography
ed25519-dalek = "2.1"
sha3 = "0.10"
blake3 = "1.5"
# Networking
libp2p = "0.53"
# Storage
rocksdb = "0.21"
# Logging
tracing = "0.1"
tracing-subscriber = "0.3"
# Error handling
thiserror = "1.0"
anyhow = "1.0"

[dev-dependencies]
proptest = "1.4"
criterion = "0.5"

[profile.release]
lto = "thin"              # Link-time optimization
codegen-units = 1         # Better optimization
opt-level = 3             # Maximum optimization
panic = "abort"           # Smaller binary, faster panics
strip = true              # Remove debug symbols

[profile.bench]
inherits = "release"
debug = true              # Keep symbols for profiling
```

## Cryptography

### Hashing

Always use cryptographically secure hash functions:

```rust
use blake3::Hasher;
use sha3::{Digest, Keccak256};

pub fn hash_block(block: &Block) -> [u8; 32] {
    // BLAKE3 is fastest for general use
    let mut hasher = Hasher::new();
    hasher.update(&bincode::serialize(block).unwrap());
    *hasher.finalize().as_bytes()
}

pub fn hash_ethereum_style(data: &[u8]) -> [u8; 32] {
    // Keccak256 for Ethereum compatibility
    Keccak256::digest(data).into()
}
```

### Digital Signatures

Ed25519 for performance, ECDSA (secp256k1) for Bitcoin/Ethereum compatibility:

```rust
use ed25519_dalek::{Keypair, Signature, Signer, Verifier};

pub struct Account {
    keypair: Keypair,
    address: Address,
}

impl Account {
    pub fn sign_transaction(&self, tx: &Transaction) -> Signature {
        let message = bincode::serialize(tx).unwrap();
        self.keypair.sign(&message)
    }
}

pub fn verify_signature(
    tx: &Transaction,
    signature: &Signature,
    public_key: &PublicKey,
) -> BlockchainResult<()> {
    let message = bincode::serialize(tx)?;
    public_key
        .verify(&message, signature)
        .map_err(|e| BlockchainError::InvalidSignature(e.to_string()))
}
```

### Merkle Trees

Essential for efficient state proofs:

```rust
pub struct MerkleTree {
    leaves: Vec<[u8; 32]>,
    nodes: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    pub fn new(data: Vec<&[u8]>) -> Self {
        let leaves: Vec<[u8; 32]> = data
            .iter()
            .map(|d| blake3::hash(d).into())
            .collect();
        
        let mut nodes = vec![leaves.clone()];
        let mut current_level = leaves;
        
        while current_level.len() > 1 {
            let mut next_level = vec![];
            for chunk in current_level.chunks(2) {
                let hash = if chunk.len() == 2 {
                    blake3::hash(&[chunk[0], chunk[1]].concat())
                } else {
                    blake3::hash(&chunk[0])
                };
                next_level.push(hash.into());
            }
            nodes.push(next_level.clone());
            current_level = next_level;
        }
        
        Self { leaves, nodes }
    }
    
    pub fn root(&self) -> [u8; 32] {
        self.nodes.last().unwrap()[0]
    }
    
    pub fn proof(&self, index: usize) -> Vec<([u8; 32], bool)> {
        // Generate Merkle proof for efficient verification
        let mut proof = vec![];
        let mut idx = index;
        
        for level in &self.nodes[..self.nodes.len() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            if sibling_idx < level.len() {
                proof.push((level[sibling_idx], idx % 2 == 0));
            }
            idx /= 2;
        }
        proof
    }
}
```

## State Management

### Account-Based Model

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub nonce: u64,
    pub balance: u128,
    pub storage: HashMap<[u8; 32], Vec<u8>>,
    pub code_hash: Option<[u8; 32]>,
}

pub struct StateManager {
    db: Arc<RocksDB>,
    cache: RwLock<HashMap<Address, Account>>,
}

impl StateManager {
    pub async fn get_account(&self, address: &Address) -> BlockchainResult<Account> {
        // Check cache first
        if let Some(account) = self.cache.read().await.get(address) {
            return Ok(account.clone());
        }
        
        // Load from database
        let key = format!("account:{}", hex::encode(address));
        let data = self.db.get(key.as_bytes())?
            .ok_or(BlockchainError::AccountNotFound)?;
        let account: Account = bincode::deserialize(&data)?;
        
        // Update cache
        self.cache.write().await.insert(*address, account.clone());
        Ok(account)
    }
    
    pub async fn apply_transaction(
        &self,
        tx: &Transaction,
    ) -> BlockchainResult<Receipt> {
        let mut from = self.get_account(&tx.from).await?;
        let mut to = self.get_account(&tx.to).await?;
        
        // Validate nonce
        if from.nonce != tx.nonce {
            return Err(BlockchainError::NonceMismatch {
                expected: from.nonce,
                got: tx.nonce,
            });
        }
        
        // Check balance
        if from.balance < tx.value + tx.gas_limit * tx.gas_price {
            return Err(BlockchainError::InsufficientBalance {
                have: from.balance,
                need: tx.value + tx.gas_limit * tx.gas_price,
            });
        }
        
        // Update state
        from.balance -= tx.value;
        from.nonce += 1;
        to.balance += tx.value;
        
        // Persist changes
        self.update_account(&tx.from, &from).await?;
        self.update_account(&tx.to, &to).await?;
        
        Ok(Receipt {
            status: true,
            gas_used: 21000,
            logs: vec![],
        })
    }
}
```

### UTXO Model

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct OutPoint {
    pub tx_hash: [u8; 32],
    pub index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxOut {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

pub struct UtxoSet {
    db: Arc<RocksDB>,
    cache: RwLock<HashMap<OutPoint, TxOut>>,
}

impl UtxoSet {
    pub async fn validate_and_update(
        &self,
        tx: &Transaction,
    ) -> BlockchainResult<()> {
        let mut total_in = 0u64;
        
        // Validate inputs and collect values
        for input in &tx.inputs {
            let utxo = self.get_utxo(&input.previous_output).await?
                .ok_or(BlockchainError::UtxoNotFound)?;
            
            // Verify signature against script
            self.verify_script(&input.script_sig, &utxo.script_pubkey, tx)?;
            total_in += utxo.value;
        }
        
        // Validate outputs
        let total_out: u64 = tx.outputs.iter().map(|o| o.value).sum();
        if total_out > total_in {
            return Err(BlockchainError::InsufficientFunds);
        }
        
        // Update UTXO set
        for input in &tx.inputs {
            self.remove_utxo(&input.previous_output).await?;
        }
        
        let tx_hash = tx.hash();
        for (index, output) in tx.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                tx_hash,
                index: index as u32,
            };
            self.add_utxo(outpoint, output.clone()).await?;
        }
        
        Ok(())
    }
}
```

## Consensus Algorithms

### Proof of Authority (PoA)

Simple and efficient for permissioned chains:

```rust
pub struct PoaConsensus {
    validators: Vec<Address>,
    current_proposer: usize,
}

impl PoaConsensus {
    pub fn can_propose(&self, address: &Address, block_height: u64) -> bool {
        let proposer_index = (block_height as usize) % self.validators.len();
        &self.validators[proposer_index] == address
    }
    
    pub fn validate_block(&self, block: &Block) -> BlockchainResult<()> {
        // Verify proposer is authorized
        if !self.can_propose(&block.proposer, block.height) {
            return Err(BlockchainError::UnauthorizedProposer);
        }
        
        // Verify block signature
        verify_signature(&block.header_hash(), &block.signature, &block.proposer)?;
        
        Ok(())
    }
}
```

### Proof of Stake (PoS)

For more decentralized chains, see [references/consensus-pos.md](references/consensus-pos.md).

### BFT Consensus

For Byzantine fault tolerance (PBFT, Tendermint), see [references/consensus-bft.md](references/consensus-bft.md).

## Networking

### P2P with libp2p

```rust
use libp2p::{
    gossipsub, identify, noise, swarm::SwarmEvent, tcp, yamux, Swarm,
};

pub async fn create_p2p_node(keypair: Keypair) -> Result<Swarm<ChainBehaviour>> {
    let peer_id = PeerId::from(keypair.public());
    
    let transport = tcp::tokio::Transport::default()
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&keypair)?)
        .multiplex(yamux::Config::default())
        .boxed();
    
    let behaviour = ChainBehaviour {
        gossipsub: gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub::Config::default(),
        )?,
        identify: identify::Behaviour::new(identify::Config::new(
            "/blockchain/1.0.0".to_string(),
            keypair.public(),
        )),
    };
    
    let swarm = Swarm::new(transport, behaviour, peer_id, SwarmConfig::default());
    Ok(swarm)
}

pub async fn handle_network_events(
    mut swarm: Swarm<ChainBehaviour>,
    chain: Arc<Blockchain>,
) {
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::Behaviour(ChainEvent::Gossipsub(
                gossipsub::Event::Message { message, .. }
            )) => {
                match bincode::deserialize::<NetworkMessage>(&message.data) {
                    Ok(NetworkMessage::NewBlock(block)) => {
                        if let Err(e) = chain.add_block(block).await {
                            tracing::error!("Failed to add block: {}", e);
                        }
                    }
                    Ok(NetworkMessage::NewTransaction(tx)) => {
                        chain.add_to_mempool(tx).await;
                    }
                    Err(e) => tracing::error!("Invalid message: {}", e),
                }
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!("Listening on {:?}", address);
            }
            _ => {}
        }
    }
}
```

## Storage

### RocksDB Integration

```rust
use rocksdb::{Options, DB};

pub struct BlockchainDB {
    db: Arc<DB>,
}

impl BlockchainDB {
    pub fn new(path: &str) -> BlockchainResult<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(10000);
        opts.increase_parallelism(num_cpus::get() as i32);
        
        let db = DB::open(&opts, path)?;
        Ok(Self { db: Arc::new(db) })
    }
    
    pub fn store_block(&self, block: &Block) -> BlockchainResult<()> {
        let key = format!("block:{}", block.height);
        let value = bincode::serialize(block)?;
        self.db.put(key.as_bytes(), value)?;
        
        // Index by hash
        let hash_key = format!("hash:{}", hex::encode(block.hash()));
        self.db.put(hash_key.as_bytes(), block.height.to_le_bytes())?;
        
        Ok(())
    }
    
    pub fn get_block(&self, height: u64) -> BlockchainResult<Option<Block>> {
        let key = format!("block:{}", height);
        match self.db.get(key.as_bytes())? {
            Some(data) => Ok(Some(bincode::deserialize(&data)?)),
            None => Ok(None),
        }
    }
}
```

## Smart Contracts

### WASM Runtime

```rust
use wasmer::{Engine, Instance, Module, Store, Value};

pub struct WasmRuntime {
    store: Store,
    engine: Engine,
}

impl WasmRuntime {
    pub fn new() -> Self {
        let engine = Engine::default();
        let store = Store::new(engine.clone());
        Self { store, engine }
    }
    
    pub fn execute_contract(
        &mut self,
        code: &[u8],
        function: &str,
        args: Vec<Value>,
        gas_limit: u64,
    ) -> BlockchainResult<Value> {
        let module = Module::new(&self.engine, code)?;
        let instance = Instance::new(&mut self.store, &module, &imports)?;
        
        let func = instance
            .exports
            .get_function(function)?;
        
        // Set gas metering
        let mut metering = instance.exports.get_global("gas_used")?;
        metering.set(&mut self.store, Value::I64(0))?;
        
        let result = func.call(&mut self.store, &args)?;
        
        // Check gas usage
        let gas_used = metering.get(&mut self.store).i64().unwrap() as u64;
        if gas_used > gas_limit {
            return Err(BlockchainError::OutOfGas);
        }
        
        Ok(result[0])
    }
}
```

## Performance Optimization

### Parallel Transaction Execution

```rust
use rayon::prelude::*;

pub async fn execute_block_parallel(
    state: &StateManager,
    transactions: &[Transaction],
) -> BlockchainResult<Vec<Receipt>> {
    // Analyze transaction dependencies
    let dependency_graph = build_dependency_graph(transactions);
    let execution_groups = topological_sort(&dependency_graph);
    
    let mut receipts = vec![];
    
    for group in execution_groups {
        // Execute independent transactions in parallel
        let group_receipts: Vec<_> = group
            .par_iter()
            .map(|&tx_index| {
                state.apply_transaction(&transactions[tx_index])
            })
            .collect::<Result<Vec<_>, _>>()?;
        
        receipts.extend(group_receipts);
    }
    
    Ok(receipts)
}
```

### Memory Pool Optimization

```rust
use std::collections::BTreeMap;

pub struct Mempool {
    // Sorted by gas price (descending) then nonce
    transactions: BTreeMap<(u128, u64), Transaction>,
    by_sender: HashMap<Address, BTreeSet<u64>>,
    max_size: usize,
}

impl Mempool {
    pub fn add_transaction(&mut self, tx: Transaction) -> BlockchainResult<()> {
        if self.transactions.len() >= self.max_size {
            // Remove lowest gas price transaction
            if let Some((key, _)) = self.transactions.iter().next() {
                let key = *key;
                if tx.gas_price > key.0 {
                    self.transactions.remove(&key);
                } else {
                    return Err(BlockchainError::MempoolFull);
                }
            }
        }
        
        let key = (tx.gas_price, tx.nonce);
        self.transactions.insert(key, tx.clone());
        self.by_sender.entry(tx.from)
            .or_default()
            .insert(tx.nonce);
        
        Ok(())
    }
    
    pub fn select_transactions(&self, gas_limit: u64) -> Vec<Transaction> {
        let mut selected = vec![];
        let mut total_gas = 0u64;
        
        for (_, tx) in self.transactions.iter().rev() {
            if total_gas + tx.gas_limit <= gas_limit {
                selected.push(tx.clone());
                total_gas += tx.gas_limit;
            }
        }
        
        selected
    }
}
```

## Security Best Practices

### Input Validation

```rust
impl Transaction {
    pub fn validate(&self) -> BlockchainResult<()> {
        // Check for zero address
        if self.to == Address::zero() && self.data.is_empty() {
            return Err(BlockchainError::InvalidRecipient);
        }
        
        // Validate value doesn't overflow
        if self.value > u128::MAX / 2 {
            return Err(BlockchainError::ValueOverflow);
        }
        
        // Check gas limits
        if self.gas_limit == 0 || self.gas_price == 0 {
            return Err(BlockchainError::InvalidGas);
        }
        
        // Signature validation
        if !self.verify_signature()? {
            return Err(BlockchainError::InvalidSignature);
        }
        
        Ok(())
    }
}
```

### Reentrancy Protection

```rust
pub struct ExecutionContext {
    call_depth: usize,
    locks: HashSet<Address>,
}

impl ExecutionContext {
    const MAX_CALL_DEPTH: usize = 1024;
    
    pub fn call_contract(
        &mut self,
        address: &Address,
        data: &[u8],
    ) -> BlockchainResult<Vec<u8>> {
        // Check call depth
        if self.call_depth >= Self::MAX_CALL_DEPTH {
            return Err(BlockchainError::CallDepthExceeded);
        }
        
        // Check for reentrancy
        if !self.locks.insert(*address) {
            return Err(BlockchainError::ReentrancyDetected);
        }
        
        self.call_depth += 1;
        let result = self.execute_internal(address, data);
        self.call_depth -= 1;
        self.locks.remove(address);
        
        result
    }
}
```

### Integer Overflow Protection

```rust
// Use checked arithmetic
impl Account {
    pub fn add_balance(&mut self, amount: u128) -> BlockchainResult<()> {
        self.balance = self.balance
            .checked_add(amount)
            .ok_or(BlockchainError::BalanceOverflow)?;
        Ok(())
    }
    
    pub fn sub_balance(&mut self, amount: u128) -> BlockchainResult<()> {
        self.balance = self.balance
            .checked_sub(amount)
            .ok_or(BlockchainError::InsufficientBalance {
                have: self.balance,
                need: amount,
            })?;
        Ok(())
    }
}
```

## Testing

### Property-Based Testing

```rust
#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    
    proptest! {
        #[test]
        fn test_state_transition_always_valid(
            initial_balance in 0u128..u128::MAX / 2,
            transfer_amount in 0u128..u128::MAX / 2,
        ) {
            let mut account = Account {
                balance: initial_balance,
                nonce: 0,
            };
            
            if transfer_amount <= initial_balance {
                assert!(account.sub_balance(transfer_amount).is_ok());
                assert_eq!(account.balance, initial_balance - transfer_amount);
            } else {
                assert!(account.sub_balance(transfer_amount).is_err());
                assert_eq!(account.balance, initial_balance);
            }
        }
        
        #[test]
        fn test_merkle_proof_always_valid(
            data in prop::collection::vec(
                prop::collection::vec(any::<u8>(), 1..100),
                1..100
            )
        ) {
            let refs: Vec<&[u8]> = data.iter().map(|v| v.as_slice()).collect();
            let tree = MerkleTree::new(refs);
            let root = tree.root();
            
            for (i, item) in data.iter().enumerate() {
                let proof = tree.proof(i);
                assert!(verify_merkle_proof(&root, item, &proof));
            }
        }
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_full_block_lifecycle() {
    let chain = Blockchain::new_test().await;
    
    // Create accounts
    let alice = Account::new();
    let bob = Account::new();
    chain.fund_account(&alice.address, 1000).await;
    
    // Create transaction
    let tx = Transaction {
        from: alice.address,
        to: bob.address,
        value: 100,
        nonce: 0,
        gas_limit: 21000,
        gas_price: 1,
        data: vec![],
    };
    let signed_tx = alice.sign_transaction(&tx);
    
    // Submit to mempool
    chain.submit_transaction(signed_tx.clone()).await.unwrap();
    
    // Mine block
    let block = chain.mine_next_block().await.unwrap();
    assert_eq!(block.transactions.len(), 1);
    assert_eq!(block.transactions[0], signed_tx);
    
    // Verify state changes
    let alice_balance = chain.get_balance(&alice.address).await.unwrap();
    let bob_balance = chain.get_balance(&bob.address).await.unwrap();
    assert_eq!(alice_balance, 900 - 21000); // minus gas
    assert_eq!(bob_balance, 100);
}
```

### Benchmark Performance

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_signature_verification(c: &mut Criterion) {
    let keypair = Keypair::generate(&mut OsRng);
    let message = b"test transaction";
    let signature = keypair.sign(message);
    
    c.bench_function("ed25519_verify", |b| {
        b.iter(|| {
            keypair.public.verify(
                black_box(message),
                black_box(&signature)
            ).unwrap()
        })
    });
}

fn benchmark_merkle_tree(c: &mut Criterion) {
    let data: Vec<Vec<u8>> = (0..1000)
        .map(|i| vec![i as u8; 32])
        .collect();
    let refs: Vec<&[u8]> = data.iter().map(|v| v.as_slice()).collect();
    
    c.bench_function("merkle_tree_1000", |b| {
        b.iter(|| {
            MerkleTree::new(black_box(refs.clone()))
        })
    });
}

criterion_group!(benches, benchmark_signature_verification, benchmark_merkle_tree);
criterion_main!(benches);
```

## Deployment

### CLI Setup

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "blockchain-node")]
#[command(about = "Blockchain node implementation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the blockchain node
    Start {
        #[arg(long, default_value = "config.toml")]
        config: String,
        #[arg(long)]
        validator: bool,
    },
    /// Initialize a new chain
    Init {
        #[arg(long)]
        chain_id: String,
        #[arg(long)]
        genesis_validators: Vec<String>,
    },
    /// Account management
    Account {
        #[command(subcommand)]
        action: AccountCommands,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Start { config, validator } => {
            let config = Config::load(&config)?;
            let node = Node::new(config, validator).await?;
            node.run().await?;
        }
        Commands::Init { chain_id, genesis_validators } => {
            init_chain(&chain_id, &genesis_validators)?;
        }
        Commands::Account { action } => {
            handle_account_command(action)?;
        }
    }
    
    Ok(())
}
```

### Docker Deployment

```dockerfile
FROM rust:1.75-slim as builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/blockchain-node /usr/local/bin/
EXPOSE 8545 30303
VOLUME ["/data"]
ENTRYPOINT ["blockchain-node"]
CMD ["start", "--config", "/data/config.toml"]
```

## Additional Resources

- **Advanced consensus**: [references/consensus-pos.md](references/consensus-pos.md), [references/consensus-bft.md](references/consensus-bft.md)
- **Framework-specific guides**: [references/substrate.md](references/substrate.md), [references/solana.md](references/solana.md), [references/near.md](references/near.md)
- **Security patterns**: [references/security-audit.md](references/security-audit.md)
- **Performance tuning**: [references/performance.md](references/performance.md)
- **Production deployment**: [references/production.md](references/production.md)

## Common Dependencies

Essential crates for blockchain development:

```toml
# Core async
tokio = { version = "1.35", features = ["full"] }
async-trait = "0.1"

# Serialization
serde = { version = "1.0", features = ["derive"] }
bincode = "1.3"
borsh = "1.0"  # Solana-style serialization

# Cryptography
ed25519-dalek = "2.1"
secp256k1 = "0.28"
sha3 = "0.10"
blake3 = "1.5"
curve25519-dalek = "4.1"

# Networking
libp2p = "0.53"
quinn = "0.10"  # QUIC protocol

# Storage
rocksdb = "0.21"
sled = "0.34"  # Embedded database alternative

# WASM
wasmer = "4.2"
wasmtime = "16.0"

# Numeric types
primitive-types = "0.12"  # U256, H256, etc.
num-bigint = "0.4"

# Error handling
thiserror = "1.0"
anyhow = "1.0"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Testing
proptest = "1.4"
criterion = "0.5"
```
