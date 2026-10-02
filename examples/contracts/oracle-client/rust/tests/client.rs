//! Exercises the client against a mock oracle that follows the specification
//! in `docs/architecture/gas-oracle.md`.

use gas_oracle_client::{ema, quote_fee, GasOracleClient, OracleConfig, OracleError, OracleState};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol};

#[contracttype]
enum DataKey {
    Admin,
    Config,
    State,
    Reporter(Address),
}

#[contract]
struct MockOracle;

fn admin(env: &Env) -> Result<Address, OracleError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(OracleError::NotInitialized)
}

fn load_config(env: &Env) -> Result<OracleConfig, OracleError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(OracleError::NotInitialized)
}

fn validate(config: &OracleConfig) -> Result<(), OracleError> {
    let alpha_ok = (1..=ema::ALPHA_DENOMINATOR).contains(&config.alpha_bps);
    let bounds_ok = config.min_fee >= 1 && config.min_fee <= config.max_fee;
    if alpha_ok && bounds_ok && config.max_age_ledgers >= 1 {
        Ok(())
    } else {
        Err(OracleError::InvalidConfig)
    }
}

#[contractimpl]
impl MockOracle {
    pub fn initialize(env: Env, admin: Address, config: OracleConfig) -> Result<(), OracleError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(OracleError::AlreadyInitialized);
        }
        admin.require_auth();
        validate(&config)?;
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn set_config(env: Env, config: OracleConfig) -> Result<(), OracleError> {
        admin(&env)?.require_auth();
        validate(&config)?;
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn add_reporter(env: Env, reporter: Address) -> Result<(), OracleError> {
        admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Reporter(reporter), &true);
        Ok(())
    }

    pub fn remove_reporter(env: Env, reporter: Address) -> Result<(), OracleError> {
        admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::Reporter(reporter));
        Ok(())
    }

    pub fn submit(env: Env, reporter: Address, fee_per_op: u64) -> Result<u64, OracleError> {
        let config = load_config(&env)?;
        reporter.require_auth();
        if !env.storage().persistent().has(&DataKey::Reporter(reporter)) {
            return Err(OracleError::Unauthorized);
        }
        if fee_per_op < config.min_fee || fee_per_op > config.max_fee {
            return Err(OracleError::ObservationOutOfRange);
        }

        let ledger = env.ledger().sequence();
        let state = match env
            .storage()
            .instance()
            .get::<_, OracleState>(&DataKey::State)
        {
            Some(prev) if prev.last_ledger == ledger => return Err(OracleError::AlreadyUpdated),
            Some(prev) => OracleState {
                ema: ema::update(prev.ema, fee_per_op, config.alpha_bps),
                last_observation: fee_per_op,
                last_ledger: ledger,
                samples: prev.samples + 1,
            },
            None => OracleState {
                ema: ema::to_fixed(fee_per_op),
                last_observation: fee_per_op,
                last_ledger: ledger,
                samples: 1,
            },
        };
        env.storage().instance().set(&DataKey::State, &state);
        Self::fee_per_op(env)
    }

    pub fn fee_per_op(env: Env) -> Result<u64, OracleError> {
        let config = load_config(&env)?;
        let state = Self::state(env.clone())?;
        if env.ledger().sequence() - state.last_ledger > config.max_age_ledgers {
            return Err(OracleError::Stale);
        }
        Ok(ema::to_stroops_ceil(state.ema).clamp(config.min_fee, config.max_fee))
    }

    pub fn estimate_fee(env: Env, ops: u32) -> Result<u64, OracleError> {
        Self::fee_per_op(env)?
            .checked_mul(ops as u64)
            .ok_or(OracleError::Overflow)
    }

    pub fn state(env: Env) -> Result<OracleState, OracleError> {
        load_config(&env)?;
        env.storage()
            .instance()
            .get(&DataKey::State)
            .ok_or(OracleError::NoData)
    }

    pub fn config(env: Env) -> Result<OracleConfig, OracleError> {
        load_config(&env)
    }
}

struct Fixture {
    env: Env,
    oracle: Address,
    admin: Address,
    reporter: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_sequence_number(1_000);

        let oracle = env.register(MockOracle, ());
        let admin = Address::generate(&env);
        let reporter = Address::generate(&env);

        let client = GasOracleClient::new(&env, &oracle);
        client.initialize(&admin, &default_config());
        client.add_reporter(&reporter);

        Self {
            env,
            oracle,
            admin,
            reporter,
        }
    }

    fn client(&self) -> GasOracleClient<'_> {
        GasOracleClient::new(&self.env, &self.oracle)
    }

    fn advance(&self, ledgers: u32) {
        let next = self.env.ledger().sequence() + ledgers;
        self.env.ledger().set_sequence_number(next);
    }
}

