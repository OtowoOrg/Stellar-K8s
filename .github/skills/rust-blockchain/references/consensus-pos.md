# Proof of Stake (PoS) Consensus

Advanced implementation of Proof of Stake consensus mechanism in Rust.

## Basic PoS Implementation

```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Validator {
    pub address: Address,
    pub stake: u128,
    pub commission: u8,  // 0-100
    pub uptime: f64,
}

pub struct PoSConsensus {
    validators: HashMap<Address, Validator>,
    delegations: HashMap<Address, HashMap<Address, u128>>,  // delegator -> validator -> amount
    total_stake: u128,
    min_stake: u128,
    epoch_length: u64,
    current_epoch: u64,
}

impl PoSConsensus {
    pub fn select_proposer(&self, block_height: u64, seed: [u8; 32]) -> Address {
        // Weighted random selection based on stake
        let target = self.pseudo_random(seed, block_height) % self.total_stake;
        
        let mut cumulative_stake = 0u128;
        for (address, validator) in &self.validators {
            let validator_total_stake = self.get_total_stake(address);
            cumulative_stake += validator_total_stake;
            
            if cumulative_stake > target {
                return *address;
            }
        }
        
        unreachable!("Proposer selection failed");
    }
    
    fn get_total_stake(&self, validator: &Address) -> u128 {
        let self_stake = self.validators.get(validator)
            .map(|v| v.stake)
            .unwrap_or(0);
        
        let delegated_stake: u128 = self.delegations
            .values()
            .filter_map(|delegations| delegations.get(validator))
            .sum();
        
        self_stake + delegated_stake
    }
    
    fn pseudo_random(&self, seed: [u8; 32], nonce: u64) -> u128 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&seed);
        hasher.update(&nonce.to_le_bytes());
        let hash = hasher.finalize();
        u128::from_le_bytes(hash.as_bytes()[0..16].try_into().unwrap())
    }
}
```

## Slashing Conditions

```rust
#[derive(Debug, Clone)]
pub enum SlashingOffense {
    DoubleSign { block_height: u64 },
    Downtime { missed_blocks: u64 },
    InvalidBlock { reason: String },
}

impl PoSConsensus {
    pub async fn slash_validator(
        &mut self,
        validator: &Address,
        offense: SlashingOffense,
    ) -> BlockchainResult<()> {
        let slash_percentage = match offense {
            SlashingOffense::DoubleSign { .. } => 50,  // 50% slash
            SlashingOffense::Downtime { missed_blocks } if missed_blocks > 1000 => 5,
            SlashingOffense::InvalidBlock { .. } => 10,
            _ => return Ok(()),
        };
        
        let validator_info = self.validators.get_mut(validator)
            .ok_or(BlockchainError::ValidatorNotFound)?;
        
        let slash_amount = validator_info.stake * slash_percentage / 100;
        validator_info.stake -= slash_amount;
        
        // Emit slashing event
        self.emit_event(Event::ValidatorSlashed {
            validator: *validator,
            offense,
            amount: slash_amount,
        });
        
        // Remove validator if stake below minimum
        if validator_info.stake < self.min_stake {
            self.remove_validator(validator).await?;
        }
        
        Ok(())
    }
}
```

## Delegation System

```rust
impl PoSConsensus {
    pub async fn delegate(
        &mut self,
        delegator: Address,
        validator: Address,
        amount: u128,
    ) -> BlockchainResult<()> {
        // Verify validator exists and is active
        if !self.validators.contains_key(&validator) {
            return Err(BlockchainError::ValidatorNotFound);
        }
        
        // Update delegation
        self.delegations
            .entry(delegator)
            .or_default()
            .insert(validator, amount);
        
        self.total_stake += amount;
        
        Ok(())
    }
    
    pub async fn undelegate(
        &mut self,
        delegator: Address,
        validator: Address,
    ) -> BlockchainResult<u128> {
        let amount = self.delegations
            .get_mut(&delegator)
            .and_then(|delegations| delegations.remove(&validator))
            .ok_or(BlockchainError::NoDelegationFound)?;
        
        self.total_stake -= amount;
        
        // Queue for unbonding period (e.g., 21 days)
        self.queue_unbonding(delegator, amount, self.current_epoch + 21);
        
        Ok(amount)
    }
    
    fn queue_unbonding(&mut self, address: Address, amount: u128, unlock_epoch: u64) {
        self.unbonding_queue
            .entry(unlock_epoch)
            .or_default()
            .push((address, amount));
    }
}
```

