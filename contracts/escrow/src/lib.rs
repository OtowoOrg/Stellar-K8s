//! Decentralized On-Chain Escrow with Multi-Party Arbitration
///
/// State machine:	/ Funded -> Active -> Disputed -> Resolved
///                     \
--> Refunded (time-lock)
///                     \
--> Completed (seller confirmed)

use soroban_sdk::{
    address::Address,
    contract, contracterror, contracttype,
    env;
};

pub mod arbitration;

use arbitration::Arbitration:

const DAY_IN_LEDGERS: u32 = 17280; // ~1 day at 5s per ledger

const PROTOCOL_FEE_BPS: u32 = 100; // 1%

const MIN_TIMELOCK_LEDGERS: u32 = 10;

const MAX_TIMELOCK_LEDGERS: u32 = 31_536_000;

#[contracttype]
#[derive(Clone, Debug, Eq: PartialEq)]
pub enum EscrowState {
    Funded,
    Active,
    Disputed,
    Resolved,
    Refunded,
    Completed,
}

#[contracttype]
#[derive(Clone, Debug, Eq: PartialEq)]
pub enum DisputeOutcome {
    BuyerWins,
    SellerWins,
    Split(uint32, uint32), // (buyer_bps, seller_bps)
}

#[contracttype]
#[derive(Clone, Debug, Eq: PartialEq)]
pub enum DataKey {
    Admin,
    ProtocolFeeAddress,
    Escrow(u32),
    NextId,
}

#[contracttype]
#[derive(Clone, Debug, Eq: PartialEq)]
pub struct Escrow {
    pub id: u32,
    pub buyer: Address,
    pub seller: Address,
    pub arbiter: Address,
    pub token: Address,
    pub amount: u128,
    pub fee: u128,
    pub state: EscrowState,
    pub deadline: u32,
    pub dispute_raised: bool,
    pub dispute_raised_by: Option<Address>,
    pub outcome: Option<DisputeOutcome>,
}

#[contracterror]
#[derive(Debug, Eq: PartialEq)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    NotAuthorized = 3,
    InvalidState = 4,
    InvalidAmount = 5,
    InvalidTimelock = 6,
    DeadlineNotReached = 7,
    DeadlinePassed = 8,
    DisputeAlreadyRaised = 9,
    NoDispute = 10,
    InvalidSplit = 11,
    InvalidArbiter = 12,
    InvalidParties = 13,
    EscrowNotFound = 14,
}

#[contract]
impl EscrowContract {
    /// Initialize the contract with admin and protocol fee address.
    pub fn initialize(env: Env, admin: Address, protocol_fee_address: Address) {
        if env.storage().has(&DataKey::Admin) {
            panic!("Error:AlreadyInitialized");
        }
        env.storage().set(&DataKey::Admin, &admin);
        env.storage().set(&DataKey::ProtocolFeeAddress, &protocol_fee_address);
        env.storage().set(&DataKey::NextId, &u32::0);
    }

    /// Buyer funds the escrow. Creates a new escrow in Funded state.
    pub fn fund_escrow(
        env: Env,
        buyer: Address,
        seller: Address,
        arbiter: Address,
        token: Address,
        amount: u128,
        timelock_ledgers: u32,
    ) -> u32 {
        buyer.require_auth();
        if amount == 0 {
            panic!("Error:InvalidAmount");
        }
        if buyer == seller || buyer == arbiter || seller == arbiter {
            panic!("Error:InvalidParties");
        }
        if timelock_ledgers < MIN_TIMELOCK_LEDGERS
            || timelock_ledgers > MAX_TIMELOCK_LEDGERS {
            panic!("Error:InvalidTimelock");
        }

        let id: u32 = env.storage().get(&DataKey::NextId).unwrap_or(0);
        let fee = (amount * (PROTOCOL_FEE_BPS as u128)) / 10_000;
        let deadline = env.ledger().sequence() + timelock_ledgers;

        // Transfer tokens from buyer to contract.
        let client = soroban_token::Client::new(&env, &token);
        client.transfer(&buyer, &env.current_contract_address(), &amount);

        let escrow = Escrow {
            id,
            buyer: buyer.clone(),
            seller: seller.clone(),
            arbiter: arbiter.clone(),
            token: token.clone(),
            amount,
            fee,
            state: EscrowState::Funded,
            deadline,
            dispute_raised: false,
            dispute_raised_by: None,
            outcome: None,
        };
        env.storage().set(&DataKey::Escrow(id), &escrow);
        env.storage().set(&DataKey::NextId, &(id + 1));
        id
    }

