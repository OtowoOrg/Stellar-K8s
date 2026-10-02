use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    pub payer: Address,
    pub payee: Address,
    pub token: Address,
    pub total_amount: i128,
    pub start_time: u64,
    pub duration: u64,
    pub total_withdrawn: i128,
}
