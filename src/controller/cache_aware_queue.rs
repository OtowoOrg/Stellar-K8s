// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Cache-aware controller queue backoff and prioritization
//!
//! Standard exponential backoff treats every reconcile key identically: the
//! only input is the retry count. In practice, keys backed by warm/valid
//! cache entries can safely wait longer before the next reconcile (the cached
//! view is still trustworthy), while keys whose cache is cold or thrashing
//! need to be revisited sooner so the cache gets refreshed and stays useful.
//! On top of that, not all keys are equally urgent: a node with repeated
//! failures should jump the queue ahead of a steady-state node that merely
//! missed cache once.
//!
//! This module provides two independent pieces that compose:
//!
//! - [`calculate_cache_aware_backoff`] — folds cache hit rate and priority
//!   into the classic `base * 2^retry` exponential backoff calculation.
//! - [`CacheAwarePriorityQueue`] — a delay queue that only releases a key
//!   once its backoff has elapsed, and among ready keys always releases the
//!   highest-priority one first.
//!
//! # Design
//!
//! The queue keeps two heaps:
//! - `delayed`: a min-heap ordered by `ready_at`, holding keys that are still
//!   waiting out their backoff.
//! - `ready`: a max-heap ordered by `(priority, insertion order)`, holding
//!   keys whose backoff has elapsed and are waiting to be popped.
//!
//! Every call to [`CacheAwarePriorityQueue::pop_ready`] first promotes any
//! `delayed` entries whose `ready_at` has passed into `ready`, then pops the
//! highest-priority entry from `ready`.
//!
//! Rescheduling a key that is already queued does not create a duplicate:
//! each key carries a monotonically increasing epoch, and stale copies of a
//! superseded schedule are discarded lazily when they reach the front of a
//! heap.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Priority tier for a queued reconcile request.
///
/// Ordering is significant: variants declared later sort higher, so
/// `Critical > High > Normal > Low` when compared or placed in a max-heap.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum ReconcilePriority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

/// Base delay for cache-aware exponential back-off.
pub const DEFAULT_BASE_BACKOFF_SECS: u64 = 10;

/// Upper bound applied after all multipliers.
pub const DEFAULT_MAX_BACKOFF_SECS: u64 = 600;

/// Lower bound applied after all multipliers, so a `Critical` item under a
/// high cache-miss rate never collapses to a zero-delay busy loop.
const MIN_BACKOFF_SECS: f64 = 2.0;

/// Inputs to [`calculate_cache_aware_backoff`].
#[derive(Debug, Clone, Copy)]
pub struct CacheAwareBackoffInput {
    /// Number of consecutive retries so far (0-indexed).
    pub retry_count: u32,
    /// Cache hit rate observed for this key's data, in `[0.0, 1.0]`.
    /// `1.0` means the cache is fully warm and trustworthy; `0.0` means
    /// every lookup missed and the cached view is not to be relied upon.
    pub cache_hit_rate: f64,
    /// Priority tier for this key.
    pub priority: ReconcilePriority,
}

/// Compute a cache-aware exponential back-off duration.
///
/// The classic `base * 2^retry` term is scaled by two independent factors:
///
/// - **Cache factor** — a high hit rate stretches the delay (the cached
///   state is still valid, so there is no rush to reconcile again); a low
///   hit rate compresses it (the cache is cold/thrashing and needs a fresh
///   reconcile sooner to repopulate it). Ranges from `0.5x` (hit rate 0.0)
///   to `1.5x` (hit rate 1.0).
/// - **Priority factor** — `Critical` keys are serviced far sooner than
///   `Low` ones regardless of cache state.
///
/// The result is always clamped to `[MIN_BACKOFF_SECS, DEFAULT_MAX_BACKOFF_SECS]`.
pub fn calculate_cache_aware_backoff(input: CacheAwareBackoffInput) -> Duration {
    let hit_rate = input.cache_hit_rate.clamp(0.0, 1.0);
    let base = DEFAULT_BASE_BACKOFF_SECS as f64;
    let exp = base * 2f64.powi(input.retry_count.min(10) as i32);

    let cache_factor = 0.5 + hit_rate;
    let priority_factor = match input.priority {
        ReconcilePriority::Critical => 0.25,
        ReconcilePriority::High => 0.5,
        ReconcilePriority::Normal => 1.0,
        ReconcilePriority::Low => 1.75,
    };

    let secs = (exp * cache_factor * priority_factor)
        .clamp(MIN_BACKOFF_SECS, DEFAULT_MAX_BACKOFF_SECS as f64);
    Duration::from_secs_f64(secs)
}