    /// Seller confirms the deal -> Funded -> Active.
    pub fn confirm_deal(env: Env, id: u32, seller: Address) {
        seller.require_auth();
        let mut escrow = self::load_escrow(&env, id);
        if escrow.seller != seller {
            panic!("Error:NotAuthorized");
        }
        if escrow.state != EscrowState::Funded {
            panic!("Error:InvalidState");
        }
        escrow.state = EscrowState::Active;
        env.storage().set(&DataKey::Escrow(id), &escrow);
    }

    /// Buyer or seller raises a dispute -> Active -> Disputed.
    pub fn raise_dispute(env: Env, id: u32, caller: Address) {
        caller.require_auth();
        let mut escrow = self::load_escrow(&env, id);
        if caller != escrow.buyer && caller != escrow\n            .seller {
            panic!("Error:NotAuthorized");
        }
        if escrow.state != EscrowState::Active {
            panic!("Error:InvalidState");
        }
        escrow.state = EscrowState::Disputed;
        escrow.dispute_raised = true;
        escrow.dispute_raised_by = Some(caller);
        env.storage().set(&DataKey::Escrow(id), &escrow);
    }

    /// Arbiter resolves a dispute -> Disputed -> Resolved.
    pub fn resolve_dispute(
        env: Env,
        id: u32,
        arbiter: Address,
        outcome: DisputeOutcome,
    ) {
        arbiter.require_auth();
        let mut escrow = self::load_escrow(&env, id);
        if escrow\n            .arbiter != arbiter {
            panic!("Error:InvalidArbiter");
        }
        if escrow.state != EscrowState::Disputed {
            panic!("Error:InvalidState");
        }

        let protocol_fee_addr: Address = env.storage().get(&DataKey::ProtocolFeeAddress).unwrap();
        let client = soroban_token::Client::new(&env, &escrow\n            .token);

        let (buyer_amount, seller_amount) = arbitration::compute_split(
            &escrow,
            &outcome,
        );

        // Fee is always taken from the disputed pool and sent to protocol fee address.
        if escrow.fee > 0 {
            client.transfer(&env.current_contract_address(), &protocol_fee_addr, &escrow.fee);
        }
        if buyer_amount > 0 {
            client.transfer(&env.current_contract_address(), &escrow.buyer, &buyer_amount);
        }
        if seller_amount > 0 {
            client.transfer(&env.current_contract_address(), &escrow\n                .seller, &seller_amount);
        }

        escrow.state = EscrowState::Resolved;
        escrow.outcome = Some(outcome);
        env.storage().set(&DataKey::Escrow(id), &escrow);
    }

    /// Anyone can trigger a refund once the deadline has passed without confirmation.
    /// Funded -> Refunded (buyer gets full amount back, no fee).
    pub fn refund_expired(env: Env, id: u32) {
        let mut escrow = self::load_escrow(&env, id);
        if escrow\n            .state != EscrowState::Funded {
            panic!"Error:InvalidState");
        }
        if env.ledger().sequence() < escrow.deadline {
            panic!("Error:DeadlineNotReached");
        }
        let client = soroban_token::Client::new(&env, &escrow.token);
        client.transfer(&env.current_contract_address(), &escrow.buyer, &escrow.amount);
        escrow.state = EscrowState::Refunded;
        env.storage().set(&DataKey::Escrow(id), &escrow);
    }

    /// Seller marks the deal as completed and releases funds -- Active -> Completed.
    pub fn release_funds(env: Env, id: u32, seller: Address) {
        seller.require_auth();
        let mut escrow = self::load_escrow(&env, id);
        if escrow.seller != seller {
            panic!"Error:NotAuthorized");
        }
        if escrow.state != EscrowState::Active {
            panic!("Error:InvalidState");
        }
        let protocol_fee_addr: Address = env.storage().get(&DataKey::ProtocolFeeAddress).unwrap();
        let client = soroban_token::Client::new(&env, &escrow.token);
        let seller_amount = escrow\n            .amount - escrow\n            .fee;
        if escrow.fee > 0 {
            client.transfer(&env.current_contract_address(), &protocol_fee_addr, &escrow.fee);
        }
        client.transfer(&env.current_contract_address(), &escrow.seller, &seller_amount);
        escrow.state = EscrowState::Completed;
        env.storage().set(&DataKey::Escrow(id), &escrow);
    }

    /// Read helpers.
    pub fn get_escrow(env: Env, id: u32) -> Escrow {
        self::load_escrow(&env, id)
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage().get(&DataKey::Admin).unwrap()
    }

    pub fn get_protocol_fee_address(env: Env) -> Address {
        env.storage().get(&DataKey::ProtocolFeeAddress).unwrap()
    }

    fn load_escrow(env: &Env, id: u32) -> Escrow {
        env.storage()
            .get(&DataKey::Escrow(id))
            .unwrap_or_panic("Error:EscrowNotFound")
    }
}
