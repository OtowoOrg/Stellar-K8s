#![no_std]

mod stream;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};
use stream::Stream;

#[contracttype]
pub enum DataKey {
    StreamCount,
    Stream(u64), // stream_id -> Stream
}

#[contract]
pub struct StreamingPayContract;

#[contractimpl]
impl StreamingPayContract {
    pub fn create_stream(
        env: Env,
        payer: Address,
        payee: Address,
        token: Address,
        amount: i128,
        duration: u64,
    ) -> u64 {
        payer.require_auth();

        if amount <= 0 {
            panic!("amount must be positive");
        }
        if duration == 0 {
            panic!("duration must be greater than 0");
        }

        // Transfer funds from payer to contract
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&payer, &env.current_contract_address(), &amount);

        let count: u64 = env.storage().instance().get(&DataKey::StreamCount).unwrap_or(0);
        let stream_id = count + 1;

        let current_time = env.ledger().timestamp();

        let stream = Stream {
            payer,
            payee,
            token,
            total_amount: amount,
            start_time: current_time,
            duration,
            total_withdrawn: 0,
        };

        env.storage().persistent().set(&DataKey::Stream(stream_id), &stream);
        env.storage().instance().set(&DataKey::StreamCount, &stream_id);

        stream_id
    }

    pub fn withdraw(env: Env, stream_id: u64) {
        let mut stream: Stream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .expect("stream not found");

        stream.payee.require_auth();

        let current_time = env.ledger().timestamp();
        let elapsed = current_time.saturating_sub(stream.start_time);
        
        let mut unlocked_amount = if elapsed >= stream.duration {
            stream.total_amount
        } else {
            // Integer division rounds down, favoring the payer
            (stream.total_amount as u128 * elapsed as u128 / stream.duration as u128) as i128
        };

        let available = unlocked_amount - stream.total_withdrawn;
        if available > 0 {
            let token_client = token::Client::new(&env, &stream.token);
            token_client.transfer(&env.current_contract_address(), &stream.payee, &available);
            
            stream.total_withdrawn += available;
            env.storage().persistent().set(&DataKey::Stream(stream_id), &stream);
        }
    }

    pub fn cancel_stream(env: Env, stream_id: u64) {
        let mut stream: Stream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .expect("stream not found");

        // Either payer or payee can cancel? Typically payer cancels. Let's allow payer to cancel.
        stream.payer.require_auth();

        let current_time = env.ledger().timestamp();
        let elapsed = current_time.saturating_sub(stream.start_time);
        
        let unlocked_amount = if elapsed >= stream.duration {
            stream.total_amount
        } else {
            (stream.total_amount as u128 * elapsed as u128 / stream.duration as u128) as i128
        };

        let payee_owed = unlocked_amount - stream.total_withdrawn;
        let payer_refund = stream.total_amount - unlocked_amount;

        let token_client = token::Client::new(&env, &stream.token);

        if payee_owed > 0 {
            token_client.transfer(&env.current_contract_address(), &stream.payee, &payee_owed);
        }
        
        if payer_refund > 0 {
            token_client.transfer(&env.current_contract_address(), &stream.payer, &payer_refund);
        }

        // Remove the stream from storage
        env.storage().persistent().remove(&DataKey::Stream(stream_id));
    }
}
