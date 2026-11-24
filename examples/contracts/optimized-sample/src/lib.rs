//! Optimized Soroban sample — compares storage and loop patterns for gas tuning.
//!
//! See docs/performance/wasm-tuning.md for benchmark tables.

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Env, Symbol};

/// Compact storage keys (preferred over string literals).
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Meta,
    Counter,
    /// Unoptimized monolithic state key for benchmark comparison.
    Monolithic,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Meta {
    pub version: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonolithicState {
    pub version: u32,
    pub counter: u64,
    pub label: Symbol,
}

/// Host-independent helpers exercised by unit tests (no Soroban Env required).
pub fn sum_iterative_impl(n: u32) -> u64 {
    let mut acc: u64 = 0;
    let mut i: u32 = 0;
    while i < n {
        acc = acc.saturating_add(i as u64);
        i += 1;
    }
    acc
}

#[contract]
pub struct OptimizedSample;

#[contractimpl]
impl OptimizedSample {
    /// Optimized path: read/write only the counter shard.
    pub fn increment_optimized(env: Env) -> u64 {
        let mut count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::Counter)
            .unwrap_or(0);
        count = count.saturating_add(1);
        env.storage().persistent().set(&DataKey::Counter, &count);
        count
    }

    /// Unoptimized path: loads entire struct even when only counter changes.
    pub fn increment_unoptimized(env: Env) -> u64 {
        let mut state: MonolithicState = env
            .storage()
            .persistent()
            .get(&DataKey::Monolithic)
            .unwrap_or(MonolithicState {
                version: 1,
                counter: 0,
                label: symbol_short!("bench"),
            });
        state.counter = state.counter.saturating_add(1);
        env.storage().persistent().set(&DataKey::Monolithic, &state);
        state.counter
    }

    /// Iterative sum (preferred — bounded stack).
    pub fn sum_iterative(_env: Env, n: u32) -> u64 {
        sum_iterative_impl(n)
    }

    /// Initialize meta separately from hot counter path.
    pub fn init(env: Env) {
        if !env.storage().persistent().has(&DataKey::Meta) {
            env.storage()
                .persistent()
                .set(&DataKey::Meta, &Meta { version: 1 });
        }
    }

    pub fn read_meta(env: Env) -> Meta {
        env.storage()
            .persistent()
            .get(&DataKey::Meta)
            .unwrap_or(Meta { version: 0 })
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterative_sum_matches_formula() {
        assert_eq!(sum_iterative_impl(100), 4950);
        assert_eq!(sum_iterative_impl(0), 0);
    }

    #[test]
    fn iterative_sum_saturating_large_n() {
        assert_eq!(sum_iterative_impl(1_000), 499_500);
    }
}
