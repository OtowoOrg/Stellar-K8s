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


use soroban_sdk::{contracttype, Address, Vec, Env};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Outcome {
    Yes = 0,
    No = 1,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Order {
    pub id: u64,
    pub trader: Address,
    pub outcome: Outcome,
    pub side: Side,
    pub price: u32,       // Expressed in cents (1 to 99)
    pub amount: i128,     // Total share count
    pub filled: i128,     // Filled share count
    pub created_at: u64,
}

impl Order {
    pub fn remaining(&self) -> i128 {
        self.amount - self.filled
    }

    pub fn is_filled(&self) -> bool {
        self.filled >= self.amount
    }
}

/// PriceLevel represents an aggregated queue of order IDs at a single price point.
/// Implements a FIFO queue per price level to achieve optimal WASM execution.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceLevel {
    pub price: u32,
    pub order_ids: Vec<u64>,
    pub total_volume: i128,
}

/// Bucketed Limit Orderbook holding sorted price levels.
/// Bids: Sorted Descending (highest buy price first)
/// Asks: Sorted Ascending (lowest sell price first)
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderBook {
    pub outcome: Outcome,
    pub bids: Vec<PriceLevel>, // Sorted descending
    pub asks: Vec<PriceLevel>, // Sorted ascending
}

impl OrderBook {
    pub fn new(env: &Env, outcome: Outcome) -> Self {
        Self {
            outcome,
            bids: Vec::new(env),
            asks: Vec::new(env),
        }
    }

    /// Inserts an order ID into the appropriate price level while preserving sorted invariant.
    pub fn insert_order(&mut self, env: &Env, order: &Order) {
        let is_buy = order.side == Side::Buy;
        let levels = if is_buy { &mut self.bids } else { &mut self.asks };
        let mut found_idx: Option<u32> = None;
        let mut insert_pos: Option<u32> = None;

        for i in 0..levels.len() {
            let lvl = levels.get(i).unwrap();
            if lvl.price == order.price {
                found_idx = Some(i);
                break;
            }
            if is_buy {
                // Bids: descending order
                if order.price > lvl.price && insert_pos.is_none() {
                    insert_pos = Some(i);
                }
            } else {
                // Asks: ascending order
                if order.price < lvl.price && insert_pos.is_none() {
                    insert_pos = Some(i);
                }
            }
        }

        if let Some(idx) = found_idx {
            let mut lvl = levels.get(idx).unwrap();
            lvl.order_ids.push_back(order.id);
            lvl.total_volume += order.remaining();
            levels.set(idx, lvl);
        } else {
            let mut ids = Vec::new(env);
            ids.push_back(order.id);
            let new_lvl = PriceLevel {
                price: order.price,
                order_ids: ids,
                total_volume: order.remaining(),
            };
            if let Some(pos) = insert_pos {
                levels.insert(pos, new_lvl);
            } else {
                levels.push_back(new_lvl);
            }
        }
    }

    /// Best bid price (highest buy offer)
    pub fn best_bid(&self) -> Option<u32> {
        if self.bids.is_empty() {
            None
        } else {
            Some(self.bids.get(0).unwrap().price)
        }
    }

    /// Best ask price (lowest sell offer)
    pub fn best_ask(&self) -> Option<u32> {
        if self.asks.is_empty() {
            None
        } else {
            Some(self.asks.get(0).unwrap().price)
        }
    }
}
