//! Automatic retry policy tuning based on failure telemetry
//!
//! Tracks per-operation failure rates over a sliding window and dynamically
//! adjusts base delay, max delay, and max attempt count so that flapping
//! operations back off aggressively while stable operations recover quickly.
//!
//! # Design
//!
//! Each [`RetryPolicyTuner`] maintains a ring-buffer of [`FailureRecord`]s
//! for every operation key. On each call to [`RetryPolicyTuner::record`] the
//! tuner recomputes the [`RetryPolicy`] for that key and stores it.
//! Callers fetch the current policy via [`RetryPolicyTuner::policy_for`].
//!
//! Tuning heuristics:
//! - failure rate > 80 % → max delay × 2, max attempts + 2
//! - failure rate 50–80 % → max delay × 1.5
//! - failure rate < 10 % (after ≥ MIN_SAMPLES) → relax toward defaults
//! - failure rate == 0 % for a full window → reset to defaults

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Minimum number of samples before the tuner adjusts away from defaults.
const MIN_SAMPLES: usize = 5;

/// Size of the sliding window (number of recent outcomes tracked per key).
const WINDOW_SIZE: usize = 20;

/// Default base delay between retry attempts.
pub const DEFAULT_BASE_DELAY_SECS: u64 = 15;

/// Default maximum delay cap (5 minutes).
pub const DEFAULT_MAX_DELAY_SECS: u64 = 300;

/// Default maximum number of retry attempts before giving up.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// Upper bound on the tuned max delay (30 minutes).
const MAX_TUNED_DELAY_SECS: u64 = 1_800;

/// Upper bound on the tuned max attempt count.
const MAX_TUNED_ATTEMPTS: u32 = 12;

/// A single outcome observation recorded in the sliding window.
#[derive(Debug, Clone)]
struct FailureRecord {
    /// Wall-clock time of the observation (used for TTL pruning if needed).
    _observed_at: Instant,
    /// True if the operation failed on this attempt.
    failed: bool,
    /// Optional error category for per-class tuning in the future.
    error_class: Option<ErrorClass>,
}

/// Coarse error categories that influence tuning differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorClass {
    /// Transient network or API-server error (503, timeout).
    Transient,
    /// Rate-limiting response (429).
    RateLimit,
    /// Permanent configuration or validation error (400, 422).
    Permanent,
    /// Resource conflict (409 Conflict).
    Conflict,
}

/// The retry policy produced by the tuner for a given operation key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Base delay for exponential back-off: `base * 2^attempt`.
    pub base_delay: Duration,
    /// Cap applied after exponential growth.
    pub max_delay: Duration,
    /// Maximum number of attempts before the operation is considered failed.
    pub max_attempts: u32,
    /// Number of samples used to derive this policy.
    pub sample_count: usize,
    /// Failure rate (0–100) from the current window.
    pub failure_rate_pct: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(DEFAULT_BASE_DELAY_SECS),
            max_delay: Duration::from_secs(DEFAULT_MAX_DELAY_SECS),
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            sample_count: 0,
            failure_rate_pct: 0,
        }
    }
}

impl RetryPolicy {
    /// Compute the delay for the given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base = self.base_delay.as_secs();
        let secs = base.saturating_mul(2_u64.saturating_pow(attempt.min(10)));
        Duration::from_secs(secs.min(self.max_delay.as_secs()))
    }

    /// Returns true if `attempt` (1-indexed) exceeds the allowed maximum.
    pub fn is_exhausted(&self, attempt: u32) -> bool {
        attempt >= self.max_attempts
    }
}

/// Per-key sliding-window state.
#[derive(Debug, Default)]
struct KeyState {
    /// Ring buffer of recent outcomes (oldest first).
    window: Vec<FailureRecord>,
    /// Cached policy recomputed after each record call.
    policy: RetryPolicy,
}

impl KeyState {
    fn record(&mut self, failed: bool, error_class: Option<ErrorClass>) {
        if self.window.len() >= WINDOW_SIZE {
            self.window.remove(0);
        }
        self.window.push(FailureRecord {
            _observed_at: Instant::now(),
            failed,
            error_class,
        });
        self.policy = self.compute_policy();
    }

