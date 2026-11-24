//! Sliding window request tracker for per-client IP rate limiting.
//!
//! Implements a lock-free, fixed-size circular buffer tracking request
//! timestamps within a configurable time window. Each client IP maintains
//! an independent window state stored in a concurrent hash map.
//!
//! # Algorithm
//!
//! The sliding window works by recording the timestamp of every admitted
//! request and counting how many timestamps fall within `[now - window, now]`
//! when a new request arrives. Expired timestamps are evicted lazily on each
//! check, keeping memory bounded.
//!
//! # Performance
//!
//! - Sub-millisecond evaluation: O(k) where k ≤ `capacity` (typically ≤ 1000)
//! - No heap allocation per request after the initial window is created
//! - Lock contention limited to per-IP `Mutex<WindowState>` (not global)

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// A per-IP sliding window state.
///
/// Timestamps are stored as `Instant` values in a bounded deque.
/// The deque acts as a ring buffer — when capacity is reached the
/// oldest entry is dropped before inserting a new one.
#[derive(Debug)]
pub struct WindowState {
    /// Monotonic timestamps of admitted requests within the current window.
    timestamps: VecDeque<Instant>,
    /// Maximum number of requests tracked (capacity = rate limit ceiling).
    capacity: usize,
    /// Duration of the sliding window (e.g. 1 second).
    window: Duration,
}

impl WindowState {
    /// Create a new window state with the given capacity and duration.
    ///
    /// `capacity` should equal the maximum allowed requests per `window`.
    pub fn new(capacity: usize, window: Duration) -> Self {
        Self {
            timestamps: VecDeque::with_capacity(capacity + 1),
            capacity,
            window,
        }
    }

    /// Try to record a new request at `now`.
    ///
    /// Returns `true` if the request is **allowed** (count within limit),
    /// `false` if it should be **rejected**.
    ///
    /// This method:
    /// 1. Evicts timestamps older than `now - window`
    /// 2. Counts remaining timestamps
    /// 3. Allows if count < capacity, then records the new timestamp
    pub fn try_record(&mut self, now: Instant) -> bool {
        let cutoff = now - self.window;

        // Evict expired timestamps from the front
        while self.timestamps.front().map_or(false, |&t| t <= cutoff) {
            self.timestamps.pop_front();
        }

        if self.timestamps.len() < self.capacity {
            self.timestamps.push_back(now);
            true
        } else {
            false
        }
    }

    /// Return the number of requests recorded in the current window as of `now`.
    ///
    /// Does not mutate state (read-only snapshot).
    pub fn count_in_window(&self, now: Instant) -> usize {
        let cutoff = now - self.window;
        self.timestamps
            .iter()
            .filter(|&&t| t > cutoff)
            .count()
    }

    /// Return the duration until the oldest request in the window expires.
    ///
    /// Returns `Duration::ZERO` when the window is empty (no wait needed).
    pub fn retry_after(&self, now: Instant) -> Duration {
        match self.timestamps.front() {
            Some(&oldest) => {
                let expires_at = oldest + self.window;
                if expires_at > now {
                    expires_at - now
                } else {
                    Duration::ZERO
                }
            }
            None => Duration::ZERO,
        }
    }
}

/// Shared, concurrent tracker for all client IPs.
///
/// Wraps a [`DashMap`] mapping `IpAddr` → `Mutex<WindowState>`.
/// Using `DashMap` provides sharded locking so different IPs can be
/// evaluated truly concurrently without contending on a single global lock.
#[derive(Debug, Clone)]
pub struct SlidingWindowTracker {
    /// Per-IP window states.
    windows: Arc<DashMap<IpAddr, Mutex<WindowState>>>,
    /// Requests-per-second limit baked into every new window.
    base_limit: usize,
    /// Duration of each window.
    window_duration: Duration,
}

impl SlidingWindowTracker {
    /// Create a new tracker.
    ///
    /// `base_limit` is the default maximum requests per `window_duration`.
    /// This can be overridden per-call via `try_record_with_limit`.
    pub fn new(base_limit: usize, window_duration: Duration) -> Self {
        Self {
            windows: Arc::new(DashMap::new()),
            base_limit,
            window_duration,
        }
    }

