//! Storage keys, configuration types, errors and events.

use soroban_sdk::{contracterror, contractevent, contracttype, Address, String, Vec};

/// Basis-point denominator (100% = 10_000).
pub const BPS: u32 = 10_000;
/// Parts-per-million denominator used by the allocation view.
pub const PPM: u32 = 1_000_000;
/// Upper bound on basket size; keeps per-call oracle/token fan-out bounded.
pub const MAX_COMPONENTS: u32 = 10;
/// Upper bound on token precision accepted for components and the basket.
pub const MAX_DECIMALS: u32 = 18;
/// Upper bound on the rebalance incentive (1%).
pub const MAX_INCENTIVE_BPS: u32 = 100;

pub(crate) const DAY_IN_LEDGERS: u32 = 17_280;
pub(crate) const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
pub(crate) const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;
pub(crate) const BALANCE_BUMP: u32 = 60 * DAY_IN_LEDGERS;
pub(crate) const BALANCE_THRESHOLD: u32 = BALANCE_BUMP - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BasketError {
    InvalidConfig = 1,
    TooManyComponents = 2,
    DuplicateComponent = 3,
    InvalidWeights = 4,
    InvalidDecimals = 5,
    InvalidAmount = 6,
    LengthMismatch = 7,
    InsufficientBalance = 8,
    InsufficientAllowance = 9,
    InsufficientSupply = 10,
    SlippageExceeded = 11,
    NotArbitrageur = 12,
    UnknownComponent = 13,
    SameAsset = 14,
    PriceUnavailable = 15,
    StalePrice = 16,
    NotUnderweight = 17,
    NotOverweight = 18,
    Overshoot = 19,
    Overflow = 20,
    InvalidExpiration = 21,
}

/// A component as supplied by the basket creator.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentSpec {
    /// SEP-41 token contract of the underlying asset.
    pub token: Address,
    /// Underlying base units backing one whole basket token
    /// (`10^decimals` share units) when the basket is bootstrapped.
    pub units: i128,
    /// Target value allocation in basis points; all weights sum to 10_000.
    pub weight_bps: u32,
}

/// Full basket configuration passed to the constructor (and by the factory).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasketConfig {
    pub name: String,
    pub symbol: String,
    pub decimals: u32,
    pub components: Vec<ComponentSpec>,
    /// SEP-40 price oracle used for rebalancing valuations.
    pub oracle: Address,
    /// Maximum accepted oracle price age, in seconds.
    pub max_price_age: u64,
    /// Premium paid to arbitrageurs on the outgoing leg, in basis points.
    pub rebalance_incentive_bps: u32,
}

/// A stored component, including the precision read from its token contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Component {
    pub token: Address,
    pub decimals: u32,
    pub units: i128,
    pub weight_bps: u32,
}

/// Parameters governing oracle-guided rebalancing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebalanceParams {
    pub oracle: Address,
    pub max_price_age: u64,
    pub incentive_bps: u32,
}

/// Current vs. target allocation of one component.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Allocation {
    pub token: Address,
    pub reserve: i128,
    pub target_ppm: u32,
    pub weight_ppm: u32,
}

/// Largest trade an arbitrageur may execute for a given pair right now.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebalanceQuote {
    pub max_amount_in: i128,
    pub amount_out: i128,
}

#[contracttype]
#[derive(Clone)]
pub(crate) enum DataKey {
    Admin,
    Name,
    Symbol,
    Decimals,
    Components,
    Reserves,
    TotalSupply,
    Params,
    Arbitrageur(Address),
    Balance(Address),
    Allowance(AllowanceKey),
}

#[contracttype]
#[derive(Clone)]
pub(crate) struct AllowanceKey {
    pub from: Address,
    pub spender: Address,
}

#[contracttype]
#[derive(Clone)]
pub(crate) struct AllowanceValue {
    pub amount: i128,
    pub expiration_ledger: u32,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Basket tokens issued against a full set of underlying deposits.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Issued {
    #[topic]
    pub to: Address,
    pub shares: i128,
    pub amounts: Vec<i128>,
}

/// Basket tokens redeemed for the full set of underlying assets.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Redeemed {
    #[topic]
    pub from: Address,
    pub shares: i128,
    pub amounts: Vec<i128>,
}

/// An arbitrageur swapped an underweight asset in for an overweight one.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rebalanced {
    #[topic]
    pub arbitrageur: Address,
    pub asset_in: Address,
    pub amount_in: i128,
    pub asset_out: Address,
    pub amount_out: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArbitrageurSet {
    #[topic]
    pub arbitrageur: Address,
    pub enabled: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamsUpdated {
    pub params: RebalanceParams,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChanged {
    pub admin: Address,
}

// SEP-41 token events.

#[contractevent(data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transfer {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent(data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approve {
    #[topic]
    pub from: Address,
    #[topic]
    pub spender: Address,
    pub amount: i128,
    pub expiration_ledger: u32,
}

#[contractevent(data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mint {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent(data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Burn {
    #[topic]
    pub from: Address,
    pub amount: i128,
}
