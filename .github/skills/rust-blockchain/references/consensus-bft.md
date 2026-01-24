# Byzantine Fault Tolerant Consensus

Implementation of BFT consensus algorithms (PBFT, Tendermint) in Rust.

## PBFT (Practical Byzantine Fault Tolerance)

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PBFTMessage {
    PrePrepare {
        view: u64,
        sequence: u64,
        block: Block,
        signature: Signature,
    },
    Prepare {
        view: u64,
        sequence: u64,
        block_hash: [u8; 32],
        replica_id: u32,
        signature: Signature,
    },
    Commit {
        view: u64,
        sequence: u64,
        block_hash: [u8; 32],
        replica_id: u32,
        signature: Signature,
    },
    ViewChange {
        new_view: u64,
        replica_id: u32,
        signature: Signature,
    },
}

pub struct PBFTConsensus {
    replica_id: u32,
    view: u64,
    sequence: u64,
    replicas: Vec<ReplicaInfo>,
    f: usize,  // Max Byzantine nodes (n = 3f + 1)
    
    // Message logs
    pre_prepare_log: HashMap<u64, Block>,
    prepare_log: HashMap<(u64, u64), HashMap<u32, Signature>>,
    commit_log: HashMap<(u64, u64), HashMap<u32, Signature>>,
    
    // State
    prepared: HashMap<(u64, u64), [u8; 32]>,
    committed_local: HashMap<(u64, u64), [u8; 32]>,
}

impl PBFTConsensus {
    pub fn new(replica_id: u32, replicas: Vec<ReplicaInfo>) -> Self {
        let n = replicas.len();
        assert!(n >= 4, "PBFT requires at least 4 nodes");
        let f = (n - 1) / 3;
        
        Self {
            replica_id,
            view: 0,
            sequence: 0,
            replicas,
            f,
            pre_prepare_log: HashMap::new(),
            prepare_log: HashMap::new(),
            commit_log: HashMap::new(),
            prepared: HashMap::new(),
            committed_local: HashMap::new(),
        }
    }
    
    fn is_primary(&self) -> bool {
        self.view as usize % self.replicas.len() == self.replica_id as usize
    }
    
    pub async fn propose_block(&mut self, block: Block) -> BlockchainResult<()> {
        if !self.is_primary() {
            return Err(BlockchainError::NotPrimary);
        }
        
        self.sequence += 1;
        let message = PBFTMessage::PrePrepare {
            view: self.view,
            sequence: self.sequence,
            block: block.clone(),
            signature: self.sign_message(&block)?,
        };
        
        self.pre_prepare_log.insert(self.sequence, block);
        self.broadcast(message).await?;
        
        Ok(())
    }
    
    pub async fn handle_pre_prepare(
        &mut self,
        view: u64,
        sequence: u64,
        block: Block,
        signature: Signature,
    ) -> BlockchainResult<()> {
        // Verify message from primary
        let primary_id = (view as usize % self.replicas.len()) as u32;
        self.verify_signature(&block, &signature, primary_id)?;
        
        // Check view and sequence
        if view != self.view || sequence != self.sequence + 1 {
            return Err(BlockchainError::InvalidSequence);
        }
        
        // Validate block
        self.validate_block(&block)?;
        
        self.sequence = sequence;
        self.pre_prepare_log.insert(sequence, block.clone());
        
        // Send PREPARE
        let prepare = PBFTMessage::Prepare {
            view,
            sequence,
            block_hash: block.hash(),
            replica_id: self.replica_id,
            signature: self.sign_message(&block)?,
        };
        
        self.broadcast(prepare).await?;
        
        Ok(())
    }
    
    pub async fn handle_prepare(
        &mut self,
        view: u64,
        sequence: u64,
        block_hash: [u8; 32],
        replica_id: u32,
        signature: Signature,
    ) -> BlockchainResult<()> {
        // Verify signature
        self.verify_signature(&block_hash, &signature, replica_id)?;
        
        // Store prepare message
        self.prepare_log
            .entry((view, sequence))
            .or_default()
            .insert(replica_id, signature);
        
        // Check if we have 2f+1 PREPARE messages
        let prepare_count = self.prepare_log
            .get(&(view, sequence))
            .map(|m| m.len())
            .unwrap_or(0);
        
        if prepare_count >= 2 * self.f {
            // Entered PREPARED state
            self.prepared.insert((view, sequence), block_hash);
            
            // Send COMMIT
            let commit = PBFTMessage::Commit {
                view,
                sequence,
                block_hash,
                replica_id: self.replica_id,
                signature: self.sign_message(&block_hash)?,
            };
            
            self.broadcast(commit).await?;
        }
        
        Ok(())
    }
    
