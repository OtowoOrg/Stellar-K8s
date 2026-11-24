#![cfg(test)]

use super::reward::*;

/// Pure-math unit tests for reward accounting (no Soroban SDK needed).
/// Rate values use realistic magnitudes — rate is in (reward_units * PRECISION / second / total_stake)
/// so we pick small rates that won't overflow u128.

const RATE: u128 = 1_000_000; // 1e6 reward-units per second (pre-precision: actual per-token accrual = RATE * PRECISION / stake)

#[test]
fn test_reward_per_token_zero_staked_returns_stored() {
    // When total_staked is 0, stored value should be returned unchanged.
    let stored = 1_000_000u128;
    let result = compute_reward_per_token(1000, 0, 2000, RATE, 0, stored);
    assert_eq!(result, stored, "should return stored when no stake");
}

#[test]
fn test_reward_per_token_past_period_finish_caps() {
    // After period ends, further time should not accrue rewards.
    let total = 1_000_000_000i128; // 1e9 tokens staked
    // Period is 0..1000. Query at t=2000 vs t=3000 — both should return same.
    let r1 = compute_reward_per_token(2000, 500, 1000, RATE, total, 0);
    let r2 = compute_reward_per_token(3000, 500, 1000, RATE, total, 0);
    assert_eq!(r1, r2, "rewards must not accrue past period_finish");
}

#[test]
fn test_reward_per_token_increases_with_time() {
    let total = 1_000_000i128;
    let r1 = compute_reward_per_token(500, 0, 10_000, RATE, total, 0);
    let r2 = compute_reward_per_token(1000, 0, 10_000, RATE, total, 0);
    assert!(r2 > r1, "reward_per_token must increase with time");
}

#[test]
fn test_compute_earned_proportional() {
    // Two stakers with equal stake and same duration should earn the same.
    let total = 2_000i128; // two stakers, 1000 each
    let rpt_at_t1000 = compute_reward_per_token(1000, 0, 10_000, RATE, total, 0);

    let earned_a = compute_earned(1000, rpt_at_t1000, 0, 0);
    let earned_b = compute_earned(1000, rpt_at_t1000, 0, 0);
    assert_eq!(earned_a, earned_b, "equal stakers must earn equal rewards");
}

#[test]
fn test_compute_earned_proportional_to_stake() {
    // Staker with 2x stake earns 2x rewards.
    let total = 3_000i128; // staker_a=1000, staker_b=2000
    let rpt = compute_reward_per_token(1000, 0, 10_000, RATE, total, 0);
    let earned_a = compute_earned(1000, rpt, 0, 0);
    let earned_b = compute_earned(2000, rpt, 0, 0);
    assert_eq!(earned_b, earned_a * 2, "rewards must be proportional to stake");
}

#[test]
fn test_last_time_reward_applicable_before_finish() {
    assert_eq!(last_time_reward_applicable(500, 1000), 500);
}

#[test]
fn test_last_time_reward_applicable_after_finish() {
    assert_eq!(last_time_reward_applicable(1500, 1000), 1000);
}

#[test]
fn test_compute_new_reward_rate_fresh_period() {
    // New period with no carry-over.
    let (rate, finish) = compute_new_reward_rate(0, 0, 0, 1_000_000, 1000);
    assert_eq!(rate, 1000u128);
    assert_eq!(finish, 1000u64);
}

#[test]
fn test_compute_new_reward_rate_rollover() {
    // 500s remain at rate=1000 -> leftover=500_000; add 1_000_000 -> 1_500_000 / 1000s = 1500
    let (rate, finish) = compute_new_reward_rate(0, 500, 1000, 1_000_000, 1000);
    assert_eq!(rate, 1500u128);
    assert_eq!(finish, 1000u64);
}

#[test]
fn test_total_pool_balance_solvency() {
    // Assert each individual staker's earnings never exceed total rewards emitted in their window.
    let rate = RATE;
    let duration_secs = 1_000u64;
    // total_emitted approx = rate * duration (in reward units, ignoring precision because it cancels)
    let total_emitted = (rate * duration_secs as u128) as i128;

    let scenarios: &[(u64, u64, i128)] = &[
        (0, 1000, 500),
        (200, 800, 1000),
        (500, 1000, 300),
    ];

    for &(enter, exit, stake) in scenarios {
        let rpt_start = compute_reward_per_token(enter, 0, duration_secs, rate, stake, 0);
        let rpt_end   = compute_reward_per_token(exit,  0, duration_secs, rate, stake, 0);
        let single    = compute_earned(stake, rpt_end, rpt_start, 0);
        assert!(
            single <= total_emitted,
            "staker earned {} but total emitted is {}",
            single,
            total_emitted
        );
    }
}

#[test]
fn test_zero_reward_for_zero_stake() {
    let rpt = compute_reward_per_token(1000, 0, 2000, RATE, 0, 5000);
    assert_eq!(rpt, 5000, "rpt unchanged when no stake");
    let earned = compute_earned(0, rpt, 0, 0);
    assert_eq!(earned, 0, "zero stake earns zero");
}

#[test]
fn test_stored_rewards_accumulate_correctly() {
    // Staker with already stored rewards gets more on next checkpoint.
    let already_earned: i128 = 500_000;
    // rpt_delta = REWARD_PRECISION -> pending = 1 * REWARD_PRECISION / REWARD_PRECISION = 1
    let rpt_delta = REWARD_PRECISION;
    let stake = 1i128;
    let earned = compute_earned(stake, rpt_delta, 0, already_earned);
    assert_eq!(earned, already_earned + 1);
}

#[test]
fn test_rounding_no_drift_many_checkpoints() {
    // Simulate 100 sequential checkpoints for a single staker to verify no cumulative drift.
    let rate = RATE;
    let total_stake = 1_000_000i128;
    let period_finish = 10_000u64;
    let step = 100u64; // checkpoint every 100s

    let mut rpt_stored = 0u128;
    let mut last_update = 0u64;

    for tick in 1..=100u64 {
        let current_time = tick * step;
        rpt_stored = compute_reward_per_token(
            current_time,
            last_update,
            period_finish,
            rate,
            total_stake,
            rpt_stored,
        );
        last_update = current_time.min(period_finish);
    }

    // Final rpt should equal a single direct computation from 0 to period_finish.
    let direct_rpt = compute_reward_per_token(10_000, 0, period_finish, rate, total_stake, 0);
    assert_eq!(
        rpt_stored, direct_rpt,
        "incremental checkpoints must produce zero drift vs single computation"
    );
}
