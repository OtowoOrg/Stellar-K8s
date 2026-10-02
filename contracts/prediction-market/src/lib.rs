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

#![no_std]

pub mod orderbook;
pub mod matching;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env,
};
use orderbook::{Order, OrderBook, Outcome, Side};
use matching::MatchingEngine;

const OUTCOME_PAYOUT_CENTS: i128 = 100; // 1 USDC = 100 cents

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarketStatus {
    Open,
    Resolved(Outcome),
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Status,
    NextOrderId,
    Order(u64),
    Book(Outcome),
    Shares(Address, Outcome),
    Escrow(Address),
}

#[contract]
pub struct PredictionMarketContract;

#[contractimpl]
impl PredictionMarketContract {
    /// Initializes the prediction market with an admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Status, &MarketStatus::Open);
        env.storage().instance().set(&DataKey::NextOrderId, &1u64);

        // Initialize empty orderbooks for Yes and No outcomes
        env.storage().instance().set(&DataKey::Book(Outcome::Yes), &OrderBook::new(&env, Outcome::Yes));
        env.storage().instance().set(&DataKey::Book(Outcome::No), &OrderBook::new(&env, Outcome::No));
    }

    /// Places a limit order for binary prediction shares.
    /// Price is specified in cents (1 to 99). Payout on event resolution is 100 cents (1 USDC).
    pub fn place_order(
        env: Env,
        trader: Address,
        outcome: Outcome,
        side: Side,
        price: u32,
        amount: i128,
    ) -> u64 {
        trader.require_auth();

        assert!(price >= 1 && price <= 99, "price must be between 1 and 99 cents");
        assert!(amount > 0, "amount must be positive");

        let status: MarketStatus = env.storage().instance().get(&DataKey::Status).unwrap();
        assert_eq!(status, MarketStatus::Open, "market is not open for trading");

        // Escrow verification
        if side == Side::Buy {
            let required_usdc = amount * (price as i128);
            let mut current_escrow: i128 = env.storage().instance().get(&DataKey::Escrow(trader.clone())).unwrap_or(0);
            assert!(current_escrow >= required_usdc, "insufficient USDC escrow balance");
            current_escrow -= required_usdc;
            env.storage().instance().set(&DataKey::Escrow(trader.clone()), &current_escrow);
        } else {
            let mut current_shares: i128 = env.storage().instance().get(&DataKey::Shares(trader.clone(), outcome)).unwrap_or(0);
            assert!(current_shares >= amount, "insufficient outcome shares to sell");
            current_shares -= amount;
            env.storage().instance().set(&DataKey::Shares(trader.clone(), outcome), &current_shares);
        }

        let order_id: u64 = env.storage().instance().get(&DataKey::NextOrderId).unwrap();
        env.storage().instance().set(&DataKey::NextOrderId, &(order_id + 1));

        let incoming = Order {
            id: order_id,
            trader: trader.clone(),
            outcome,
            side,
            price,
            amount,
            filled: 0,
            created_at: env.ledger().timestamp(),
        };

        // Load orderbook
        let mut book: OrderBook = env.storage().instance().get(&DataKey::Book(outcome)).unwrap();

        // Run matching engine
        let (fills, rem_order, updated_makers) = MatchingEngine::match_order(
            &env,
            &mut book,
            incoming,
            |id| env.storage().instance().get(&DataKey::Order(id)),
        );

        // Process trade fills and balance transfers
        for i in 0..fills.len() {
            let fill = fills.get(i).unwrap();
            let trade_cost = fill.amount * (fill.price as i128);

            if side == Side::Buy {
                // Taker is Buyer:
                // Taker receives Outcome shares
                let taker_shares: i128 = env.storage().instance().get(&DataKey::Shares(trader.clone(), outcome)).unwrap_or(0);
                env.storage().instance().set(&DataKey::Shares(trader.clone(), outcome), &(taker_shares + fill.amount));

                // Maker (Seller) receives USDC
                let maker_escrow: i128 = env.storage().instance().get(&DataKey::Escrow(fill.maker.clone())).unwrap_or(0);
                env.storage().instance().set(&DataKey::Escrow(fill.maker.clone()), &(maker_escrow + trade_cost));

                // Price improvement refund to buyer if maker ask price was lower than taker limit price
                if price > fill.price {
                    let refund = fill.amount * ((price - fill.price) as i128);
                    let cur = env.storage().instance().get(&DataKey::Escrow(trader.clone())).unwrap_or(0);
                    env.storage().instance().set(&DataKey::Escrow(trader.clone()), &(cur + refund));
                }
            } else {
                // Taker is Seller:
                // Taker receives USDC
                let taker_escrow: i128 = env.storage().instance().get(&DataKey::Escrow(trader.clone())).unwrap_or(0);
                env.storage().instance().set(&DataKey::Escrow(trader.clone()), &(taker_escrow + trade_cost));

                // Maker (Buyer) receives Outcome shares
                let maker_shares: i128 = env.storage().instance().get(&DataKey::Shares(fill.maker.clone(), outcome)).unwrap_or(0);
                env.storage().instance().set(&DataKey::Shares(fill.maker.clone(), outcome), &(maker_shares + fill.amount));
            }
        }

        // Persist updated maker orders
        for i in 0..updated_makers.len() {
            let maker = updated_makers.get(i).unwrap();
            env.storage().instance().set(&DataKey::Order(maker.id), &maker);
        }

        // If order has remaining volume, insert into book as resting limit order
        if rem_order.remaining() > 0 {
            book.insert_order(&env, &rem_order);
            env.storage().instance().set(&DataKey::Order(rem_order.id), &rem_order);
        } else {
            env.storage().instance().set(&DataKey::Order(rem_order.id), &rem_order);
        }

        // Save updated orderbook
        env.storage().instance().set(&DataKey::Book(outcome), &book);

        env.events().publish(
            (symbol_short!("order"), symbol_short!("placed")),
            (order_id, trader, price, amount),
        );

        order_id
    }

    /// Deposits USDC into trader's internal trading escrow.
    pub fn deposit_escrow(env: Env, trader: Address, amount: i128) {
        trader.require_auth();
        assert!(amount > 0, "amount must be positive");
        let current: i128 = env.storage().instance().get(&DataKey::Escrow(trader.clone())).unwrap_or(0);
        env.storage().instance().set(&DataKey::Escrow(trader), &(current + amount));
    }

    /// Mints complementary binary pair shares (1 Yes + 1 No per 100 cents of USDC).
    pub fn mint_complete_set(env: Env, trader: Address, amount: i128) {
        trader.require_auth();
        assert!(amount > 0, "amount must be positive");
        let cost = amount * OUTCOME_PAYOUT_CENTS;
        let mut escrow: i128 = env.storage().instance().get(&DataKey::Escrow(trader.clone())).unwrap_or(0);
        assert!(escrow >= cost, "insufficient escrow for complete set");
        escrow -= cost;
        env.storage().instance().set(&DataKey::Escrow(trader.clone()), &escrow);

        let yes_shares: i128 = env.storage().instance().get(&DataKey::Shares(trader.clone(), Outcome::Yes)).unwrap_or(0);
        let no_shares: i128 = env.storage().instance().get(&DataKey::Shares(trader.clone(), Outcome::No)).unwrap_or(0);

        env.storage().instance().set(&DataKey::Shares(trader.clone(), Outcome::Yes), &(yes_shares + amount));
        env.storage().instance().set(&DataKey::Shares(trader.clone(), Outcome::No), &(no_shares + amount));
    }

    /// Resolves the prediction market event outcome. Only admin can trigger.
    pub fn resolve_market(env: Env, admin: Address, winning_outcome: Outcome) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        assert_eq!(admin, stored_admin, "unauthorized admin");

        let status: MarketStatus = env.storage().instance().get(&DataKey::Status).unwrap();
        assert_eq!(status, MarketStatus::Open, "market already resolved");

        env.storage().instance().set(&DataKey::Status, &MarketStatus::Resolved(winning_outcome));

        env.events().publish(
            (symbol_short!("market"), symbol_short!("resolved")),
            winning_outcome as u32,
        );
    }

    /// Liquidates losing shares and pays out 1 USDC (100 cents) per winning share.
    pub fn claim_payout(env: Env, trader: Address) -> i128 {
        trader.require_auth();
        let status: MarketStatus = env.storage().instance().get(&DataKey::Status).unwrap();

        let winning_outcome = match status {
            MarketStatus::Resolved(out) => out,
            MarketStatus::Open => panic!("market has not yet resolved"),
        };

        let losing_outcome = match winning_outcome {
            Outcome::Yes => Outcome::No,
            Outcome::No => Outcome::Yes,
        };

        // Liquidate / burn losing shares
        env.storage().instance().set(&DataKey::Shares(trader.clone(), losing_outcome), &0i128);

        // Liquidate winning shares and calculate payout
        let winning_shares: i128 = env.storage().instance().get(&DataKey::Shares(trader.clone(), winning_outcome)).unwrap_or(0);
        assert!(winning_shares > 0, "no winning shares to claim");

        let payout = winning_shares * OUTCOME_PAYOUT_CENTS;
        env.storage().instance().set(&DataKey::Shares(trader.clone(), winning_outcome), &0i128);

        let current_escrow: i128 = env.storage().instance().get(&DataKey::Escrow(trader.clone())).unwrap_or(0);
        env.storage().instance().set(&DataKey::Escrow(trader.clone()), &(current_escrow + payout));

        env.events().publish(
            (symbol_short!("claim"), symbol_short!("payout")),
            (trader, payout),
        );

        payout
    }

    /// Fetches an order by ID.
    pub fn get_order(env: Env, order_id: u64) -> Option<Order> {
        env.storage().instance().get(&DataKey::Order(order_id))
    }

    /// Fetches the orderbook for an outcome.
    pub fn get_orderbook(env: Env, outcome: Outcome) -> OrderBook {
        env.storage().instance().get(&DataKey::Book(outcome)).unwrap()
    }

    /// Fetches trader's share balance for an outcome.
    pub fn get_user_shares(env: Env, trader: Address, outcome: Outcome) -> i128 {
        env.storage().instance().get(&DataKey::Shares(trader, outcome)).unwrap_or(0)
    }

    /// Fetches trader's internal USDC escrow balance in cents.
    pub fn get_user_escrow(env: Env, trader: Address) -> i128 {
        env.storage().instance().get(&DataKey::Escrow(trader)).unwrap_or(0)
    }

    /// Fetches market status.
    pub fn get_market_status(env: Env) -> MarketStatus {
        env.storage().instance().get(&DataKey::Status).unwrap()
    }
}