    fn compute_policy(&self) -> RetryPolicy {
        let n = self.window.len();
        if n < MIN_SAMPLES {
            return RetryPolicy {
                sample_count: n,
                ..Default::default()
            };
        }

        let failures = self.window.iter().filter(|r| r.failed).count();
        let failure_rate_pct = ((failures * 100) / n) as u8;

        // Check whether rate-limiting errors dominate – if so, apply gentler
        // back-off to avoid thundering-herd against the API server.
        let rate_limit_heavy = self
            .window
            .iter()
            .filter(|r| r.failed && r.error_class == Some(ErrorClass::RateLimit))
            .count()
            > n / 3;

        let (max_delay_secs, max_attempts) = if failure_rate_pct == 0 {
            // Fully healthy: reset to defaults.
            (DEFAULT_MAX_DELAY_SECS, DEFAULT_MAX_ATTEMPTS)
        } else if failure_rate_pct >= 80 {
            // Heavily failing: aggressive back-off.
            let delay = (DEFAULT_MAX_DELAY_SECS * 2).min(MAX_TUNED_DELAY_SECS);
            let attempts = (DEFAULT_MAX_ATTEMPTS + 2).min(MAX_TUNED_ATTEMPTS);
            (delay, attempts)
        } else if failure_rate_pct >= 50 {
            // Moderately failing: moderate back-off.
            let delay = (DEFAULT_MAX_DELAY_SECS * 3 / 2).min(MAX_TUNED_DELAY_SECS);
            (delay, DEFAULT_MAX_ATTEMPTS + 1)
        } else if failure_rate_pct < 10 {
            // Mostly healthy: relax toward defaults.
            (DEFAULT_MAX_DELAY_SECS, DEFAULT_MAX_ATTEMPTS)
        } else {
            // 10–49 %: light adjustment.
            (DEFAULT_MAX_DELAY_SECS, DEFAULT_MAX_ATTEMPTS)
        };

        // Rate-limit-heavy windows get a slightly higher base delay to space
        // out retries more gently.
        let base_delay_secs = if rate_limit_heavy { 30 } else { DEFAULT_BASE_DELAY_SECS };

        RetryPolicy {
            base_delay: Duration::from_secs(base_delay_secs),
            max_delay: Duration::from_secs(max_delay_secs),
            max_attempts,
            sample_count: n,
            failure_rate_pct,
        }
    }
}

/// Thread-safe, shared retry policy tuner.
///
/// Intended to be held in [`ControllerState`] and shared across reconcile loops.
#[derive(Clone, Debug)]
pub struct RetryPolicyTuner {
    inner: Arc<Mutex<HashMap<String, KeyState>>>,
}

impl Default for RetryPolicyTuner {
    fn default() -> Self {
        Self::new()
    }
}

impl RetryPolicyTuner {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record an outcome for `operation_key`.
    ///
    /// Call this after every attempt regardless of success or failure so the
    /// tuner can track both the failure rate and the success recovery.
    pub fn record(&self, operation_key: &str, failed: bool, error_class: Option<ErrorClass>) {
        let mut map = self.inner.lock().expect("RetryPolicyTuner lock poisoned");
        let state = map.entry(operation_key.to_string()).or_default();
        state.record(failed, error_class);

        let p = &state.policy;
        if failed {
            debug!(
                operation = operation_key,
                failure_rate_pct = p.failure_rate_pct,
                max_delay_secs = p.max_delay.as_secs(),
                max_attempts = p.max_attempts,
                "retry policy updated after failure"
            );
        }

        if state.window.len() >= MIN_SAMPLES && p.failure_rate_pct == 0 {
            info!(
                operation = operation_key,
                sample_count = p.sample_count,
                "operation fully recovered — retry policy reset to defaults"
            );
        }
    }

    /// Returns the current [`RetryPolicy`] for `operation_key`.
    ///
    /// If no samples have been recorded yet, returns the default policy.
    pub fn policy_for(&self, operation_key: &str) -> RetryPolicy {
        let map = self.inner.lock().expect("RetryPolicyTuner lock poisoned");
        map.get(operation_key)
            .map(|s| s.policy.clone())
            .unwrap_or_default()
    }

    /// Returns a snapshot of all tracked operation keys and their policies.
    pub fn all_policies(&self) -> HashMap<String, RetryPolicy> {
        let map = self.inner.lock().expect("RetryPolicyTuner lock poisoned");
        map.iter()
            .map(|(k, s)| (k.clone(), s.policy.clone()))
            .collect()
    }

