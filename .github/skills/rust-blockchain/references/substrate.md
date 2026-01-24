# Substrate Framework Guide

Building blockchain applications with Substrate framework in Rust.

## Getting Started with Substrate

### Installation

```bash
# Install Substrate dependencies
curl https://getsubstrate.io -sSf | bash -s -- --fast

# Create new Substrate node
cargo install --git https://github.com/substrate-developer-hub/substrate-node-template
substrate-node-template --dev
```

### Project Structure

```
substrate-node/
├── Cargo.toml
├── node/              # Node implementation
│   ├── Cargo.toml
│   └── src/
│       ├── chain_spec.rs
│       ├── cli.rs
│       ├── command.rs
│       ├── rpc.rs
│       └── service.rs
├── pallets/           # Custom pallets (modules)
│   └── template/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── mock.rs
│           └── tests.rs
└── runtime/           # Runtime logic
    ├── Cargo.toml
    └── src/
        └── lib.rs
```

## Building Custom Pallets

### Basic Pallet Structure

```rust
#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    
    #[pallet::pallet]
    pub struct Pallet<T>(_);
    
    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        
        /// Maximum number of items
        #[pallet::constant]
        type MaxItems: Get<u32>;
    }
    
    /// Storage for items
    #[pallet::storage]
    #[pallet::getter(fn items)]
    pub type Items<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<u128, T::MaxItems>,
        ValueQuery,
    >;
    
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        ItemAdded { who: T::AccountId, item: u128 },
        ItemRemoved { who: T::AccountId, item: u128 },
    }
    
    #[pallet::error]
    pub enum Error<T> {
        TooManyItems,
        ItemNotFound,
        Unauthorized,
    }
    
    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(10_000)]
        pub fn add_item(
            origin: OriginFor<T>,
            item: u128,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            
            Items::<T>::try_mutate(&who, |items| {
                items.try_push(item)
                    .map_err(|_| Error::<T>::TooManyItems)?;
                Ok(())
            })?;
            
            Self::deposit_event(Event::ItemAdded { who, item });
            Ok(())
        }
        
        #[pallet::weight(10_000)]
        pub fn remove_item(
            origin: OriginFor<T>,
            item: u128,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            
            Items::<T>::try_mutate(&who, |items| {
                let pos = items.iter()
                    .position(|&x| x == item)
                    .ok_or(Error::<T>::ItemNotFound)?;
                items.remove(pos);
                Ok(())
            })?;
            
            Self::deposit_event(Event::ItemRemoved { who, item });
            Ok(())
        }
    }
}
```

### Token Pallet (ERC20-like)

```rust
#[frame_support::pallet]
pub mod pallet {
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    
    #[pallet::pallet]
    pub struct Pallet<T>(_);
    
    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type Balance: Member + Parameter + AtLeast32BitUnsigned + Default + Copy + MaxEncodedLen;
    }
    
    #[pallet::storage]
    pub type Balances<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        T::Balance,
        ValueQuery,
    >;
    
    #[pallet::storage]
    pub type Allowances<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        T::AccountId,  // owner
        Blake2_128Concat,
        T::AccountId,  // spender
        T::Balance,
        ValueQuery,
    >;
    
    #[pallet::storage]
    pub type TotalSupply<T: Config> = StorageValue<_, T::Balance, ValueQuery>;
    
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        Transfer { from: T::AccountId, to: T::AccountId, amount: T::Balance },
        Approval { owner: T::AccountId, spender: T::AccountId, amount: T::Balance },
        Mint { to: T::AccountId, amount: T::Balance },
        Burn { from: T::AccountId, amount: T::Balance },
    }
    
    #[pallet::error]
    pub enum Error<T> {
        InsufficientBalance,
        InsufficientAllowance,
        Overflow,
    }
    
    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(10_000)]
        pub fn transfer(
            origin: OriginFor<T>,
            to: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let from = ensure_signed(origin)?;
            Self::do_transfer(&from, &to, amount)?;
            Ok(())
        }
        
        #[pallet::weight(10_000)]
        pub fn approve(
            origin: OriginFor<T>,
            spender: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let owner = ensure_signed(origin)?;
            
            Allowances::<T>::insert(&owner, &spender, amount);
            Self::deposit_event(Event::Approval { owner, spender, amount });
            
            Ok(())
        }
        
        #[pallet::weight(10_000)]
        pub fn transfer_from(
            origin: OriginFor<T>,
            from: T::AccountId,
            to: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let spender = ensure_signed(origin)?;
            
            // Check allowance
            let allowance = Allowances::<T>::get(&from, &spender);
            ensure!(allowance >= amount, Error::<T>::InsufficientAllowance);
            
            // Update allowance
            Allowances::<T>::insert(&from, &spender, allowance - amount);
            
            // Transfer
            Self::do_transfer(&from, &to, amount)?;
            
            Ok(())
        }
        
        #[pallet::weight(10_000)]
        pub fn mint(
            origin: OriginFor<T>,
            to: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            ensure_root(origin)?;
            
            let new_balance = Balances::<T>::get(&to)
                .checked_add(&amount)
                .ok_or(Error::<T>::Overflow)?;
            
            Balances::<T>::insert(&to, new_balance);
            
            let new_supply = TotalSupply::<T>::get()
                .checked_add(&amount)
                .ok_or(Error::<T>::Overflow)?;
            TotalSupply::<T>::put(new_supply);
            
            Self::deposit_event(Event::Mint { to, amount });
            Ok(())
        }
    }
    
    impl<T: Config> Pallet<T> {
        fn do_transfer(
            from: &T::AccountId,
            to: &T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let from_balance = Balances::<T>::get(from);
            ensure!(from_balance >= amount, Error::<T>::InsufficientBalance);
            
            let to_balance = Balances::<T>::get(to);
            
            Balances::<T>::insert(from, from_balance - amount);
            Balances::<T>::insert(to, to_balance + amount);
            
            Self::deposit_event(Event::Transfer {
                from: from.clone(),
                to: to.clone(),
                amount,
            });
            
            Ok(())
        }
    }
}
```

