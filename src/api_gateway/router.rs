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
//! Intelligent request routing based on path, method, headers, and version.

use crate::api_gateway::config::{RouteConfig, VersioningConfig};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The result of matching a request against the route table.
#[derive(Debug, Clone)]
pub struct RouteMatch<'a> {
    pub route: &'a RouteConfig,
    /// Remaining path after stripping the matched prefix
    pub remaining_path: String,
}

/// Tracks in-flight requests (including long-lived streams) and the draining
/// state so rolling updates never reset connections.
#[derive(Debug)]
pub struct DrainState {
    draining: AtomicBool,
    in_flight: AtomicU64,
    streams: AtomicU64,
    started_at: std::sync::Mutex<Option<Instant>>,
    grace: Duration,
}

impl DrainState {
    pub fn new(grace: Duration) -> Self {
        Self {
            draining: AtomicBool::new(false),
            in_flight: AtomicU64::new(0),
            streams: AtomicU64::new(0),
            started_at: std::sync::Mutex::new(None),
            grace,
        }
    }

    /// Begin draining: readiness flips to false, new work is rejected.
    pub fn begin_drain(&self) {
        let mut started = self.started_at.lock().unwrap();
        if started.is_none() {
            *started = Some(Instant::now());
        }
        self.draining.store(true, Ordering::SeqCst);
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    /// Readiness reflects draining: not ready once draining starts.
    pub fn is_ready(&self) -> bool {
        !self.is_draining()
    }

    pub fn in_flight(&self) -> u64 {
        self.in_flight.load(Ordering::SeqCst)
    }

    pub fn active_streams(&self) -> u64 {
        self.streams.load(Ordering::SeqCst)
    }

    /// Register an in-flight request. Returns a guard that decrements on drop.
    pub fn track(&self) -> DrainGuard<'_> {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        DrainGuard { state: self, stream: false }
    }

    /// Register a long-lived stream. Streams survive the update with bounded
    /// interruption: they are allowed to finish within the grace window.
    pub fn track_stream(&self) -> DrainGuard<'_> {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        self.streams.fetch_add(1, Ordering::SeqCst);
        DrainGuard { state: self, stream: true }
    }

    /// Remaining grace time before forceful termination fallback.
    pub fn remaining_grace(&self) -> Duration {
        let started = *self.started_at.lock().unwrap();
        match started {
            Some(t) => self.grace.saturating_sub(t.elapsed()),
            None => self.grace,
        }
    }

    /// Whether the grace timeout has elapsed and forceful termination is due.
    pub fn grace_expired(&self) -> bool {
        self.is_draining() && self.remaining_grace().is_zero()
    }

    /// Enforce the grace timeout: returns `true` when all work has drained,
    /// `false` when the grace window expired and forceful termination is needed.
    pub fn wait_for_drain(&self) -> bool {
        if self.in_flight() == 0 {
            return true;
        }
        !self.grace_expired()
    }
}

/// RAII guard decrementing in-flight counters when a request/stream completes.
pub struct DrainGuard<'a> {
    state: &'a DrainState,
    stream: bool,
}

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        if self.stream {
            self.state.streams.fetch_sub(1, Ordering::SeqCst);
        }
        self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Route table built from [`RouteConfig`] entries.
pub struct Router {
    routes: Vec<RouteConfig>,
    versioning: VersioningConfig,
    drain: Arc<DrainState>,
}

impl Router {
    pub fn new(routes: Vec<RouteConfig>, versioning: VersioningConfig) -> Self {
        Self {
            routes,
            versioning,
            drain: Arc::new(DrainState::new(Duration::from_secs(30))),
        }
    }

    /// Build a router with an explicit drain grace period.
    pub fn with_drain(routes: Vec<RouteConfig>, versioning: VersioningConfig, grace: Duration) -> Self {
        Self {
            routes,
            versioning,
            drain: Arc::new(DrainState::new(grace)),
        }
    }

    pub fn drain_state(&self) -> Arc<DrainState> {
        Arc::clone(&self.drain)
    }

    /// Readiness probe: false while draining.
    pub fn is_ready(&self) -> bool {
        self.drain.is_ready()
    }

    /// Begin graceful shutdown of the router.
    pub fn begin_drain(&self) {
        self.drain.begin_drain();
    }

