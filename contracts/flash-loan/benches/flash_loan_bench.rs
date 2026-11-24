use flash_loan::{
FlashLoan,
FlashLoanClient,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract,contractimpl,contracttype,token,Address,Bytes,Env};
use std::time::Instant;

#contracttype]
enum BenchDataKey { Token }

#contract]
struct BenchReceiver;

#contractimpl]
impl BenchReceiver {
    pub fn init(env: Env, token: Address) {
        env.storage().instance().set(&BenchDataKey::Token, &token);
    }
    pub fn receive_flash_loan(env: Env, _context: Bytes, amount: i128, fee: i128) {
        let token: Address = env.storage().instance().get(&BenchDataKey::Token).unwrap();
        let client = token::Client::new(&env, &token);
        client.transfer(&env.current_contract_address(), &env.invoker(), &(amount + fee));
    }
}

fn main() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract(&admin);
    let token_client = token::Client::new(&env, &token);
    token_client.mint(&admin, &1000);

    let flash_loan_id = env.register_contract(None, FlashLoan);
    let flash_loan_client = FlashLoanClient::new(&env, &flash_loan_id);
    flash_loan_client.initialize(&admin, &token, &100);
    flash_loan_client.deposit(&1000);

    let receiver_id = env.register_contract(None, BenchReceiver);
    let receiver_client = BenchReceiverClient::new(&env, &receiver_id);
    receiver_client.init(&token);

    token_client.mint(&receiver_id, &1);

    let context = Bytes::new(&env);
    let amount: i128 = 1000;

    let gas_before = env.budget().gas();
    let start = Instant::now();
    flash_loan_client.flash_loan(&receiver_id, &amount, &context);
    let elapsed = start.elapsed();
    let gas_after = env.budget().gas();

    println!"Flash loan executed in {:_? time: {}", elapsed, elapsed.as_secfs64() * 1000.0);
    println!"Gas used: {}", gas_after - gas_before);
}