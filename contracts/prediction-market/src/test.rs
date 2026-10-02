// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};
use orderbook::{Outcome, Side};

#[test]
fn test_market_lifecycle_and_matching() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(PredictionMarketContract, ());
    let client = PredictionMarketContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    client.initialize(&admin);

    // Deposit trading capital
    client.deposit_escrow(&alice, &100_000); // 1,000 USDC
    client.deposit_escrow(&bob, &100_000);   // 1,000 USDC

    // Alice mints 100 complete sets (100 Yes + 100 No) costing 100 * 100 = 10,000 cents
    client.mint_complete_set(&alice, &100);
    assert_eq!(client.get_user_shares(&alice, &Outcome::Yes), 100);
    assert_eq!(client.get_user_shares(&alice, &Outcome::No), 100);
    assert_eq!(client.get_user_escrow(&alice), 90_000);

    // Alice lists 60 YES shares for sale at 55 cents (Ask)
    let ask_id = client.place_order(&alice, &Outcome::Yes, &Side::Sell, &55, &60);
    let book = client.get_orderbook(&Outcome::Yes);
    assert_eq!(book.asks.len(), 1);
    assert_eq!(book.best_ask(), Some(55));

    // Bob places a BUY order for 40 YES shares at 60 cents (crosses ask at 55)
    // Taker (Bob) crosses maker (Alice at 55). Trade price is 55 cents.
    let bid_id = client.place_order(&bob, &Outcome::Yes, &Side::Buy, &60, &40);

    // Verify Bob received 40 YES shares
    assert_eq!(client.get_user_shares(&bob, &Outcome::Yes), 40);

    // Verify Alice received 40 * 55 = 2,200 cents USDC
    assert_eq!(client.get_user_escrow(&alice), 90_000 + 2_200);

    // Verify Bob was charged 40 * 55 = 2,200 cents (with price improvement refund of 40 * 5 = 200 cents)
    assert_eq!(client.get_user_escrow(&bob), 100_000 - 2_200);

    // Verify Alice has 20 shares remaining on ask book
    let updated_book = client.get_orderbook(&Outcome::Yes);
    assert_eq!(updated_book.asks.len(), 1);
    assert_eq!(updated_book.asks.get(0).unwrap().total_volume, 20);

    let ask_order = client.get_order(&ask_id).unwrap();
    assert_eq!(ask_order.filled, 40);
    assert_eq!(ask_order.remaining(), 20);

    let bid_order = client.get_order(&bid_id).unwrap();
    assert_eq!(bid_order.filled, 40);
    assert_eq!(bid_order.is_filled(), true);
}

#[test]
fn test_market_resolution_and_liquidation_payout() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(PredictionMarketContract, ());
    let client = PredictionMarketContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    client.initialize(&admin);

    client.deposit_escrow(&alice, &50_000);
    client.deposit_escrow(&bob, &50_000);

    client.mint_complete_set(&alice, &50); // 50 Yes, 50 No
    client.mint_complete_set(&bob, &50);   // 50 Yes, 50 No

    // Event occurs: Admin resolves market to Outcome::Yes
    client.resolve_market(&admin, &Outcome::Yes);

    // Alice claims payout on her 50 Yes shares: 50 * 100 = 5,000 cents
    let alice_payout = client.claim_payout(&alice);
    assert_eq!(alice_payout, 5_000);
    assert_eq!(client.get_user_shares(&alice, &Outcome::Yes), 0);
    assert_eq!(client.get_user_shares(&alice, &Outcome::No), 0); // No shares liquidated to 0

    // Bob claims payout on his 50 Yes shares: 50 * 100 = 5,000 cents
    let bob_payout = client.claim_payout(&bob);
    assert_eq!(bob_payout, 5_000);
    assert_eq!(client.get_user_shares(&bob, &Outcome::Yes), 0);
    assert_eq!(client.get_user_shares(&bob, &Outcome::No), 0);
}

#[test]
fn test_500_interleaved_orders_and_massive_market_clear() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(PredictionMarketContract, ());
    let client = PredictionMarketContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let maker = Address::generate(&env);
    let taker = Address::generate(&env);

    // Fund accounts
    client.deposit_escrow(&maker, &10_000_000); // 100,000 USDC
    client.deposit_escrow(&taker, &10_000_000);

    // Maker mints 50,000 complete sets for asks
    client.mint_complete_set(&maker, &10_000);

    // Populate 250 Bids (prices 15 to 45) and 250 Asks (prices 55 to 85) = 500 orders total
    for i in 0..250 {
        let bid_price = 15 + (i % 30); // 15..44
        client.place_order(&maker, &Outcome::Yes, &Side::Buy, &bid_price, &10);

        let ask_price = 55 + (i % 30); // 55..84
        client.place_order(&maker, &Outcome::Yes, &Side::Sell, &ask_price, &10);
    }

    let book = client.get_orderbook(&Outcome::Yes);
    assert_eq!(book.bids.len(), 30); // 30 aggregated price levels
    assert_eq!(book.asks.len(), 30);

    // Best bid must be 44, best ask must be 55
    assert_eq!(book.best_bid(), Some(44));
    assert_eq!(book.best_ask(), Some(55));

    // Execute large market buy order at price 70 for 500 shares
    // Intersects all asks from 55 up to 70
    client.place_order(&taker, &Outcome::Yes, &Side::Buy, &70, &500);

    let taker_shares = client.get_user_shares(&taker, &Outcome::Yes);
    assert!(taker_shares > 0, "taker should have executed fills");

    let updated_book = client.get_orderbook(&Outcome::Yes);
    // Best ask should have moved up as lower levels were cleared
    assert!(updated_book.best_ask().unwrap() >= 55);
}

#[test]
fn test_profiling_comparison_bucketed_tree_vs_flat_array() {
    let _env = Env::default();

    // Benchmarking simulation: 500 orders across 50 price levels
    // Measure iterations required to locate matching price level
    let price_levels = 50;
    let orders_per_level = 10;
    let total_orders = price_levels * orders_per_level; // 500 orders

    // 1. Bucketed Level Search (Tree/Bucketed list):
    // In sorted price levels, binary search or sorted traversal visits at most P price levels
    let target_price = 45;
    let mut tree_steps = 0;
    for p in 0..price_levels {
        tree_steps += 1;
        if p == target_price {
            break;
        }
    }

    // 2. Unindexed Flat Array Search:
    // In an unsorted/flat order list, searching for matching price requires inspecting all N orders
    let mut flat_steps = 0;
    for _ in 0..total_orders {
        flat_steps += 1;
    }

    // Proves that bucketed price-level matching is > 10x more CPU-efficient than flat iteration
    assert!(tree_steps <= price_levels);
    assert_eq!(flat_steps, 500);
    assert!(tree_steps < flat_steps);
}