## Reward Distribution

```rust
impl PoSConsensus {
    pub async fn distribute_rewards(&mut self, block_rewards: u128) -> BlockchainResult<()> {
        for (validator_address, validator) in &self.validators {
            let total_stake = self.get_total_stake(validator_address);
            let validator_share = block_rewards * total_stake / self.total_stake;
            
            // Commission for validator
            let commission_amount = validator_share * validator.commission as u128 / 100;
            let delegator_share = validator_share - commission_amount;
            
            // Reward validator
            self.reward_account(validator_address, commission_amount + 
                (delegator_share * validator.stake / total_stake)).await?;
            
            // Reward delegators proportionally
            if let Some(delegations) = self.delegations.values().find(|d| d.contains_key(validator_address)) {
                for (delegator, stake) in delegations {
                    if delegator != validator_address {
                        let reward = delegator_share * stake / total_stake;
                        self.reward_account(delegator, reward).await?;
                    }
                }
            }
        }
        
        Ok(())
    }
}
```

## Epoch Management

```rust
impl PoSConsensus {
    pub async fn transition_epoch(&mut self) -> BlockchainResult<()> {
        self.current_epoch += 1;
        
        // Process unbonding queue
        if let Some(unbonding) = self.unbonding_queue.remove(&self.current_epoch) {
            for (address, amount) in unbonding {
                self.release_unbonded(address, amount).await?;
            }
        }
        
        // Update validator set
        self.update_validator_set().await?;
        
        // Calculate and distribute rewards
        let epoch_rewards = self.calculate_epoch_rewards();
        self.distribute_rewards(epoch_rewards).await?;
        
        Ok(())
    }
    
    async fn update_validator_set(&mut self) -> BlockchainResult<()> {
        // Remove validators with insufficient stake
        self.validators.retain(|_, v| v.stake >= self.min_stake);
        
        // Recalculate total stake
        self.total_stake = self.validators.values()
            .map(|v| self.get_total_stake(&v.address))
            .sum();
        
        Ok(())
    }
}
```

## BLS Signature Aggregation

For efficient multi-signature verification:

```rust
use blst::min_pk::{AggregateSignature, PublicKey, SecretKey, Signature};

pub struct BLSValidator {
    pub address: Address,
    pub public_key: PublicKey,
    pub stake: u128,
}

impl PoSConsensus {
    pub fn aggregate_signatures(
        &self,
        signatures: Vec<(Address, Signature)>,
    ) -> BlockchainResult<AggregateSignature> {
        let mut agg_sig = AggregateSignature::new();
        
        for (validator, signature) in signatures {
            // Verify validator is authorized
            if !self.validators.contains_key(&validator) {
                return Err(BlockchainError::UnauthorizedValidator);
            }
            
            agg_sig.add_signature(&signature, true)?;
        }
        
        Ok(agg_sig)
    }
    
    pub fn verify_aggregate(
        &self,
        message: &[u8],
        aggregate: &AggregateSignature,
        signers: &[Address],
    ) -> BlockchainResult<()> {
        // Check 2/3+ stake requirement
        let total_stake: u128 = signers
            .iter()
            .filter_map(|addr| self.validators.get(addr))
            .map(|v| self.get_total_stake(&v.address))
            .sum();
        
        if total_stake < self.total_stake * 2 / 3 {
            return Err(BlockchainError::InsufficientStake);
        }
        
        // Aggregate public keys
        let pub_keys: Vec<&PublicKey> = signers
            .iter()
            .filter_map(|addr| self.validators.get(addr))
            .map(|v| &v.public_key)
            .collect();
        
        // Verify aggregated signature
        if !aggregate.verify(message, &pub_keys) {
            return Err(BlockchainError::InvalidAggregateSignature);
        }
        
        Ok(())
    }
}
```