## Runtime Configuration

### Configuring the Runtime

```rust
// runtime/src/lib.rs
use frame_support::{
    construct_runtime, parameter_types,
    traits::{ConstU32, ConstU64},
    weights::Weight,
};
use sp_runtime::{
    create_runtime_str, generic, impl_opaque_keys,
    traits::{BlakeTwo256, Block as BlockT, IdentifyAccount, Verify},
    MultiSignature,
};

pub type Signature = MultiSignature;
pub type AccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;
pub type Balance = u128;
pub type Index = u32;
pub type Hash = sp_core::H256;
pub type BlockNumber = u32;

// System configuration
parameter_types! {
    pub const BlockHashCount: BlockNumber = 250;
    pub const MaximumBlockWeight: Weight = Weight::from_parts(2_000_000_000_000, 0);
    pub const MaximumBlockLength: u32 = 5 * 1024 * 1024;
}

impl frame_system::Config for Runtime {
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Index = Index;
    type BlockNumber = BlockNumber;
    type Hash = Hash;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type Lookup = sp_runtime::traits::AccountIdLookup<AccountId, ()>;
    type Header = generic::Header<BlockNumber, BlakeTwo256>;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = BlockHashCount;
    type Version = Version;
    type PalletInfo = PalletInfo;
    type AccountData = pallet_balances::AccountData<Balance>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type SS58Prefix = ();
    type OnSetCode = ();
    type MaxConsumers = ConstU32<16>;
}

// Token pallet configuration
impl pallet_token::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type Balance = Balance;
}

// Construct the runtime
construct_runtime!(
    pub enum Runtime where
        Block = Block,
        NodeBlock = Block,
        UncheckedExtrinsic = UncheckedExtrinsic
    {
        System: frame_system,
        Timestamp: pallet_timestamp,
        Balances: pallet_balances,
        Token: pallet_token,
    }
);
```

## Advanced Features

### Off-Chain Workers

```rust
use sp_runtime::offchain::{http, Duration};

#[pallet::hooks]
impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
    fn offchain_worker(block_number: T::BlockNumber) {
        // Fetch data from external API
        if let Err(e) = Self::fetch_price_and_send_signed() {
            log::error!("Error in offchain worker: {:?}", e);
        }
    }
}

impl<T: Config> Pallet<T> {
    fn fetch_price_and_send_signed() -> Result<(), &'static str> {
        // Make HTTP request
        let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
        let request = http::Request::get("https://api.example.com/price");
        
        let pending = request
            .deadline(deadline)
            .send()
            .map_err(|_| "HTTP request failed")?;
        
        let response = pending
            .try_wait(deadline)
            .map_err(|_| "HTTP timeout")?
            .map_err(|_| "HTTP error")?;
        
        if response.code != 200 {
            return Err("HTTP status error");
        }
        
        let body = response.body().collect::<Vec<u8>>();
        let price: u128 = serde_json::from_slice(&body)
            .map_err(|_| "JSON parse error")?;
        
        // Submit signed transaction
        let signer = Signer::<T, T::AuthorityId>::all_accounts();
        let results = signer.send_signed_transaction(|_account| {
            Call::submit_price { price }
        });
        
        Ok(())
    }
}
```

### Chain Extensions for Smart Contracts

```rust
use pallet_contracts::chain_extension::{
    ChainExtension, Environment, Ext, InitState, RetVal, SysConfig,
};

pub struct MyChainExtension;

impl ChainExtension<Runtime> for MyChainExtension {
    fn call<E: Ext>(&mut self, env: Environment<E, InitState>) -> Result<RetVal, DispatchError>
    where
        E: Ext<T = Runtime>,
    {
        let func_id = env.func_id();
        
        match func_id {
            // Custom function to get oracle price
            1000 => {
                let asset_id: u32 = env.read_as()?;
                let price = pallet_oracle::Pallet::<Runtime>::get_price(asset_id)
                    .ok_or(DispatchError::Other("Price not found"))?;
                
                env.write(&price.encode(), false, None)?;
                Ok(RetVal::Converging(0))
            }
            _ => Err(DispatchError::Other("Unknown function")),
        }
    }
}
```