    /// Match an incoming request to a route.
    ///
    /// Matching priority:
    /// 1. Longest path prefix wins
    /// 2. Method must be in the allowed list (or list is empty = all)
    /// 3. All header predicates must match
    /// 4. Sunset versions return `None` (caller should respond 410)
    pub fn match_route<'a>(
        &'a self,
        path: &str,
        method: &str,
        headers: &HashMap<String, String>,
    ) -> MatchResult<'a> {
        // While draining, reject new work so in-flight requests can finish.
        if self.drain.is_draining() {
            return MatchResult::Draining;
        }

        // Check if the path starts with a sunset version prefix
        for sv in &self.versioning.sunset_versions {
            if path.contains(&format!("/{sv}/")) || path.ends_with(&format!("/{sv}")) {
                return MatchResult::Sunset(sv.clone());
            }
        }

        let mut best: Option<&RouteConfig> = None;
        let mut best_len = 0usize;

        for route in &self.routes {
            if !path.starts_with(&route.path_prefix) {
                continue;
            }
            if !route.methods.is_empty()
                && !route.methods.iter().any(|m| m.eq_ignore_ascii_case(method))
            {
                continue;
            }
            // Header predicates
            if !route
                .header_predicates
                .iter()
                .all(|(k, v)| headers.get(k.as_str()).map(|hv| hv == v).unwrap_or(false))
            {
                continue;
            }
            if route.path_prefix.len() > best_len {
                best_len = route.path_prefix.len();
                best = Some(route);
            }
        }

        match best {
            None => MatchResult::NotFound,
            Some(route) => {
                let remaining = path[route.path_prefix.len()..].to_string();
                MatchResult::Matched(RouteMatch {
                    route,
                    remaining_path: remaining,
                })
            }
        }
    }
}

pub enum MatchResult<'a> {
    Matched(RouteMatch<'a>),
    NotFound,
    /// Version has been sunset — respond 410 Gone
    Sunset(String),
    /// Router is draining — respond 503 with Retry-After
    Draining,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_gateway::config::{Protocol, VersioningConfig};

    fn make_route(prefix: &str, version: &str) -> RouteConfig {
        RouteConfig {
            id: prefix.into(),
            path_prefix: prefix.into(),
            methods: vec![],
            upstream: "http://upstream".into(),
            protocol: Protocol::Rest,
            version: version.into(),
            header_predicates: HashMap::new(),
            deprecated: false,
            sunset_date: None,
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let router = Router::new(
            vec![
                make_route("/api/v1", "v1"),
                make_route("/api/v1/transactions", "v1"),
            ],
            VersioningConfig::default(),
        );
        let m = router.match_route("/api/v1/transactions/abc", "GET", &HashMap::new());
        match m {
            MatchResult::Matched(r) => assert_eq!(r.route.path_prefix, "/api/v1/transactions"),
            _ => panic!("expected match"),
        }
    }

    #[test]
    fn no_match_returns_not_found() {
        let router = Router::new(vec![], VersioningConfig::default());
        assert!(matches!(
            router.match_route("/unknown", "GET", &HashMap::new()),
            MatchResult::NotFound
        ));
    }

    #[test]
    fn draining_rejects_new_requests_and_flips_readiness() {
        let router = Router::new(vec![make_route("/api", "v1")], VersioningConfig::default());
        assert!(router.is_ready());
        router.begin_drain();
        assert!(!router.is_ready());
        assert!(matches!(
            router.match_route("/api/x", "GET", &HashMap::new()),
            MatchResult::Draining
        ));
    }

    #[test]
    fn in_flight_and_streams_are_tracked() {
        let state = DrainState::new(Duration::from_secs(5));
        let g = state.track();
        let s = state.track_stream();
        assert_eq!(state.in_flight(), 2);
        assert_eq!(state.active_streams(), 1);
        drop(g);
        drop(s);
        assert_eq!(state.in_flight(), 0);
        assert_eq!(state.active_streams(), 0);
    }

    #[test]
    fn grace_timeout_enforced() {
        let state = DrainState::new(Duration::from_millis(0));
        let _g = state.track();
        state.begin_drain();
        assert!(state.grace_expired());
        assert!(!state.wait_for_drain());
    }
}