/// Derive a [`ReconcilePriority`] from cache health and recent failure
/// history.
///
/// Keys with a stale cache (low hit rate) or recent failures are promoted
/// ahead of steady-state keys, so a large backlog of healthy nodes cannot
/// starve the few nodes that actually need attention.
pub fn priority_from_signals(cache_hit_rate: f64, consecutive_failures: u32) -> ReconcilePriority {
    if consecutive_failures >= 3 {
        return ReconcilePriority::Critical;
    }
    if consecutive_failures > 0 || cache_hit_rate < 0.3 {
        return ReconcilePriority::High;
    }
    if cache_hit_rate < 0.7 {
        return ReconcilePriority::Normal;
    }
    ReconcilePriority::Low
}

/// An entry waiting out its back-off in the delay heap.
#[derive(Debug, Clone)]
struct DelayedItem {
    key: String,
    priority: ReconcilePriority,
    ready_at: Instant,
    epoch: u64,
}

// Min-heap on `ready_at`: earliest ready time sorts as "greatest" so
// `BinaryHeap` (a max-heap) pops it first.
impl PartialEq for DelayedItem {
    fn eq(&self, other: &Self) -> bool {
        self.ready_at == other.ready_at
    }
}
impl Eq for DelayedItem {}
impl PartialOrd for DelayedItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DelayedItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.ready_at.cmp(&self.ready_at)
    }
}

/// An entry that has cleared its back-off and is waiting to be popped.
#[derive(Debug, Clone)]
struct ReadyItem {
    key: String,
    priority: ReconcilePriority,
    epoch: u64,
    /// Monotonically increasing insertion sequence, used to break priority
    /// ties in FIFO order.
    seq: u64,
}

// Max-heap on `(priority, older seq first)`.
impl PartialEq for ReadyItem {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}
impl Eq for ReadyItem {}
impl PartialOrd for ReadyItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ReadyItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

#[derive(Debug, Default)]
struct QueueState {
    delayed: BinaryHeap<DelayedItem>,
    ready: BinaryHeap<ReadyItem>,
    /// Latest epoch scheduled per key; entries in either heap whose epoch
    /// does not match are stale and are discarded lazily.
    epochs: HashMap<String, u64>,
    next_seq: u64,
}

/// A thread-safe delay queue that releases keys only after their cache-aware
/// back-off elapses, always releasing the highest-priority ready key first.
///
/// Intended to be held in `ControllerState` and shared across reconcile
/// loops, the same way [`super::retry_policy_tuner::RetryPolicyTuner`] is.
#[derive(Debug)]
pub struct CacheAwarePriorityQueue {
    state: Mutex<QueueState>,
}