### Benchmarking Weights

```rust
#[cfg(feature = "runtime-benchmarks")]
use frame_benchmarking::{benchmarks, whitelisted_caller};

benchmarks! {
    transfer {
        let caller: T::AccountId = whitelisted_caller();
        let recipient: T::AccountId = account("recipient", 0, 0);
        let amount = 100u32.into();
        
        // Setup
        Balances::<T>::insert(&caller, 1000u32.into());
        
    }: _(RawOrigin::Signed(caller.clone()), recipient.clone(), amount)
    verify {
        assert_eq!(Balances::<T>::get(&caller), 900u32.into());
        assert_eq!(Balances::<T>::get(&recipient), 100u32.into());
    }
    
    approve {
        let caller: T::AccountId = whitelisted_caller();
        let spender: T::AccountId = account("spender", 0, 0);
        let amount = 100u32.into();
        
    }: _(RawOrigin::Signed(caller.clone()), spender.clone(), amount)
    verify {
        assert_eq!(Allowances::<T>::get(&caller, &spender), amount);
    }
}
```

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use frame_support::{assert_ok, assert_noop};
    use sp_core::H256;
    use sp_runtime::{
        testing::Header,
        traits::{BlakeTwo256, IdentityLookup},
    };
    
    type UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;
    type Block = frame_system::mocking::MockBlock<Test>;
    
    frame_support::construct_runtime!(
        pub enum Test where
            Block = Block,
            NodeBlock = Block,
            UncheckedExtrinsic = UncheckedExtrinsic,
        {
            System: frame_system,
            Token: pallet_token,
        }
    );
    
    impl frame_system::Config for Test {
        type BaseCallFilter = frame_support::traits::Everything;
        type BlockWeights = ();
        type BlockLength = ();
        type DbWeight = ();
        type RuntimeOrigin = RuntimeOrigin;
        type RuntimeCall = RuntimeCall;
        type Index = u64;
        type BlockNumber = u64;
        type Hash = H256;
        type Hashing = BlakeTwo256;
        type AccountId = u64;
        type Lookup = IdentityLookup<Self::AccountId>;
        type Header = Header;
        type RuntimeEvent = RuntimeEvent;
        type BlockHashCount = ();
        type Version = ();
        type PalletInfo = PalletInfo;
        type AccountData = ();
        type OnNewAccount = ();
        type OnKilledAccount = ();
        type SystemWeightInfo = ();
        type SS58Prefix = ();
        type OnSetCode = ();
        type MaxConsumers = frame_support::traits::ConstU32<16>;
    }
    
    impl Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type Balance = u128;
    }
    
    fn new_test_ext() -> sp_io::TestExternalities {
        frame_system::GenesisConfig::default()
            .build_storage::<Test>()
            .unwrap()
            .into()
    }
    
    #[test]
    fn transfer_works() {
        new_test_ext().execute_with(|| {
            Balances::<Test>::insert(1, 1000);
            
            assert_ok!(Token::transfer(RuntimeOrigin::signed(1), 2, 100));
            
            assert_eq!(Balances::<Test>::get(1), 900);
            assert_eq!(Balances::<Test>::get(2), 100);
        });
    }
    
    #[test]
    fn transfer_insufficient_balance_fails() {
        new_test_ext().execute_with(|| {
            Balances::<Test>::insert(1, 50);
            
            assert_noop!(
                Token::transfer(RuntimeOrigin::signed(1), 2, 100),
                Error::<Test>::InsufficientBalance
            );
        });
    }
}
```

## Deployment

### Chain Specification

```rust
// node/src/chain_spec.rs
use sc_service::ChainType;
use sp_core::{sr25519, Pair, Public};
use sp_runtime::traits::{IdentifyAccount, Verify};

pub fn development_config() -> Result<ChainSpec, String> {
    Ok(ChainSpec::from_genesis(
        "Development",
        "dev",
        ChainType::Development,
        move || {
            testnet_genesis(
                vec![authority_keys_from_seed("Alice")],
                get_account_id_from_seed::<sr25519::Public>("Alice"),
                vec![
                    get_account_id_from_seed::<sr25519::Public>("Alice"),
                    get_account_id_from_seed::<sr25519::Public>("Bob"),
                ],
                true,
            )
        },
        vec![],
        None,
        None,
        None,
        None,
        None,
    ))
}

fn testnet_genesis(
    initial_authorities: Vec<(AccountId, AuraId, GrandpaId)>,
    root_key: AccountId,
    endowed_accounts: Vec<AccountId>,
    enable_println: bool,
) -> GenesisConfig {
    GenesisConfig {
        system: SystemConfig {
            code: wasm_binary_unwrap().to_vec(),
        },
        balances: BalancesConfig {
            balances: endowed_accounts
                .iter()
                .cloned()
                .map(|k| (k, 1 << 60))
                .collect(),
        },
        // ... other pallet configs
    }
}
```

### Running the Node

```bash
# Build the node
cargo build --release

# Run in development mode
./target/release/node-template --dev

# Run with custom chain spec
./target/release/node-template --chain=local --validator

# Purge chain data
./target/release/node-template purge-chain --dev
```
