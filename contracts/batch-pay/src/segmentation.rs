use soroban_sdk::{contracttype, Address, BytesN, Env, Vec};

pub const MAX_BATCH_SIZE: u32 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationError {
    Empty,
    TooLarge,
    LengthMismatch,
    InvalidAmount,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    Processing,
    ProcessedBatch(BytesN<32>),
}

#[derive(Clone)]
#[contracttype]
pub struct Config {
    pub admin: Address,
    pub token: Address,
}

pub fn validate_batch(
    recipients: &Vec<Address>,
    amounts: &Vec<i128>,
) -> Result<(), ValidationError> {
    let count = recipients.len();
    if count == 0 {
        return Err(ValidationError::Empty);
    }
    if count > MAX_BATCH_SIZE {
        return Err(ValidationError::TooLarge);
    }
    if count != amounts.len() {
        return Err(ValidationError::LengthMismatch);
    }
    for index in 0..count {
        if amounts.get(index).unwrap() <= 0 {
            return Err(ValidationError::InvalidAmount);
        }
    }
    Ok(())
}

pub fn is_processing(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Processing)
        .unwrap_or(false)
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec};

    #[test]
    fn rejects_empty_mismatched_and_non_positive_batches() {
        let env = Env::default();
        let recipient = Address::generate(&env);

        assert_eq!(
            validate_batch(&Vec::new(&env), &Vec::new(&env)),
            Err(ValidationError::Empty)
        );
        assert_eq!(
            validate_batch(&vec![&env, recipient.clone()], &Vec::new(&env)),
            Err(ValidationError::LengthMismatch)
        );
        assert_eq!(
            validate_batch(&vec![&env, recipient.clone()], &vec![&env, 0_i128]),
            Err(ValidationError::InvalidAmount)
        );
        assert_eq!(
            validate_batch(&vec![&env, recipient], &vec![&env, -1_i128]),
            Err(ValidationError::InvalidAmount)
        );
    }
}