impl Default for CacheAwarePriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheAwarePriorityQueue {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
        }
    }

    /// Schedule (or reschedule) `key` to become ready for reconciliation
    /// after `delay`, at the given `priority`.
    ///
    /// If `key` is already queued (delayed or ready), the previous schedule
    /// is superseded — only the most recent call's delay and priority take
    /// effect once it surfaces.
    pub fn schedule(&self, key: impl Into<String>, priority: ReconcilePriority, delay: Duration) {
        let key = key.into();
        let mut state = self.state.lock().expect("queue lock poisoned");
        let epoch = {
            let e = state.epochs.entry(key.clone()).or_insert(0);
            *e += 1;
            *e
        };
        state.delayed.push(DelayedItem {
            key,
            priority,
            ready_at: Instant::now() + delay,
            epoch,
        });
    }

    /// Convenience wrapper that derives both the back-off delay and the
    /// priority from cache/failure signals, then schedules the key.
    pub fn schedule_with_signals(
        &self,
        key: impl Into<String>,
        retry_count: u32,
        cache_hit_rate: f64,
        consecutive_failures: u32,
    ) {
        let priority = priority_from_signals(cache_hit_rate, consecutive_failures);
        let delay = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count,
            cache_hit_rate,
            priority,
        });
        self.schedule(key, priority, delay);
    }

    /// Move any delayed entries whose back-off has elapsed into the ready
    /// heap. Stale (superseded) entries are dropped rather than promoted.
    fn promote_ready(state: &mut QueueState) {
        let now = Instant::now();
        while let Some(top) = state.delayed.peek() {
            if top.ready_at > now {
                break;
            }
            let item = state.delayed.pop().expect("peeked item must exist");
            if state.epochs.get(&item.key) != Some(&item.epoch) {
                continue; // superseded by a later schedule() call
            }
            let seq = state.next_seq;
            state.next_seq += 1;
            state.ready.push(ReadyItem {
                key: item.key,
                priority: item.priority,
                epoch: item.epoch,
                seq,
            });
        }
    }

    /// Pop the highest-priority key whose back-off has elapsed, if any.
    ///
    /// Returns `None` if the queue is empty or every remaining entry is
    /// still waiting out its back-off.
    pub fn pop_ready(&self) -> Option<String> {
        let mut state = self.state.lock().expect("queue lock poisoned");
        Self::promote_ready(&mut state);

        while let Some(top) = state.ready.pop() {
            if state.epochs.get(&top.key) == Some(&top.epoch) {
                return Some(top.key);
            }
            // Superseded by a later schedule() call; keep looking.
        }
        None
    }

    /// Duration until the next delayed entry becomes ready, if the ready
    /// heap is currently empty. Useful for callers that want to sleep
    /// instead of busy-polling [`Self::pop_ready`].
    pub fn next_ready_in(&self) -> Option<Duration> {
        let state = self.state.lock().expect("queue lock poisoned");
        if !state.ready.is_empty() {
            return Some(Duration::ZERO);
        }
        state
            .delayed
            .peek()
            .map(|top| top.ready_at.saturating_duration_since(Instant::now()))
    }

    /// Total number of entries across both heaps, including any
    /// not-yet-collected stale duplicates.
    pub fn len(&self) -> usize {
        let state = self.state.lock().expect("queue lock poisoned");
        state.delayed.len() + state.ready.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove all state for `key` (e.g. after the resource is deleted).
    pub fn forget(&self, key: &str) {
        let mut state = self.state.lock().expect("queue lock poisoned");
        state.epochs.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially_with_retry_count() {
        let mk = |retry_count| {
            calculate_cache_aware_backoff(CacheAwareBackoffInput {
                retry_count,
                cache_hit_rate: 0.5,
                priority: ReconcilePriority::Normal,
            })
        };
        assert!(mk(1) > mk(0));
        assert!(mk(2) > mk(1));
    }

    #[test]
    fn warm_cache_extends_delay_vs_cold_cache() {
        let warm = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count: 2,
            cache_hit_rate: 1.0,
            priority: ReconcilePriority::Normal,
        });
        let cold = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count: 2,
            cache_hit_rate: 0.0,
            priority: ReconcilePriority::Normal,
        });
        assert!(
            cold < warm,
            "cold cache should be revisited sooner than warm cache"
        );
    }

    #[test]
    fn critical_priority_shrinks_delay_vs_low_priority() {
        let critical = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count: 2,
            cache_hit_rate: 0.5,
            priority: ReconcilePriority::Critical,
        });
        let low = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count: 2,
            cache_hit_rate: 0.5,
            priority: ReconcilePriority::Low,
        });
        assert!(critical < low);
    }

    #[test]
    fn backoff_is_clamped_to_bounds() {
        let d = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count: 100,
            cache_hit_rate: 0.0,
            priority: ReconcilePriority::Low,
        });
        assert!(d.as_secs() <= DEFAULT_MAX_BACKOFF_SECS);

        let d = calculate_cache_aware_backoff(CacheAwareBackoffInput {
            retry_count: 0,
            cache_hit_rate: 0.0,
            priority: ReconcilePriority::Critical,
        });
        assert_eq!(d.as_secs_f64(), MIN_BACKOFF_SECS);
    }

    #[test]
    fn priority_from_signals_escalates_on_failures() {
        assert_eq!(priority_from_signals(0.9, 0), ReconcilePriority::Low);
        assert_eq!(priority_from_signals(0.5, 0), ReconcilePriority::Normal);
        assert_eq!(priority_from_signals(0.2, 0), ReconcilePriority::High);
        assert_eq!(priority_from_signals(0.9, 1), ReconcilePriority::High);
        assert_eq!(priority_from_signals(0.9, 3), ReconcilePriority::Critical);
    }

    #[test]
    fn pop_ready_returns_none_before_backoff_elapses() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("a", ReconcilePriority::Normal, Duration::from_secs(60));
        assert_eq!(q.pop_ready(), None);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn pop_ready_returns_key_once_ready() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("a", ReconcilePriority::Normal, Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(q.pop_ready(), Some("a".to_string()));
        assert_eq!(q.pop_ready(), None);
        assert!(q.is_empty());
    }

    #[test]
    fn higher_priority_pops_first_among_ready_keys() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("low", ReconcilePriority::Low, Duration::from_millis(1));
        q.schedule(
            "critical",
            ReconcilePriority::Critical,
            Duration::from_millis(1),
        );
        q.schedule(
            "normal",
            ReconcilePriority::Normal,
            Duration::from_millis(1),
        );
        std::thread::sleep(Duration::from_millis(15));

        assert_eq!(q.pop_ready(), Some("critical".to_string()));
        assert_eq!(q.pop_ready(), Some("normal".to_string()));
        assert_eq!(q.pop_ready(), Some("low".to_string()));
    }

    #[test]
    fn fifo_order_breaks_ties_within_same_priority() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("first", ReconcilePriority::Normal, Duration::from_millis(1));
        q.schedule(
            "second",
            ReconcilePriority::Normal,
            Duration::from_millis(1),
        );
        std::thread::sleep(Duration::from_millis(15));

        assert_eq!(q.pop_ready(), Some("first".to_string()));
        assert_eq!(q.pop_ready(), Some("second".to_string()));
    }

    #[test]
    fn rescheduling_a_key_supersedes_the_previous_entry() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("a", ReconcilePriority::Low, Duration::from_millis(1));
        // Immediately reschedule with higher priority and a longer delay.
        q.schedule("a", ReconcilePriority::Critical, Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(15));

        // The stale Low/1ms schedule must not surface early.
        assert_eq!(q.pop_ready(), None);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn forget_drops_future_deliveries_for_a_key() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("a", ReconcilePriority::Normal, Duration::from_millis(1));
        q.forget("a");
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(q.pop_ready(), None);
    }

    #[test]
    fn next_ready_in_reports_remaining_delay_then_zero() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule("a", ReconcilePriority::Normal, Duration::from_millis(200));

        let remaining = q.next_ready_in().expect("entry is queued");
        assert!(remaining > Duration::ZERO && remaining <= Duration::from_millis(200));

        std::thread::sleep(Duration::from_millis(210));
        assert_eq!(q.next_ready_in(), Some(Duration::ZERO));
    }

    #[test]
    fn schedule_with_signals_assigns_higher_priority_to_failing_keys() {
        let q = CacheAwarePriorityQueue::new();
        q.schedule_with_signals("healthy", 0, 0.95, 0);
        q.schedule_with_signals("failing", 0, 0.1, 4);

        assert_eq!(q.len(), 2);
        // Failing key must clear its (shorter) back-off strictly before the
        // healthy key clears its (longer) one.
        assert!(q.next_ready_in().unwrap() < Duration::from_secs(DEFAULT_BASE_BACKOFF_SECS));
    }
}
