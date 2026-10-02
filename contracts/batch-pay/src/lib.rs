#![no_std]

mod segmentation;

use segmentation::{Config, DataKey, ValidationError};
use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contracterror, contractimpl, symbol_short, token, vec, Address, BytesN, Env,
    IntoVal, Vec,
};

#[contract]
pub struct BatchPayContract;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BatchPayError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    EmptyBatch = 3,
    BatchTooLarge = 4,
    LengthMismatch = 5,
    InvalidAmount = 6,
    DuplicateBatch = 7,
    ReentrantCall = 8,
}

#[contractimpl]
impl BatchPayContract {
    pub fn initialize(env: Env, admin: Address, token: Address) -> Result<(), BatchPayError> {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Config) {
            return Err(BatchPayError::AlreadyInitialized);
        }

        env.storage()
            .instance()
            .set(&DataKey::Config, &Config { admin, token });
        env.storage().instance().set(&DataKey::Processing, &false);
        Ok(())
    }

    pub fn process_batch(
        env: Env,
        batch_id: BytesN<32>,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<(), BatchPayError> {
        let config: Config = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(BatchPayError::NotInitialized)?;

        if segmentation::is_processing(&env) {
            return Err(BatchPayError::ReentrantCall);
        }

        config.admin.require_auth();
        match segmentation::validate_batch(&recipients, &amounts) {
            Ok(()) => {}
            Err(ValidationError::Empty) => return Err(BatchPayError::EmptyBatch),
            Err(ValidationError::TooLarge) => return Err(BatchPayError::BatchTooLarge),
            Err(ValidationError::LengthMismatch) => {
                return Err(BatchPayError::LengthMismatch)
            }
            Err(ValidationError::InvalidAmount) => return Err(BatchPayError::InvalidAmount),
        }

        let processed_key = DataKey::ProcessedBatch(batch_id);
        if env.storage().persistent().has(&processed_key) {
            return Err(BatchPayError::DuplicateBatch);
        }

        env.storage().instance().set(&DataKey::Processing, &true);
        env.storage().persistent().set(&processed_key, &true);

        let contract_address = env.current_contract_address();
        let token_client = token::Client::new(&env, &config.token);
        for index in 0..recipients.len() {
            let recipient = recipients.get(index).unwrap();
            let amount = amounts.get(index).unwrap();
            env.authorize_as_current_contract(&[InvokerContractAuthEntry::Contract(
                SubContractInvocation {
                    context: ContractContext {
                        contract: config.token.clone(),
                        fn_name: symbol_short!("transfer"),
                        args: (
                            contract_address.clone(),
                            recipient.clone(),
                            amount,
                        )
                            .into_val(&env),
                    },
                    sub_invocations: vec![&env],
                },
            )]);
            token_client.transfer(&contract_address, &recipient, &amount);
        }

        env.storage().instance().set(&DataKey::Processing, &false);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::segmentation::MAX_BATCH_SIZE;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        let token_address = token_id.address();
        let token_admin = StellarAssetClient::new(&env, &token_address);
        let contract_id = env.register_contract(None, BatchPayContract);
        let client = BatchPayContractClient::new(&env, &contract_id);
        client.initialize(&admin, &token_address);
        token_admin.mint(&contract_id, &1_000i128);

        (env, token_address, contract_id)
    }

    #[test]
    fn transfers_batch_and_rejects_replay() {
        let (env, token_address, contract_id) = setup();
        let client = BatchPayContractClient::new(&env, &contract_id);
        let token = token::Client::new(&env, &token_address);
        let first = Address::generate(&env);
        let second = Address::generate(&env);
        let batch_id = BytesN::from_array(&env, &[7; 32]);
        let recipients = vec![&env, first.clone(), second.clone()];
        let amounts = vec![&env, 125_i128, 375_i128];

        client.process_batch(&batch_id, &recipients, &amounts);

        assert_eq!(token.balance(&first), 125);
        assert_eq!(token.balance(&second), 375);

        let result = client.try_process_batch(&batch_id, &recipients, &amounts);
        assert!(matches!(result, Ok(Err(BatchPayError::DuplicateBatch))));
    }

    #[test]
    fn separate_contract_instances_have_independent_batch_namespaces() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        let token_address = token_id.address();
        let token_admin = StellarAssetClient::new(&env, &token_address);
        let token = token::Client::new(&env, &token_address);
        let first_contract = env.register_contract(None, BatchPayContract);
        let second_contract = env.register_contract(None, BatchPayContract);
        let first_client = BatchPayContractClient::new(&env, &first_contract);
        let second_client = BatchPayContractClient::new(&env, &second_contract);
        let first_recipient = Address::generate(&env);
        let second_recipient = Address::generate(&env);
        let batch_id = BytesN::from_array(&env, &[9; 32]);

        first_client.initialize(&admin, &token_address);
        second_client.initialize(&admin, &token_address);
        token_admin.mint(&first_contract, &100_i128);
        token_admin.mint(&second_contract, &100_i128);

        first_client.process_batch(
            &batch_id,
            &vec![&env, first_recipient.clone()],
            &vec![&env, 40_i128],
        );
        second_client.process_batch(
            &batch_id,
            &vec![&env, second_recipient.clone()],
            &vec![&env, 60_i128],
        );

        assert_eq!(token.balance(&first_recipient), 40);
        assert_eq!(token.balance(&second_recipient), 60);
    }

    #[test]
    fn enforces_batch_size_limit() {
        let env = Env::default();
        let mut recipients = Vec::new(&env);
        let mut amounts = Vec::new(&env);
        for _ in 0..=MAX_BATCH_SIZE {
            recipients.push_back(Address::generate(&env));
            amounts.push_back(1_i128);
        }

        assert_eq!(
            segmentation::validate_batch(&recipients, &amounts),
            Err(ValidationError::TooLarge)
        );
    }
}