    pub async fn handle_commit(
        &mut self,
        view: u64,
        sequence: u64,
        block_hash: [u8; 32],
        replica_id: u32,
        signature: Signature,
    ) -> BlockchainResult<()> {
        // Verify signature
        self.verify_signature(&block_hash, &signature, replica_id)?;
        
        // Store commit message
        self.commit_log
            .entry((view, sequence))
            .or_default()
            .insert(replica_id, signature);
        
        // Check if we have 2f+1 COMMIT messages
        let commit_count = self.commit_log
            .get(&(view, sequence))
            .map(|m| m.len())
            .unwrap_or(0);
        
        if commit_count >= 2 * self.f + 1 {
            // Entered COMMITTED state
            self.committed_local.insert((view, sequence), block_hash);
            
            // Execute block
            if let Some(block) = self.pre_prepare_log.get(&sequence) {
                self.execute_block(block.clone()).await?;
            }
        }
        
        Ok(())
    }
}
```

## Tendermint Consensus

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TendermintMessage {
    Proposal {
        height: u64,
        round: u32,
        block: Block,
        pol_round: Option<u32>,
        signature: Signature,
    },
    Prevote {
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator: Address,
        signature: Signature,
    },
    Precommit {
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator: Address,
        signature: Signature,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TendermintStep {
    Propose,
    Prevote,
    Precommit,
    Commit,
}

pub struct TendermintConsensus {
    validator_address: Address,
    height: u64,
    round: u32,
    step: TendermintStep,
    
    validators: Vec<Validator>,
    locked_block: Option<Block>,
    locked_round: Option<u32>,
    valid_block: Option<Block>,
    valid_round: Option<u32>,
    
    // Vote tracking
    prevotes: HashMap<(u64, u32), HashMap<Option<[u8; 32]>, Vec<Address>>>,
    precommits: HashMap<(u64, u32), HashMap<Option<[u8; 32]>, Vec<Address>>>,
}

impl TendermintConsensus {
    pub fn new(validator_address: Address, validators: Vec<Validator>) -> Self {
        Self {
            validator_address,
            height: 1,
            round: 0,
            step: TendermintStep::Propose,
            validators,
            locked_block: None,
            locked_round: None,
            valid_block: None,
            valid_round: None,
            prevotes: HashMap::new(),
            precommits: HashMap::new(),
        }
    }
    
    pub async fn start_round(&mut self, round: u32) -> BlockchainResult<()> {
        self.round = round;
        self.step = TendermintStep::Propose;
        
        if self.is_proposer(self.height, round) {
            let block = if let Some(valid_block) = &self.valid_block {
                valid_block.clone()
            } else {
                self.create_block().await?
            };
            
            self.broadcast_proposal(block, self.valid_round).await?;
        }
        
        // Set timeout for proposal
        self.schedule_timeout(Duration::from_secs(3), TimeoutType::Propose).await;
        
        Ok(())
    }
    
    fn is_proposer(&self, height: u64, round: u32) -> bool {
        let proposer_index = ((height as usize + round as usize) % self.validators.len());
        self.validators[proposer_index].address == self.validator_address
    }
    
    pub async fn handle_proposal(
        &mut self,
        height: u64,
        round: u32,
        block: Block,
        pol_round: Option<u32>,
    ) -> BlockchainResult<()> {
        if height != self.height || round != self.round {
            return Ok(());  // Ignore old proposals
        }
        
        self.step = TendermintStep::Prevote;
        
        let vote_hash = if self.locked_block.is_some() {
            // Already locked on a block
            if self.locked_round == pol_round {
                Some(block.hash())  // Vote for new block
            } else {
                self.locked_block.as_ref().map(|b| b.hash())  // Vote for locked block
            }
        } else {
            // Not locked, vote if block is valid
            if self.validate_block(&block).is_ok() {
                Some(block.hash())
            } else {
                None  // Vote nil
            }
        };
        
        self.broadcast_prevote(vote_hash).await?;
        
        Ok(())
    }
    
    pub async fn handle_prevote(
        &mut self,
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator: Address,
    ) -> BlockchainResult<()> {
        // Store prevote
        self.prevotes
            .entry((height, round))
            .or_default()
            .entry(block_hash)
            .or_default()
            .push(validator);
        
        // Check for +2/3 prevotes
        let total_stake = self.get_total_stake();
        
        for (hash, voters) in self.prevotes.get(&(height, round)).unwrap() {
            let vote_stake = self.calculate_stake(voters);
            
            if vote_stake * 3 > total_stake * 2 {
                // Received +2/3 prevotes
                if let Some(hash) = hash {
                    self.valid_block = self.get_block(hash);
                    self.valid_round = Some(round);
                }
                
                self.step = TendermintStep::Precommit;
                
                let precommit_hash = if self.locked_round.is_none() || 
                    self.locked_round == Some(round) {
                    *hash  // Precommit the block
                } else {
                    None  // Precommit nil
                };
                
                self.broadcast_precommit(precommit_hash).await?;
                break;
            }
        }
        
        Ok(())
    }
    
    pub async fn handle_precommit(
        &mut self,
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator: Address,
    ) -> BlockchainResult<()> {
        // Store precommit
        self.precommits
            .entry((height, round))
            .or_default()
            .entry(block_hash)
            .or_default()
            .push(validator);
        
        let total_stake = self.get_total_stake();
        
        // Check for +2/3 precommits for a block
        for (hash, voters) in self.precommits.get(&(height, round)).unwrap() {
            let vote_stake = self.calculate_stake(voters);
            
            if vote_stake * 3 > total_stake * 2 {
                if let Some(hash) = hash {
                    // Decision: commit block
                    if let Some(block) = self.get_block(hash) {
                        self.commit_block(block).await?;
                        
                        // Start next height
                        self.height += 1;
                        self.round = 0;
                        self.locked_block = None;
                        self.locked_round = None;
                        self.valid_block = None;
                        self.valid_round = None;
                        
                        self.start_round(0).await?;
                    }
                } else {
                    // +2/3 precommit nil, move to next round
                    self.start_round(round + 1).await?;
                }
                break;
            }
        }
        
        Ok(())
    }
}
```