fn default_config() -> OracleConfig {
    OracleConfig {
        alpha_bps: 2_000,
        min_fee: 100,
        max_fee: 1_000_000,
        max_age_ledgers: 12,
    }
}

#[test]
fn submit_updates_ema_as_specified() {
    let f = Fixture::new();
    let client = f.client();

    assert_eq!(client.submit(&f.reporter, &100), 100);
    f.advance(1);
    assert_eq!(client.submit(&f.reporter, &150), 110);
    f.advance(1);
    assert_eq!(client.submit(&f.reporter, &137), 116);

    let state = client.state();
    assert_eq!(state.ema, 1_154_000_000);
    assert_eq!(state.last_observation, 137);
    assert_eq!(state.samples, 3);
    assert_eq!(client.estimate_fee(&4), 464);
}

#[test]
fn quote_fee_propagates_oracle_errors() {
    let f = Fixture::new();

    assert_eq!(quote_fee(&f.env, &f.oracle, 1), Err(OracleError::NoData));

    f.client().submit(&f.reporter, &250);
    assert_eq!(quote_fee(&f.env, &f.oracle, 3), Ok(750));
    assert_eq!(
        quote_fee(&f.env, &f.oracle, u32::MAX),
        Ok(250 * u32::MAX as u64)
    );

    f.advance(13);
    assert_eq!(quote_fee(&f.env, &f.oracle, 1), Err(OracleError::Stale));
}

#[test]
fn estimate_fee_reports_overflow() {
    let f = Fixture::new();
    let client = f.client();
    client.set_config(&OracleConfig {
        max_fee: u64::MAX,
        ..default_config()
    });
    client.submit(&f.reporter, &u64::MAX);

    assert_eq!(client.try_estimate_fee(&2), Err(Ok(OracleError::Overflow)));
}

#[test]
fn submit_enforces_reporter_rules() {
    let f = Fixture::new();
    let client = f.client();
    let outsider = Address::generate(&f.env);

    assert_eq!(
        client.try_submit(&outsider, &200),
        Err(Ok(OracleError::Unauthorized))
    );
    assert_eq!(
        client.try_submit(&f.reporter, &99),
        Err(Ok(OracleError::ObservationOutOfRange))
    );

    client.submit(&f.reporter, &200);
    assert_eq!(
        client.try_submit(&f.reporter, &300),
        Err(Ok(OracleError::AlreadyUpdated))
    );

    client.remove_reporter(&f.reporter);
    f.advance(1);
    assert_eq!(
        client.try_submit(&f.reporter, &300),
        Err(Ok(OracleError::Unauthorized))
    );
}

#[test]
fn signers_match_authorization_rules() {
    let f = Fixture::new();
    let client = f.client();

    client.submit(&f.reporter, &200);
    let auths = f.env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, f.reporter);
    assert_eq!(
        auths[0].1.function,
        soroban_sdk::testutils::AuthorizedFunction::Contract((
            f.oracle.clone(),
            Symbol::new(&f.env, "submit"),
            (f.reporter.clone(), 200u64).into_val(&f.env),
        ))
    );

    let newcomer = Address::generate(&f.env);
    client.add_reporter(&newcomer);
    assert_eq!(f.env.auths()[0].0, f.admin);
}

#[test]
fn configuration_is_validated() {
    let f = Fixture::new();
    let client = f.client();

    for invalid in [
        OracleConfig {
            alpha_bps: 0,
            ..default_config()
        },
        OracleConfig {
            alpha_bps: 10_001,
            ..default_config()
        },
        OracleConfig {
            min_fee: 0,
            ..default_config()
        },
        OracleConfig {
            min_fee: 500,
            max_fee: 400,
            ..default_config()
        },
        OracleConfig {
            max_age_ledgers: 0,
            ..default_config()
        },
    ] {
        assert_eq!(
            client.try_set_config(&invalid),
            Err(Ok(OracleError::InvalidConfig))
        );
    }
    assert_eq!(
        client.try_initialize(&f.admin, &default_config()),
        Err(Ok(OracleError::AlreadyInitialized))
    );
    assert_eq!(client.config(), default_config());
}

#[test]
fn reads_fail_before_initialization() {
    let env = Env::default();
    let client = GasOracleClient::new(&env, &env.register(MockOracle, ()));

    assert_eq!(
        client.try_fee_per_op(),
        Err(Ok(OracleError::NotInitialized))
    );
    assert_eq!(client.try_config(), Err(Ok(OracleError::NotInitialized)));
}
