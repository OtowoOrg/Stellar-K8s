#![no_std]
#![allow(deprecated)]

pub mod reward;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    token::Client as TokenClient,
    Address, Env, Symbol,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    ContractPaused = 4,
    ContractNotPaused = 5,
    InvalidAmount = 6,
    InvalidDuration = 7,
    InsufficientStake = 8,
    InsufficientRewardReserve = 9,
    CalculationOverflow = 10,
    ZeroTotalStaked = 11,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    StakingToken,
    RewardToken,
    RewardRate,
    PeriodFinish,
    LastUpdateTime,
    RewardPerTokenStored,
    TotalStaked,
    TotalRewardsReserved,
    IsPaused,
    UserStake(Address),
    UserRewardPerTokenPaid(Address),
    UserRewardsEarned(Address),
}

#[contract]
pub struct StakingVaultContract;

#[contractimpl]
impl StakingVaultContract {
    /// Initialize the staking vault with admin, staking token, and reward token.
    pub fn initialize(
        env: Env,
        admin: Address,
        staking_token: Address,
        reward_token: Address,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::StakingToken, &staking_token);
        env.storage().instance().set(&DataKey::RewardToken, &reward_token);
        env.storage().instance().set(&DataKey::RewardRate, &0u128);
        env.storage().instance().set(&DataKey::PeriodFinish, &0u64);
        env.storage().instance().set(&DataKey::LastUpdateTime, &0u64);
        env.storage().instance().set(&DataKey::RewardPerTokenStored, &0u128);
        env.storage().instance().set(&DataKey::TotalStaked, &0i128);
        env.storage().instance().set(&DataKey::TotalRewardsReserved, &0i128);
        env.storage().instance().set(&DataKey::IsPaused, &false);
        Ok(())
    }

    /// Admin schedules a new reward emission over a given duration (seconds).
    pub fn notify_reward_amount(
        env: Env,
        caller: Address,
        amount: i128,
        duration: u64,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &caller)?;
        if amount <= 0 { return Err(Error::InvalidAmount); }
        if duration == 0 { return Err(Error::InvalidDuration); }
        Self::update_rewards(&env, None)?;

        let current_time = env.ledger().timestamp();
        let period_finish: u64 = env.storage().instance().get(&DataKey::PeriodFinish).unwrap_or(0);
        let reward_rate: u128 = env.storage().instance().get(&DataKey::RewardRate).unwrap_or(0);
        let (new_rate, new_finish) = reward::compute_new_reward_rate(
            current_time, period_finish, reward_rate, amount, duration,
        );

        let reward_token: Address = env.storage().instance().get(&DataKey::RewardToken).ok_or(Error::NotInitialized)?;
        TokenClient::new(&env, &reward_token).transfer(&caller, &env.current_contract_address(), &amount);

        let reserved: i128 = env.storage().instance().get(&DataKey::TotalRewardsReserved).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalRewardsReserved, &reserved.checked_add(amount).ok_or(Error::CalculationOverflow)?);
        env.storage().instance().set(&DataKey::RewardRate, &new_rate);
        env.storage().instance().set(&DataKey::PeriodFinish, &new_finish);
        env.storage().instance().set(&DataKey::LastUpdateTime, &current_time);
        env.events().publish(
            (Symbol::new(&env, "reward_added"),),
            (amount, duration, new_rate, new_finish),
        );
        Ok(())
    }

    /// Stake tokens into the vault.
    pub fn deposit(env: Env, staker: Address, amount: i128) -> Result<(), Error> {
        Self::require_not_paused(&env)?;
        if amount <= 0 { return Err(Error::InvalidAmount); }
        staker.require_auth();
        Self::update_rewards(&env, Some(&staker))?;

        let staking_token: Address = env.storage().instance().get(&DataKey::StakingToken).ok_or(Error::NotInitialized)?;
        TokenClient::new(&env, &staking_token).transfer(&staker, &env.current_contract_address(), &amount);

        let total: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        let user: i128 = env.storage().instance().get(&DataKey::UserStake(staker.clone())).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalStaked, &total.checked_add(amount).ok_or(Error::CalculationOverflow)?);
        env.storage().instance().set(&DataKey::UserStake(staker.clone()), &user.checked_add(amount).ok_or(Error::CalculationOverflow)?);
        env.events().publish((Symbol::new(&env, "staked"), staker.clone()), amount);
        Ok(())
    }

    /// Withdraw staked tokens from the vault.
    pub fn withdraw(env: Env, staker: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 { return Err(Error::InvalidAmount); }
        staker.require_auth();
        Self::update_rewards(&env, Some(&staker))?;

        let user: i128 = env.storage().instance().get(&DataKey::UserStake(staker.clone())).unwrap_or(0);
        if user < amount { return Err(Error::InsufficientStake); }
        let total: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalStaked, &total.checked_sub(amount).ok_or(Error::CalculationOverflow)?);
        env.storage().instance().set(&DataKey::UserStake(staker.clone()), &user.checked_sub(amount).ok_or(Error::CalculationOverflow)?);

        let staking_token: Address = env.storage().instance().get(&DataKey::StakingToken).ok_or(Error::NotInitialized)?;
        TokenClient::new(&env, &staking_token).transfer(&env.current_contract_address(), &staker, &amount);
        env.events().publish((Symbol::new(&env, "withdrawn"), staker.clone()), amount);
        Ok(())
    }

    /// Claim accrued reward tokens.
    pub fn claim_reward(env: Env, staker: Address) -> Result<i128, Error> {
        staker.require_auth();
        Self::update_rewards(&env, Some(&staker))?;

        let reward_amount: i128 = env.storage().instance().get(&DataKey::UserRewardsEarned(staker.clone())).unwrap_or(0);
        if reward_amount <= 0 { return Ok(0); }

        let reserved: i128 = env.storage().instance().get(&DataKey::TotalRewardsReserved).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalRewardsReserved, &reserved.saturating_sub(reward_amount));
        env.storage().instance().set(&DataKey::UserRewardsEarned(staker.clone()), &0i128);

        let reward_token: Address = env.storage().instance().get(&DataKey::RewardToken).ok_or(Error::NotInitialized)?;
        TokenClient::new(&env, &reward_token).transfer(&env.current_contract_address(), &staker, &reward_amount);
        env.events().publish((Symbol::new(&env, "reward_paid"), staker.clone()), reward_amount);
        Ok(reward_amount)
    }

    /// Compound rewards back into stake (requires staking_token == reward_token).
    pub fn compound(env: Env, staker: Address) -> Result<i128, Error> {
        Self::require_not_paused(&env)?;
        staker.require_auth();

        let staking_token: Address = env.storage().instance().get(&DataKey::StakingToken).ok_or(Error::NotInitialized)?;
        let reward_token: Address = env.storage().instance().get(&DataKey::RewardToken).ok_or(Error::NotInitialized)?;
        if staking_token != reward_token { return Err(Error::Unauthorized); }

        Self::update_rewards(&env, Some(&staker))?;

        let reward_amount: i128 = env.storage().instance().get(&DataKey::UserRewardsEarned(staker.clone())).unwrap_or(0);
        if reward_amount <= 0 { return Ok(0); }

        let reserved: i128 = env.storage().instance().get(&DataKey::TotalRewardsReserved).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalRewardsReserved, &reserved.saturating_sub(reward_amount));
        env.storage().instance().set(&DataKey::UserRewardsEarned(staker.clone()), &0i128);

        let total: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        let user: i128 = env.storage().instance().get(&DataKey::UserStake(staker.clone())).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalStaked, &total.checked_add(reward_amount).ok_or(Error::CalculationOverflow)?);
        env.storage().instance().set(&DataKey::UserStake(staker.clone()), &user.checked_add(reward_amount).ok_or(Error::CalculationOverflow)?);
        env.events().publish((Symbol::new(&env, "compounded"), staker.clone()), reward_amount);
        Ok(reward_amount)
    }

    /// Emergency withdraw principal without reward calculation — only available when paused.
    pub fn emergency_withdraw(env: Env, staker: Address) -> Result<i128, Error> {
        Self::require_paused(&env)?;
        staker.require_auth();

        let user: i128 = env.storage().instance().get(&DataKey::UserStake(staker.clone())).unwrap_or(0);
        if user <= 0 { return Err(Error::InsufficientStake); }

        let total: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalStaked, &total.saturating_sub(user));
        env.storage().instance().set(&DataKey::UserStake(staker.clone()), &0i128);

        let staking_token: Address = env.storage().instance().get(&DataKey::StakingToken).ok_or(Error::NotInitialized)?;
        TokenClient::new(&env, &staking_token).transfer(&env.current_contract_address(), &staker, &user);
        env.events().publish((Symbol::new(&env, "emergency_exit"), staker.clone()), user);
        Ok(user)
    }

    /// Admin only: pause or unpause the contract.
    pub fn set_paused(env: Env, caller: Address, paused: bool) -> Result<(), Error> {
        Self::require_admin(&env, &caller)?;
        env.storage().instance().set(&DataKey::IsPaused, &paused);
        env.events().publish((Symbol::new(&env, "pause_changed"),), paused);
        Ok(())
    }

    // ---- View functions ----

    pub fn earned(env: Env, account: Address) -> i128 {
        let current_time = env.ledger().timestamp();
        let period_finish: u64 = env.storage().instance().get(&DataKey::PeriodFinish).unwrap_or(0);
        let last_update: u64 = env.storage().instance().get(&DataKey::LastUpdateTime).unwrap_or(0);
        let reward_rate: u128 = env.storage().instance().get(&DataKey::RewardRate).unwrap_or(0);
        let total_staked: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        let stored_rpt: u128 = env.storage().instance().get(&DataKey::RewardPerTokenStored).unwrap_or(0);
        let current_rpt = reward::compute_reward_per_token(
            current_time, last_update, period_finish, reward_rate, total_staked, stored_rpt,
        );
        let user_stake: i128 = env.storage().instance().get(&DataKey::UserStake(account.clone())).unwrap_or(0);
        let user_paid: u128 = env.storage().instance().get(&DataKey::UserRewardPerTokenPaid(account.clone())).unwrap_or(0);
        let user_stored: i128 = env.storage().instance().get(&DataKey::UserRewardsEarned(account)).unwrap_or(0);
        reward::compute_earned(user_stake, current_rpt, user_paid, user_stored)
    }

    pub fn reward_per_token(env: Env) -> u128 {
        let current_time = env.ledger().timestamp();
        let period_finish: u64 = env.storage().instance().get(&DataKey::PeriodFinish).unwrap_or(0);
        let last_update: u64 = env.storage().instance().get(&DataKey::LastUpdateTime).unwrap_or(0);
        let reward_rate: u128 = env.storage().instance().get(&DataKey::RewardRate).unwrap_or(0);
        let total_staked: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        let stored_rpt: u128 = env.storage().instance().get(&DataKey::RewardPerTokenStored).unwrap_or(0);
        reward::compute_reward_per_token(
            current_time, last_update, period_finish, reward_rate, total_staked, stored_rpt,
        )
    }

    pub fn get_total_staked(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0)
    }

    pub fn get_user_stake(env: Env, account: Address) -> i128 {
        env.storage().instance().get(&DataKey::UserStake(account)).unwrap_or(0)
    }

    pub fn get_pool_info(env: Env) -> (i128, u128, u64, u64, bool) {
        let total_staked: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        let reward_rate: u128 = env.storage().instance().get(&DataKey::RewardRate).unwrap_or(0);
        let period_finish: u64 = env.storage().instance().get(&DataKey::PeriodFinish).unwrap_or(0);
        let last_update: u64 = env.storage().instance().get(&DataKey::LastUpdateTime).unwrap_or(0);
        let paused: bool = env.storage().instance().get(&DataKey::IsPaused).unwrap_or(false);
        (total_staked, reward_rate, period_finish, last_update, paused)
    }

    // ---- Internal helpers ----

    fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(Error::NotInitialized)?;
        if caller != &admin { return Err(Error::Unauthorized); }
        caller.require_auth();
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), Error> {
        if env.storage().instance().get::<_, bool>(&DataKey::IsPaused).unwrap_or(false) {
            return Err(Error::ContractPaused);
        }
        Ok(())
    }

    fn require_paused(env: &Env) -> Result<(), Error> {
        if !env.storage().instance().get::<_, bool>(&DataKey::IsPaused).unwrap_or(false) {
            return Err(Error::ContractNotPaused);
        }
        Ok(())
    }

    fn update_rewards(env: &Env, staker: Option<&Address>) -> Result<(), Error> {
        let current_time = env.ledger().timestamp();
        let period_finish: u64 = env.storage().instance().get(&DataKey::PeriodFinish).unwrap_or(0);
        let last_update: u64 = env.storage().instance().get(&DataKey::LastUpdateTime).unwrap_or(0);
        let reward_rate: u128 = env.storage().instance().get(&DataKey::RewardRate).unwrap_or(0);
        let total_staked: i128 = env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0);
        let stored_rpt: u128 = env.storage().instance().get(&DataKey::RewardPerTokenStored).unwrap_or(0);

        let new_rpt = reward::compute_reward_per_token(
            current_time, last_update, period_finish, reward_rate, total_staked, stored_rpt,
        );
        env.storage().instance().set(&DataKey::RewardPerTokenStored, &new_rpt);
        env.storage().instance().set(
            &DataKey::LastUpdateTime,
            &reward::last_time_reward_applicable(current_time, period_finish),
        );

        if let Some(user) = staker {
            let user_stake: i128 = env.storage().instance().get(&DataKey::UserStake(user.clone())).unwrap_or(0);
            let user_paid: u128 = env.storage().instance().get(&DataKey::UserRewardPerTokenPaid(user.clone())).unwrap_or(0);
            let user_stored: i128 = env.storage().instance().get(&DataKey::UserRewardsEarned(user.clone())).unwrap_or(0);
            let new_earned = reward::compute_earned(user_stake, new_rpt, user_paid, user_stored);
            env.storage().instance().set(&DataKey::UserRewardsEarned(user.clone()), &new_earned);
            env.storage().instance().set(&DataKey::UserRewardPerTokenPaid(user.clone()), &new_rpt);
        }
        Ok(())
    }
}
