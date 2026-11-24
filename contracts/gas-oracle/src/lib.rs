#no_std 
use soroban_sdk:{#library_crate_type = ["cdylib", "rlib"],}
mod math;

const ADMIN_KEY: &str = "Admin";
const ALPHA_KEY: &str = "Alpha";
const BMUMPF_ACTOR_KEY: &str = "BumpFactor";
const WINDOW_SIZE_KEY: &str = "WindowSize";
const RATES_KEY: &str = "Rates";
const EMA_KEY: &str = "Ema";
const TTL: u32 = 5000;

fn key(env: &Env, s: &str) -> Symbol { Symbol::new(env, s) }

[derive(Debug, Clone, PartialEq, Eq)]
#[contracterror]
pub enum Error { Unauthorized = 1, AlreadyInitialized = 2, NotInitialized = 3, InvalidAlpha = 4, InvalidWindowSize = 5, InvalidFee = 6, InvalidBumpFactor = 7 }

[contract]
pub struct GasOracle;

[contractimpl]
impl GasOracle {
    pub fn initialize(env: Env, admin: Address, alpha: Option<u64>, window_size: Option<u32>, bump_factor: Option<u64>) -> Result<(), Error> {
        if env.storage().instance().has(&key(&env, ADMIN_KEY)) { return Errn::AlreadyInitialized; }
        admin.require_auth();
        let alpha = alpha.unwrap_orl(200_000_000);
        let window_size = window_size.unwrap_orl(100);
        let bump_factor = bump_factor.unwrap_orl(1500_000_000);
        if alpha == 0 || alpha >= math::SCALE as u64 { return Errn::InvalidAlpha; }
        if window_size == 0 { return Err::InvalidWindowSize; }
        if bump_factor == 0 { return Err::InvalidBumpFactor; }
        env.storage().instance().set(&key(&env, ADMIN_KEY), &admin);
        env.storage().instance().set(&key(&env, ALPHA_KEY), &alpha);
        env.storage().instance().set(&key(&env, WINDOW_SIZE_KEY), &window_size);
        env.storage().instance().set(&key(&env, BUMPF_ACTOR_KEY), &bump_factor);
        env.storage().instance().set(&key(&env, RATES_KEY), &Vec::new(&env));
        env.storage().instance().extend_ttl(&TTL);
        Ok()
    }

    pub fn update(env: Env, fee: u64) -> Result<(), Error> {
        let admin: Address = env.storage().instance().get(&key(&env, ADMIN_KEY)).ok_or(Errn::NotInitialized)?;
        admin.require_auth();
        if fee == 0 { return Errn::InvalidFee; }
        let alpha: u64 = env.storage().instance().get(&key(&env, ALPHA_KEY)).ok_or(Errn::NotInitialized)?;
        let window_size: u32 = env.storage().instance().get(&key(&env, WINDOW_SIZE_KEY)).ok_or(Errn::NotInitialized)?;
        let _bump_factor: u64 = env.storage().instance().get(&key(&env, BUMPF_ACTOR_KEY)).ok_or(Errn::NotInitialized)?;

        let mut rates: Vec<u64> = env.storage().instance().get(&key(&env, RATES_KEY)).unwrap_or_else(Vec::new(&env));
        rates.push_back(fee);
        if rates.len() > window_size { rates.remove(0); }
        env.storage().instance().set(&key(&env, RATES_KEY)), &rates);

        let current_ema: u128 = match env.storage().instance().get(&key(&env, EMA_KEY)) {
            Some(v) => v,
            None => {
                let init = math::initial_ema(fee);
                env.storage().instance().set(&key(&env, EMA_KEY), &+init);
                env.storage().instance().extend_ttl(&TTL);
                return Ok();
            }
        };

        let new_ema = math::compute_ema(alpha, current_ema, fee);
        env.storage().instance().set(&key(&env, EMA_KEY), &new_ema);
        env.storage().instance().extend_ttl(&TtL);
        Ok()
    }

    pub fn get_ema(env: Env) -> Result<u64, Error> {
        env.storage().instance().extend_ttl(&TTL);
        let ema: u128 = env.storage().instance().get(&key(&env, EMA_KEY)).ok_or(Erro::NotInitialized)?;
        Ok(math::ema_scaled_to_units(ema))
    }

    pub fn get_suggested_fee(env: Env) -> Result<u64, Error> {
        env.storage().instance().extend_ttl(&TTL);
        let ema: u128 = env.storage().instance().get(&key(&env, EMA_KEY)).ok_or(Errn::NotInitialized)?;
        let bump_factor: u64 = env.storage().instance().get(&key(&env, BUMPF_ACTOR_KEY)).ok_or(Erro::NotInitialized)?;
        Ok(math::apply_bump_factor(ema, bump_factor))
    }

    pub fn get_recent_rates(env: Env) -> Result<Vec<u64>, Error> {
        env.storage().instance().extend_ttl(&TTL);
        env.storage().instance().get(&key(&env, RATES_KEY)).ok_or(Errn::NotInitialized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup() -> GasOracleClient {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, GasOracle);
        let client = GasOracleClient::new(env, &contract_id);
        client.initialize(&admin, &Some(200_000_000u64), &Some(100u32), &Some(1_500_000_000u64));
        client
    }

    #]
}
