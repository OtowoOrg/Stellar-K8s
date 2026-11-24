/// Scaling precision factor (1e18) for reward per token accounting to eliminate precision loss.
pub const REWARD_PRECISION: u128 = 1_000_000_000_000_000_000;

/// Returns the last applicable time (in seconds) for reward calculations.
/// If current timestamp is past period_finish, returns period_finish.
pub fn last_time_reward_applicable(current_time: u64, period_finish: u64) -> u64 {
    if current_time < period_finish {
        current_time
    } else {
        period_finish
    }
}

/// Computes the updated cumulative reward per token stored.
///
/// Formula:
/// stored_reward_per_token + ((last_applicable_time - last_update_time) * reward_rate * REWARD_PRECISION) / total_staked
pub fn compute_reward_per_token(
    current_time: u64,
    last_update_time: u64,
    period_finish: u64,
    reward_rate: u128,
    total_staked: i128,
    stored_reward_per_token: u128,
) -> u128 {
    if total_staked <= 0 {
        return stored_reward_per_token;
    }

    let last_applicable = last_time_reward_applicable(current_time, period_finish);
    if last_applicable <= last_update_time {
        return stored_reward_per_token;
    }

    let time_delta = (last_applicable - last_update_time) as u128;
    let reward_accrued = time_delta
        .checked_mul(reward_rate)
        .expect("reward rate overflow")
        .checked_mul(REWARD_PRECISION)
        .expect("reward precision overflow");

    let addition = reward_accrued / (total_staked as u128);
    stored_reward_per_token
        .checked_add(addition)
        .expect("reward per token overflow")
}

/// Computes the total pending rewards earned by a user.
///
/// Formula:
/// (user_staked_balance * (current_reward_per_token - user_reward_per_token_paid)) / REWARD_PRECISION + user_stored_rewards
pub fn compute_earned(
    user_staked_balance: i128,
    current_reward_per_token: u128,
    user_reward_per_token_paid: u128,
    user_stored_rewards: i128,
) -> i128 {
    if user_staked_balance <= 0 {
        return user_stored_rewards;
    }

    let reward_per_token_delta = current_reward_per_token.saturating_sub(user_reward_per_token_paid);
    let pending = ((user_staked_balance as u128)
        .checked_mul(reward_per_token_delta)
        .expect("user earned mul overflow")
        / REWARD_PRECISION) as i128;

    user_stored_rewards
        .checked_add(pending)
        .expect("user earned add overflow")
}

/// Computes the new reward rate and period finish when scheduling a new reward amount over a duration.
/// If the previous period is still active, remaining unallocated rewards are rolled over into the new period.
pub fn compute_new_reward_rate(
    current_time: u64,
    period_finish: u64,
    reward_rate: u128,
    reward_amount: i128,
    duration: u64,
) -> (u128, u64) {
    if duration == 0 {
        panic!("duration cannot be 0");
    }
    if reward_amount <= 0 {
        panic!("reward amount must be positive");
    }

    let duration_u128 = duration as u128;
    let new_rate = if current_time >= period_finish {
        (reward_amount as u128) / duration_u128
    } else {
        let remaining_seconds = (period_finish - current_time) as u128;
        let leftover = remaining_seconds
            .checked_mul(reward_rate)
            .expect("leftover overflow");
        let total_reward = leftover
            .checked_add(reward_amount as u128)
            .expect("total reward overflow");
        total_reward / duration_u128
    };

    if new_rate == 0 {
        panic!("reward rate too small for duration");
    }

    let new_period_finish = current_time
        .checked_add(duration)
        .expect("period finish overflow");

    (new_rate, new_period_finish)
}