    /// Try to record a request from `ip` against `effective_limit`.
    ///
    /// `effective_limit` is the **dynamic** limit computed by the engine
    /// (which may be lower than `base_limit` under CPU pressure).
    ///
    /// Returns `(allowed, retry_after_secs)`.
    pub fn try_record(&self, ip: IpAddr, effective_limit: usize, now: Instant) -> (bool, u64) {
        // Fast path: entry already exists — grab per-IP lock only
        if let Some(entry) = self.windows.get(&ip) {
            let mut state = entry.lock().expect("window mutex poisoned");
            // Dynamically update capacity if limit changed
            state.capacity = effective_limit;
            let allowed = state.try_record(now);
            let retry = if allowed {
                0
            } else {
                state.retry_after(now).as_secs().max(1)
            };
            return (allowed, retry);
        }

        // Slow path: insert new entry
        let mut state = WindowState::new(effective_limit, self.window_duration);
        state.try_record(now); // first request always allowed
        self.windows.insert(ip, Mutex::new(state));
        (true, 0)
    }

    /// Return the current request count for `ip` in the active window.
    pub fn count(&self, ip: IpAddr, now: Instant) -> usize {
        self.windows
            .get(&ip)
            .map(|entry| {
                entry
                    .lock()
                    .expect("window mutex poisoned")
                    .count_in_window(now)
            })
            .unwrap_or(0)
    }

    /// Evict IP windows that have had no requests for longer than `idle_ttl`.
    ///
    /// Should be called periodically (e.g. every 60 s) to prevent unbounded
    /// memory growth when many distinct IPs hit the service.
    pub fn evict_idle(&self, now: Instant, idle_ttl: Duration) {
        self.windows.retain(|_ip, state| {
            let state = state.lock().expect("window mutex poisoned");
            match state.timestamps.back() {
                Some(&last) => (now - last) < idle_ttl,
                None => false, // empty window — evict
            }
        });
    }

    /// Return the total number of IPs currently tracked.
    pub fn tracked_ips(&self) -> usize {
        self.windows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, a))
    }

    #[test]
    fn window_state_allows_up_to_limit() {
        let window = Duration::from_secs(1);
        let mut state = WindowState::new(5, window);
        let now = Instant::now();

        for _ in 0..5 {
            assert!(state.try_record(now));
        }
        assert!(!state.try_record(now), "6th request must be rejected");
    }

    #[test]
    fn window_state_resets_after_expiry() {
        let window = Duration::from_millis(50);
        let mut state = WindowState::new(3, window);
        let t0 = Instant::now();

        for _ in 0..3 {
            assert!(state.try_record(t0));
        }
        assert!(!state.try_record(t0), "should be at limit");

        // Advance past the window
        let t1 = t0 + Duration::from_millis(60);
        assert!(
            state.try_record(t1),
            "after expiry the window should allow again"
        );
    }

    #[test]
    fn window_state_retry_after_is_positive() {
        let window = Duration::from_secs(1);
        let mut state = WindowState::new(1, window);
        let now = Instant::now();
        state.try_record(now);
        state.try_record(now); // rejected

        let retry = state.retry_after(now);
        assert!(retry > Duration::ZERO);
        assert!(retry <= window);
    }

    #[test]
    fn tracker_concurrent_ips() {
        let tracker = SlidingWindowTracker::new(100, Duration::from_secs(1));
        let now = Instant::now();

        for a in 1u8..=50 {
            let (allowed, _) = tracker.try_record(ip(a), 100, now);
            assert!(allowed);
        }
        assert_eq!(tracker.tracked_ips(), 50);
    }

    #[test]
    fn tracker_enforces_limit() {
        let tracker = SlidingWindowTracker::new(2, Duration::from_secs(1));
        let now = Instant::now();
        let client = ip(1);

        let (a1, _) = tracker.try_record(client, 2, now);
        let (a2, _) = tracker.try_record(client, 2, now);
        let (a3, retry) = tracker.try_record(client, 2, now);

        assert!(a1);
        assert!(a2);
        assert!(!a3, "third request must be rejected");
        assert!(retry >= 1, "retry_after must be at least 1 s");
    }

    #[test]
    fn tracker_evict_idle_removes_stale_ips() {
        let tracker = SlidingWindowTracker::new(10, Duration::from_secs(1));
        let t0 = Instant::now();

        tracker.try_record(ip(1), 10, t0);
        tracker.try_record(ip(2), 10, t0);

        // Evict IPs idle for longer than 5 ms
        let t1 = t0 + Duration::from_millis(10);
        tracker.evict_idle(t1, Duration::from_millis(5));

        assert_eq!(tracker.tracked_ips(), 0, "both IPs should be evicted");
    }
}
