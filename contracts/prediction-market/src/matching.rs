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


use soroban_sdk::{contracttype, Address, Env, Vec};
use crate::orderbook::{Order, OrderBook, Side};

/// Maximum order matching iterations per invocation to prevent Soroban WASM CPU limits.
pub const MAX_MATCH_ITERATIONS: u32 = 50;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fill {
    pub maker_order_id: u64,
    pub maker: Address,
    pub taker: Address,
    pub price: u32,
    pub amount: i128,
}

pub struct MatchingEngine;

impl MatchingEngine {
    /// Matches an incoming order against opposing price levels in the orderbook.
    /// Returns the executed fills, the updated incoming order, and modified maker orders.
    pub fn match_order<F>(
        env: &Env,
        book: &mut OrderBook,
        mut incoming: Order,
        mut get_order_fn: F,
    ) -> (Vec<Fill>, Order, Vec<Order>)
    where
        F: FnMut(u64) -> Option<Order>,
    {
        let mut fills = Vec::new(env);
        let mut updated_makers = Vec::new(env);
        let is_buy = incoming.side == Side::Buy;

        let mut iterations: u32 = 0;

        while incoming.remaining() > 0 && iterations < MAX_MATCH_ITERATIONS {
            iterations += 1;

            // Opposing book: if incoming is Buy, opposing is Asks (lowest price first)
            // If incoming is Sell, opposing is Bids (highest price first)
            let opposing_levels = if is_buy { &mut book.asks } else { &mut book.bids };
            if opposing_levels.is_empty() {
                break;
            }

            let mut best_level = opposing_levels.get(0).unwrap();

            // Check price intersection:
            // For Buy: incoming.price must be >= best_ask.price
            // For Sell: incoming.price must be <= best_bid.price
            let crosses = if is_buy {
                incoming.price >= best_level.price
            } else {
                incoming.price <= best_level.price
            };

            if !crosses {
                break;
            }

            let trade_price = best_level.price; // Maker's price priority

            // Match against orders in this price level FIFO
            let mut level_orders_to_remove: u32 = 0;

            for i in 0..best_level.order_ids.len() {
                if incoming.remaining() <= 0 || iterations >= MAX_MATCH_ITERATIONS {
                    break;
                }
                iterations += 1;

                let maker_id = best_level.order_ids.get(i).unwrap();
                let mut maker_order = match get_order_fn(maker_id) {
                    Some(o) => o,
                    None => {
                        level_orders_to_remove += 1;
                        continue;
                    }
                };

                let match_amount = if incoming.remaining() < maker_order.remaining() {
                    incoming.remaining()
                } else {
                    maker_order.remaining()
                };

                if match_amount > 0 {
                    incoming.filled += match_amount;
                    maker_order.filled += match_amount;
                    best_level.total_volume -= match_amount;

                    fills.push_back(Fill {
                        maker_order_id: maker_id,
                        maker: maker_order.trader.clone(),
                        taker: incoming.trader.clone(),
                        price: trade_price,
                        amount: match_amount,
                    });

                    updated_makers.push_back(maker_order.clone());

                    if maker_order.is_filled() {
                        level_orders_to_remove += 1;
                    }
                }
            }

            // Drain fully filled maker orders from this price level
            for _ in 0..level_orders_to_remove {
                if !best_level.order_ids.is_empty() {
                    best_level.order_ids.remove(0);
                }
            }

            if best_level.order_ids.is_empty() || best_level.total_volume <= 0 {
                opposing_levels.remove(0);
            } else {
                opposing_levels.set(0, best_level);
            }
        }

        (fills, incoming, updated_makers)
    }
}
