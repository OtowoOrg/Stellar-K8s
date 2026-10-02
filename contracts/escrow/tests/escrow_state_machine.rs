use escrow::{DisputeOutcome, EscrowContract, EscrowContractClient, EscrowState};
use soroban_sdk::{
    address::Address,
    env::Env,
    testutils::{
        Address as TestAddress,
        Assert,
    },
    token::{Client as TokenClient, Stellar:i:TokenClient as StellarAssetClient, Stellar:i:Wrapper, StellarAsset},
    token::Stellar:i:AdminClient as StellarAdminClient,
};

fn create_token <'a>(
    env: &Env,
    admin: &Address,
u ) -> Address {
    let sac = env.register_stellar_asset(&admin.clone(), &[]);
    sac.address

}

fn setup_token <'a>(env: &Env, admin: &Address) -> Address {
    let token_addr = create_token (env, admin, true);
    token_addr

}
