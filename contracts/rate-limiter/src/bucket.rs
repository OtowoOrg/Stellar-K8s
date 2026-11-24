#![no_std]

use soroban_sdk::{contracttype};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenBucket {
    pub tokens: u32,
    pub last_refill: u32,
}

impl TokenBucket {
    pub fn new(capacity: u32, current_time: u32) -> Self {
        Self {
            tokens: capacity,
            last_refill: current_time,
        }
    }

    pub fn consume(&mut self, amount: u32, capacity: u32, refill_rate: u32, current_time: u32) -> bool {
        let time_passed = current_time.saturating_sub(self.last_refill);
        let added_tokens = time_passed.saturating_mul(refill_rate);
        
        self.tokens = core::cmp::min(capacity, self.tokens.saturating_add(added_tokens));
        self.last_refill = current_time;

        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }
}
