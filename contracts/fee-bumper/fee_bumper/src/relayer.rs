//! Per-account rate limiting for the fee-bump sponsorship contract.
//!
//! Kept separate from `lib.rs`'s contract surface per the issue's module
//! layout (mirroring `contracts/proxy-controller`'s `lib.rs`/`deployer.rs`
//! split). Deliberately `Env`-free: [`check_and_advance`] is a pure
//! function over plain values, so it's unit tested directly with
//! `cargo test` on the host target, no Soroban test harness required.

use soroban_sdk::contracttype;

/// A fixed-window rate limit: at most `max_per_window` sponsorships may be
/// authorized for a given account within any `window_seconds` period.
///
/// This is what bounds a relayer's worst-case exposure to a single
/// account: even if every sponsorship this account is ever authorized for
/// escrows the maximum estimated fee and is never settled or reclaimed,
/// the account cannot cause more than `max_per_window` such reservations
/// per window.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub max_per_window: u32,
    pub window_seconds: u64,
}

impl RateLimitConfig {
    pub fn is_valid(&self) -> bool {
        self.max_per_window > 0 && self.window_seconds > 0
    }
}

/// An account's rate-limit bookkeeping as of its last authorized
/// sponsorship.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountWindow {
    pub window_start: u64,
    pub count: u32,
}

/// Attempt to record one more sponsorship for an account at time `now`,
/// given its `previous` window state (`None` if it has never been
/// sponsored) and the current `config`.
///
/// Returns the window state to persist if the sponsorship is allowed, or
/// `None` if the account is currently rate-limited and the caller must
/// reject the request without reserving anything.
pub fn check_and_advance(
    config: &RateLimitConfig,
    previous: Option<AccountWindow>,
    now: u64,
) -> Option<AccountWindow> {
    match previous {
        None => Some(AccountWindow { window_start: now, count: 1 }),
        Some(w) if now.saturating_sub(w.window_start) >= config.window_seconds => {
            // The previous window has fully elapsed; start a fresh one
            // rather than carrying over any of its count.
            Some(AccountWindow { window_start: now, count: 1 })
        }
        Some(w) if w.count < config.max_per_window => {
            Some(AccountWindow { window_start: w.window_start, count: w.count + 1 })
        }
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(max_per_window: u32, window_seconds: u64) -> RateLimitConfig {
        RateLimitConfig { max_per_window, window_seconds }
    }

    #[test]
    fn first_sponsorship_for_a_new_account_starts_a_window() {
        let w = check_and_advance(&config(3, 60), None, 100).unwrap();
        assert_eq!(w, AccountWindow { window_start: 100, count: 1 });
    }

    #[test]
    fn stays_within_cap_increments_count_and_keeps_window_start() {
        let w1 = check_and_advance(&config(3, 60), None, 100).unwrap();
        let w2 = check_and_advance(&config(3, 60), Some(w1), 110).unwrap();
        assert_eq!(w2, AccountWindow { window_start: 100, count: 2 });
    }

    #[test]
    fn at_cap_within_window_is_rate_limited() {
        let cfg = config(2, 60);
        let w1 = check_and_advance(&cfg, None, 100).unwrap();
        let w2 = check_and_advance(&cfg, Some(w1), 110).unwrap();
        // Third attempt in the same window, still at/under window_seconds.
        let w3 = check_and_advance(&cfg, Some(w2), 120);
        assert_eq!(w3, None, "a third sponsorship within one window must be rejected");
    }

    #[test]
    fn window_boundary_is_inclusive_of_reset() {
        let cfg = config(1, 60);
        let w1 = check_and_advance(&cfg, None, 100).unwrap();
        // Exactly at the boundary: treated as elapsed, fresh window.
        let w2 = check_and_advance(&cfg, Some(w1), 160).unwrap();
        assert_eq!(w2, AccountWindow { window_start: 160, count: 1 });
    }

    #[test]
    fn one_tick_before_the_boundary_is_still_rate_limited() {
        let cfg = config(1, 60);
        let w1 = check_and_advance(&cfg, None, 100).unwrap();
        let w2 = check_and_advance(&cfg, Some(w1), 159);
        assert_eq!(w2, None);
    }

    #[test]
    fn elapsed_window_resets_count_even_if_previously_at_cap() {
        let cfg = config(1, 60);
        let w1 = check_and_advance(&cfg, None, 0).unwrap();
        assert_eq!(check_and_advance(&cfg, Some(w1), 30), None, "still limited mid-window");
        let w2 = check_and_advance(&cfg, Some(w1), 61).unwrap();
        assert_eq!(w2, AccountWindow { window_start: 61, count: 1 });
    }

    #[test]
    fn invalid_configs_are_rejected() {
        assert!(!config(0, 60).is_valid());
        assert!(!config(5, 0).is_valid());
        assert!(config(1, 1).is_valid());
    }
}