    /// Clears state for a single key (e.g., after a node is deleted).
    pub fn clear(&self, operation_key: &str) {
        let mut map = self.inner.lock().expect("RetryPolicyTuner lock poisoned");
        map.remove(operation_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuner() -> RetryPolicyTuner {
        RetryPolicyTuner::new()
    }

    #[test]
    fn default_policy_before_samples() {
        let t = tuner();
        let p = t.policy_for("op-a");
        assert_eq!(p.base_delay, Duration::from_secs(DEFAULT_BASE_DELAY_SECS));
        assert_eq!(p.max_delay, Duration::from_secs(DEFAULT_MAX_DELAY_SECS));
        assert_eq!(p.max_attempts, DEFAULT_MAX_ATTEMPTS);
        assert_eq!(p.sample_count, 0);
    }

    #[test]
    fn under_min_samples_returns_defaults() {
        let t = tuner();
        for _ in 0..MIN_SAMPLES - 1 {
            t.record("op", true, None);
        }
        let p = t.policy_for("op");
        assert_eq!(p.max_delay, Duration::from_secs(DEFAULT_MAX_DELAY_SECS));
        assert_eq!(p.max_attempts, DEFAULT_MAX_ATTEMPTS);
        assert_eq!(p.sample_count, MIN_SAMPLES - 1);
    }

    #[test]
    fn all_failures_above_80pct_increases_delay_and_attempts() {
        let t = tuner();
        // Record 10 failures (100 % failure rate, well above MIN_SAMPLES).
        for _ in 0..10 {
            t.record("op-fail", true, None);
        }
        let p = t.policy_for("op-fail");
        assert!(
            p.max_delay.as_secs() > DEFAULT_MAX_DELAY_SECS,
            "max_delay should increase on high failure rate"
        );
        assert!(
            p.max_attempts > DEFAULT_MAX_ATTEMPTS,
            "max_attempts should increase on high failure rate"
        );
        assert_eq!(p.failure_rate_pct, 100);
    }

    #[test]
    fn all_successes_resets_to_defaults() {
        let t = tuner();
        // First drive the policy up.
        for _ in 0..10 {
            t.record("op-recover", true, None);
        }
        // Then fill the window with successes.
        for _ in 0..WINDOW_SIZE {
            t.record("op-recover", false, None);
        }
        let p = t.policy_for("op-recover");
        assert_eq!(p.failure_rate_pct, 0);
        assert_eq!(p.max_delay, Duration::from_secs(DEFAULT_MAX_DELAY_SECS));
        assert_eq!(p.max_attempts, DEFAULT_MAX_ATTEMPTS);
    }

    #[test]
    fn rate_limit_errors_increase_base_delay() {
        let t = tuner();
        // More than 1/3 of the window = rate-limit errors.
        for i in 0..10 {
            let class = if i < 4 {
                Some(ErrorClass::RateLimit)
            } else {
                None
            };
            t.record("op-rl", true, class);
        }
        let p = t.policy_for("op-rl");
        assert!(
            p.base_delay.as_secs() > DEFAULT_BASE_DELAY_SECS,
            "base_delay should increase when rate-limit errors dominate"
        );
    }

    #[test]
    fn delay_for_attempt_caps_at_max_delay() {
        let p = RetryPolicy::default();
        // attempt 100 should still cap at max_delay.
        assert_eq!(p.delay_for_attempt(100), p.max_delay);
    }

    #[test]
    fn delay_for_attempt_grows_exponentially() {
        let p = RetryPolicy::default(); // base = 15 s
        assert_eq!(p.delay_for_attempt(0), Duration::from_secs(15));
        assert_eq!(p.delay_for_attempt(1), Duration::from_secs(30));
        assert_eq!(p.delay_for_attempt(2), Duration::from_secs(60));
        assert_eq!(p.delay_for_attempt(3), Duration::from_secs(120));
    }

    #[test]
    fn is_exhausted_checks_max_attempts() {
        let p = RetryPolicy::default(); // max_attempts = 5
        assert!(!p.is_exhausted(4));
        assert!(p.is_exhausted(5));
        assert!(p.is_exhausted(6));
    }

    #[test]
    fn clear_removes_key() {
        let t = tuner();
        for _ in 0..10 {
            t.record("op-del", true, None);
        }
        assert_ne!(t.policy_for("op-del").sample_count, 0);
        t.clear("op-del");
        assert_eq!(t.policy_for("op-del").sample_count, 0);
    }

    #[test]
    fn all_policies_snapshot() {
        let t = tuner();
        t.record("alpha", false, None);
        t.record("beta", true, None);
        let snap = t.all_policies();
        assert!(snap.contains_key("alpha"));
        assert!(snap.contains_key("beta"));
    }

    #[test]
    fn sliding_window_evicts_old_samples() {
        let t = tuner();
        // Fill window with failures.
        for _ in 0..WINDOW_SIZE {
            t.record("op-slide", true, None);
        }
        // Then overwrite with successes — old failures should be evicted.
        for _ in 0..WINDOW_SIZE {
            t.record("op-slide", false, None);
        }
        let p = t.policy_for("op-slide");
        assert_eq!(p.failure_rate_pct, 0, "all old failures should be evicted");
    }

    #[test]
    fn moderate_failure_rate_adjusts_max_delay() {
        let t = tuner();
        // 6 failures out of 10 = 60 % (50–80 % band).
        for i in 0..10 {
            t.record("op-mod", i < 6, None);
        }
        let p = t.policy_for("op-mod");
        assert!(p.failure_rate_pct >= 50 && p.failure_rate_pct < 80);
        assert!(
            p.max_delay.as_secs() > DEFAULT_MAX_DELAY_SECS,
            "moderate failure rate should still increase max delay"
        );
    }
}
