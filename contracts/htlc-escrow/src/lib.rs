#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Bytes, BytesN, Env};

mod crypto;

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Lock(BytesN<32>),
}

#[derive(Clone)]
#[contracttype]
pub struct Escrow {
    pub sender: Address,
    pub receiver: Address,
    pub token: Address,
    pub amount: i128,
    pub timelock: u64,
    pub hashlock: BytesN<32>,
    pub claimed: bool,
}

#[contract]
pub struct HtlcContract;

#[contractimpl]
impl HtlcContract {
    pub fn deposit(
        env: Env,
        sender: Address,
        receiver: Address,
        token: Address,
        amount: i128,
        hashlock: BytesN<32>,
        timelock: u64,
    ) {
        sender.require_auth();

        if env.storage().persistent().has(&DataKey::Lock(hashlock.clone())) {
            panic!("hashlock already exists");
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&sender, &env.current_contract_address(), &amount);

        let escrow = Escrow {
            sender: sender.clone(),
            receiver: receiver.clone(),
            token,
            amount,
            timelock,
            hashlock: hashlock.clone(),
            claimed: false,
        };

        env.storage().persistent().set(&DataKey::Lock(hashlock), &escrow);
    }

    pub fn claim(env: Env, receiver: Address, hashlock: BytesN<32>, preimage: Bytes) {
        receiver.require_auth();
        
        let mut escrow: Escrow = env.storage().persistent().get(&DataKey::Lock(hashlock.clone())).expect("escrow not found");
        
        if escrow.claimed {
            panic!("already claimed");
        }

        if escrow.receiver != receiver {
            panic!("unauthorized receiver");
        }

        let current_time = env.ledger().timestamp();
        if current_time >= escrow.timelock {
            panic!("timelock expired");
        }

        if !crypto::verify_hash(&env, &preimage, &escrow.hashlock) {
            panic!("invalid preimage");
        }

        escrow.claimed = true;
        env.storage().persistent().set(&DataKey::Lock(hashlock.clone()), &escrow);

        let token_client = token::Client::new(&env, &escrow.token);
        token_client.transfer(&env.current_contract_address(), &receiver, &escrow.amount);
    }

    pub fn refund(env: Env, sender: Address, hashlock: BytesN<32>) {
        sender.require_auth();
        
        let mut escrow: Escrow = env.storage().persistent().get(&DataKey::Lock(hashlock.clone())).expect("escrow not found");

        if escrow.claimed {
            panic!("already claimed");
        }

        if escrow.sender != sender {
            panic!("unauthorized sender");
        }

        let current_time = env.ledger().timestamp();
        if current_time < escrow.timelock {
            panic!("timelock not expired");
        }

        escrow.claimed = true;
        env.storage().persistent().set(&DataKey::Lock(hashlock.clone()), &escrow);

        let token_client = token::Client::new(&env, &escrow.token);
        token_client.transfer(&env.current_contract_address(), &sender, &escrow.amount);
    }
}