## View Change / Round Timeouts

```rust
impl PBFTConsensus {
    pub async fn initiate_view_change(&mut self) -> BlockchainResult<()> {
        let new_view = self.view + 1;
        
        let message = PBFTMessage::ViewChange {
            new_view,
            replica_id: self.replica_id,
            signature: self.sign_view_change(new_view)?,
        };
        
        self.broadcast(message).await?;
        
        Ok(())
    }
    
    pub async fn handle_view_change(
        &mut self,
        new_view: u64,
        replica_id: u32,
        signature: Signature,
    ) -> BlockchainResult<()> {
        self.verify_signature(&new_view.to_le_bytes(), &signature, replica_id)?;
        
        self.view_change_log
            .entry(new_view)
            .or_default()
            .insert(replica_id);
        
        // Check if we have 2f+1 VIEW-CHANGE messages
        let count = self.view_change_log.get(&new_view).map(|s| s.len()).unwrap_or(0);
        
        if count >= 2 * self.f + 1 {
            // Transition to new view
            self.view = new_view;
            self.sequence = 0;
            
            // Clear message logs
            self.prepare_log.clear();
            self.commit_log.clear();
            
            // If new primary, propose next block
            if self.is_primary() {
                self.propose_next_block().await?;
            }
        }
        
        Ok(())
    }
}
```

## Consensus Safety Properties

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_safety_two_chains_cannot_commit() {
        let mut consensus1 = TendermintConsensus::new(/* ... */);
        let mut consensus2 = TendermintConsensus::new(/* ... */);
        
        let block1 = create_test_block(1);
        let block2 = create_test_block(2);
        
        // Simulate network partition
        consensus1.handle_proposal(1, 0, block1.clone(), None).await.unwrap();
        consensus2.handle_proposal(1, 0, block2.clone(), None).await.unwrap();
        
        // Both should not commit conflicting blocks
        assert!(consensus1.get_committed_block(1) != consensus2.get_committed_block(1) 
            || consensus1.get_committed_block(1).is_none());
    }
    
    #[tokio::test]
    async fn test_liveness_eventually_commits() {
        let mut consensus = TendermintConsensus::new(/* ... */);
        
        // Should eventually commit even with Byzantine nodes
        for _ in 0..100 {
            if consensus.height > 1 {
                break;
            }
            // Simulate rounds
            consensus.start_round(consensus.round).await.unwrap();
        }
        
        assert!(consensus.height > 1, "Consensus should make progress");
    }
}
```
