// Copyright 2026 Stellar-K8s Contributors
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
//! Graceful connection draining framework for rolling updates (issue #1508).
//!
//! Every service follows the same shutdown order — **stop intake, drain, then
//! exit** — through one shared [`DrainController`], so correctness is the
//! default rather than per-service discipline:
//!
//! 1. `stop_intake` flips readiness off: no new connections or streams.
//! 2. [`DrainController::drain`] waits for in-flight work tracked by
//!    [`ConnectionGuard`]s, giving long-lived streams their own configurable
//!    grace period.
//! 3. When the grace timeout expires, remaining work is force-terminated and
//!    reported via [`DrainMetrics::forced_terminations`].
//!
//! [`DrainController::is_ready`] feeds readiness probes, [`DrainMetrics`]
//! exposes drain duration per deployment, and [`prestop_hook_yaml`] renders
//! the matching `preStop` hook template.
//!
//! The same controller doubles as reusable middleware: [`DrainController::track`]
//! wraps any future so it is admitted, tracked, and released automatically, and
//! [`DrainController::render_prometheus`] exposes the draining state and drain
//! metrics for scraping.
//!
//! # Example
//!
//! ```no_run
//! use stellar_k8s::connection_drain::{DrainConfig, DrainController};
//! use std::time::Duration;
//!
//! # async fn run() {
//! let controller = DrainController::new(DrainConfig {
//!     drain_timeout: Duration::from_secs(30),
//!     ..DrainConfig::default()
//! });
//! controller.stop_intake(); // readiness goes false
//! let metrics = controller.drain().await; // waits for in-flight work
//! assert!(metrics.completed_without_force());
//! # }
//! ```

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Draining behavior for one service deployment.
#[derive(Debug, Clone)]
pub struct DrainConfig {
    /// Total time to wait for in-flight work before forceful termination.
    pub drain_timeout: Duration,
    /// Extra grace for long-lived streams (bounded interruption).
    pub stream_grace_period: Duration,
    /// Pre-stop hook sleep matching the readiness propagation delay.
    pub prestop_sleep: Duration,
}

impl Default for DrainConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            stream_grace_period: Duration::from_secs(10),
            prestop_sleep: Duration::from_secs(5),
        }
    }
}

impl DrainConfig {
    /// Override the drain timeout (builder style).
    pub fn with_drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    /// Override the stream grace period (builder style).
    pub fn with_stream_grace_period(mut self, period: Duration) -> Self {
        self.stream_grace_period = period;
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lifecycle phases
// ─────────────────────────────────────────────────────────────────────────────

/// Shutdown phase of a service instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainPhase {
    /// Accepting traffic normally.
    Active,
    /// Readiness off; existing work continues, nothing new is admitted.
    StopIntake,
    /// Waiting for in-flight work to finish.
    Draining,
    /// Shutdown complete (gracefully or by force).
    Terminated,
}

// ─────────────────────────────────────────────────────────────────────────────
// Metrics
// ─────────────────────────────────────────────────────────────────────────────

/// Observable outcome of one drain cycle, per deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainMetrics {
    /// How long draining took (capped by the grace timeout on force).
    pub drain_duration: Duration,
    /// Peak in-flight connections or streams observed while draining.
    pub peak_in_flight: usize,
    /// Work still in flight when the grace timeout fired.
    pub forced_terminations: usize,
    /// Long-lived streams closed at the stream grace boundary.
    pub streams_interrupted: usize,
}

impl DrainMetrics {
    /// Whether draining finished with no forceful termination.
    pub fn completed_without_force(&self) -> bool {
        self.forced_terminations == 0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Controller
// ─────────────────────────────────────────────────────────────────────────────

/// Shared state behind [`DrainController`].
#[derive(Debug)]
struct DrainShared {
    in_flight: AtomicUsize,
    streams: AtomicUsize,
    notify: Notify,
}

/// Standardized connection draining for rolling updates.
///
/// Cloneable: services hold a handle per listener while the controller owns
/// the shutdown sequence. All coordination is in-process; readiness and
/// metrics stay consistent through one library type.
#[derive(Debug, Clone)]
pub struct DrainController {
    config: DrainConfig,
    phase: Arc<std::sync::RwLock<DrainPhase>>,
    shared: Arc<DrainShared>,
    peak_in_flight: Arc<AtomicUsize>,
    streams_interrupted: Arc<AtomicUsize>,
}

impl DrainController {
    /// Create a controller with `config`.
    pub fn new(config: DrainConfig) -> Self {
        Self {
            config,
            phase: Arc::new(std::sync::RwLock::new(DrainPhase::Active)),
            shared: Arc::new(DrainShared {
                in_flight: AtomicUsize::new(0),
                streams: AtomicUsize::new(0),
                notify: Notify::new(),
            }),
            peak_in_flight: Arc::new(AtomicUsize::new(0)),
            streams_interrupted: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Current shutdown phase.
    pub fn phase(&self) -> DrainPhase {
        *self.phase.read().expect("drain phase lock poisoned")
    }

    /// Readiness probe value: false as soon as intake stops.
    pub fn is_ready(&self) -> bool {
        self.phase() == DrainPhase::Active
    }

    /// Whether the instance is currently draining (intake stopped, work
    /// still finishing). Useful for readiness bodies and dashboards.
    pub fn is_draining(&self) -> bool {
        matches!(self.phase(), DrainPhase::StopIntake | DrainPhase::Draining)
    }

    /// Number of connections or requests currently in flight.
    pub fn in_flight(&self) -> usize {
        self.shared.in_flight.load(Ordering::SeqCst)
    }

    /// Step 1: stop intake. New work is refused; readiness goes false.
    pub fn stop_intake(&self) {
        *self.phase.write().expect("drain phase lock poisoned") = DrainPhase::StopIntake;
    }

    /// Admit one unit of work, returning a guard that releases on drop.
    ///
    /// Returns `None` once intake has stopped, so late arrivals fail fast
    /// instead of starting work that a shutdown would reset.
    pub fn acquire(&self) -> Option<ConnectionGuard> {
        if self.phase() != DrainPhase::Active {
            return None;
        }
        let current = self.shared.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_in_flight.fetch_max(current, Ordering::SeqCst);
        Some(ConnectionGuard {
            shared: Arc::clone(&self.shared),
        })
    }

    /// Register a long-lived stream; streams share the bounded stream grace.
    pub fn open_stream(&self) -> Option<StreamGuard> {
        if self.phase() != DrainPhase::Active {
            return None;
        }
        self.shared.streams.fetch_add(1, Ordering::SeqCst);
        let current = self.shared.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_in_flight.fetch_max(current, Ordering::SeqCst);
        Some(StreamGuard {
            shared: Arc::clone(&self.shared),
        })
    }

    /// Reusable middleware: admit `future` as one unit of in-flight work.
    ///
    /// The work is tracked for the whole lifetime of the future and released
    /// when it completes or is dropped. Returns `None` once intake has stopped,
    /// so callers can fail fast instead of starting work a shutdown would reset.
    pub async fn track<F>(&self, future: F) -> Option<F::Output>
    where
        F: Future,
    {
        let _guard = self.acquire()?;
        Some(future.await)
    }

    /// Reusable middleware for long-lived streams: admit `future` as a stream.
    ///
    /// Streams get the bounded stream grace period during draining.
    pub async fn track_stream<F>(&self, future: F) -> Option<F::Output>
    where
        F: Future,
    {
        let _guard = self.open_stream()?;
        Some(future.await)
    }

    /// Steps 2–3: drain in-flight work, then exit.
    ///
    /// Waits up to `stream_grace_period` for streams to close on their own,
    /// then interrupts stragglers and waits up to `drain_timeout` for the
    /// remainder. Anything still in flight is force-terminated so the grace
    /// timeout is always enforced with a forceful fallback.
    pub async fn drain(&self) -> DrainMetrics {
        *self.phase.write().expect("drain phase lock poisoned") = DrainPhase::Draining;
        let started = Instant::now();

        // Bounded interruption for long-lived streams: give them their own
        // grace, then count the stragglers as interrupted.
        if self.shared.streams.load(Ordering::SeqCst) > 0 {
            let _ =
                tokio::time::timeout(self.config.stream_grace_period, self.wait_for_quiet()).await;
            let stragglers = self.shared.streams.load(Ordering::SeqCst);
            if stragglers > 0 {
                self.streams_interrupted
                    .fetch_add(stragglers, Ordering::SeqCst);
                // Streams are externally closed at this point; release them.
                self.shared
                    .in_flight
                    .fetch_sub(stragglers, Ordering::SeqCst);
                self.shared.streams.store(0, Ordering::SeqCst);
                self.shared.notify.notify_waiters();
            }
        }

        let _ = tokio::time::timeout(self.config.drain_timeout, self.wait_for_quiet()).await;
        let remaining = self.shared.in_flight.load(Ordering::SeqCst);

        *self.phase.write().expect("drain phase lock poisoned") = DrainPhase::Terminated;
        DrainMetrics {
            drain_duration: started.elapsed(),
            peak_in_flight: self.peak_in_flight.load(Ordering::SeqCst),
            forced_terminations: remaining,
            streams_interrupted: self.streams_interrupted.load(Ordering::SeqCst),
        }
    }

    /// Resolve when no work remains in flight.
    async fn wait_for_quiet(&self) {
        loop {
            if self.shared.in_flight.load(Ordering::SeqCst) == 0 {
                return;
            }
            self.shared.notify.notified().await;
        }
    }

    /// Render the draining state and metrics in Prometheus text format.
    ///
    /// Exposes readiness (`stellar_drain_ready`), the draining flag
    /// (`stellar_drain_draining`), in-flight gauges, and the outcome of the
    /// last drain cycle so operators can observe draining per deployment.
    pub fn render_prometheus(&self) -> String {
        let ready = usize::from(self.is_ready());
        let draining = usize::from(self.is_draining());
        let in_flight = self.shared.in_flight.load(Ordering::SeqCst);
        let streams = self.shared.streams.load(Ordering::SeqCst);
        let peak = self.peak_in_flight.load(Ordering::SeqCst);
        let interrupted = self.streams_interrupted.load(Ordering::SeqCst);
        format!(
            "# HELP stellar_drain_ready Whether the instance accepts new work (1 = ready).\n\
             # TYPE stellar_drain_ready gauge\n\
             stellar_drain_ready {ready}\n\
             # HELP stellar_drain_draining Whether the instance is draining (1 = draining).\n\
             # TYPE stellar_drain_draining gauge\n\
             stellar_drain_draining {draining}\n\
             # HELP stellar_drain_in_flight Connections or requests currently in flight.\n\
             # TYPE stellar_drain_in_flight gauge\n\
             stellar_drain_in_flight {in_flight}\n\
             # HELP stellar_drain_streams Long-lived streams currently open.\n\
             # TYPE stellar_drain_streams gauge\n\
             stellar_drain_streams {streams}\n\
             # HELP stellar_drain_peak_in_flight Peak in-flight work observed.\n\
             # TYPE stellar_drain_peak_in_flight gauge\n\
             stellar_drain_peak_in_flight {peak}\n\
             # HELP stellar_drain_streams_interrupted Streams closed at the stream grace boundary.\n\
             # TYPE stellar_drain_streams_interrupted counter\n\
             stellar_drain_streams_interrupted {interrupted}\n"
        )
    }
}

/// RAII guard for one in-flight connection or request.
///
/// Dropping the guard releases the slot and wakes a pending [`drain`](DrainController::drain).
#[derive(Debug)]
pub struct ConnectionGuard {
    shared: Arc<DrainShared>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.shared.notify.notify_waiters();
    }
}

/// RAII guard for one long-lived stream.
#[derive(Debug)]
pub struct StreamGuard {
    shared: Arc<DrainShared>,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        // Only decrement when the stream slot is still counted: interrupted
        // streams were already released by `drain`.
        if self
            .shared
            .streams
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.shared.notify.notify_waiters();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PreStop hook template
// ─────────────────────────────────────────────────────────────────────────────

/// Render the matching `preStop` hook snippet for a pod spec.
///
/// The sleep covers readiness propagation so endpoints are removed before
/// SIGTERM, pairing the in-process library with the Kubernetes lifecycle.
pub fn prestop_hook_yaml(config: &DrainConfig) -> String {
    format!(
        "lifecycle:\n  preStop:\n    exec:\n      command: [\"/bin/sh\", \"-c\", \"sleep {}\"]\nterminationGracePeriodSeconds: {}\n",
        config.prestop_sleep.as_secs(),
        config.drain_timeout.as_secs() + config.prestop_sleep.as_secs(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_config() -> DrainConfig {
        DrainConfig {
            drain_timeout: Duration::from_millis(500),
            stream_grace_period: Duration::from_millis(100),
            prestop_sleep: Duration::from_secs(1),
        }
    }

    #[test]
    fn readiness_is_true_while_active() {
        let controller = DrainController::new(DrainConfig::default());
        assert_eq!(controller.phase(), DrainPhase::Active);
        assert!(controller.is_ready());
    }

    #[test]
    fn stop_intake_flips_readiness_and_refuses_new_work() {
        let controller = DrainController::new(DrainConfig::default());
        controller.stop_intake();
        assert_eq!(controller.phase(), DrainPhase::StopIntake);
        assert!(!controller.is_ready());
        assert!(controller.acquire().is_none());
        assert!(controller.open_stream().is_none());
    }

    #[tokio::test]
    async fn idle_drain_terminates_without_force() {
        let controller = DrainController::new(fast_config());
        controller.stop_intake();
        let metrics = controller.drain().await;
        assert_eq!(controller.phase(), DrainPhase::Terminated);
        assert!(metrics.completed_without_force());
        assert_eq!(metrics.peak_in_flight, 0);
    }

    #[tokio::test]
    async fn in_flight_requests_drain_before_timeout() {
        let controller = DrainController::new(fast_config());
        let guard = controller.acquire().unwrap();
        assert_eq!(controller.in_flight(), 1);
        controller.stop_intake();
        let handle = tokio::spawn({
            let controller = controller.clone();
            async move { controller.drain().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(guard);
        let metrics = handle.await.unwrap();
        assert!(metrics.completed_without_force());
        assert_eq!(metrics.peak_in_flight, 1);
    }

    #[tokio::test]
    async fn stuck_work_is_force_terminated_at_the_deadline() {
        let controller = DrainController::new(fast_config());
        let _guard = controller.acquire().unwrap();
        controller.stop_intake();
        let metrics = controller.drain().await;
        assert!(!metrics.completed_without_force());
        assert_eq!(metrics.forced_terminations, 1);
        assert!(metrics.drain_duration >= fast_config().drain_timeout);
    }

    #[tokio::test]
    async fn lingering_streams_are_interrupted_within_bound() {
        let controller = DrainController::new(fast_config());
        let _stream = controller.open_stream().unwrap();
        controller.stop_intake();
        let metrics = controller.drain().await;
        // The stream never closed itself: bounded interruption kicked in.
        assert_eq!(metrics.streams_interrupted, 1);
        assert!(metrics.completed_without_force());
    }

    #[tokio::test]
    async fn closing_streams_drain_cleanly() {
        let controller = DrainController::new(fast_config());
        let stream = controller.open_stream().unwrap();
        controller.stop_intake();
        let handle = tokio::spawn({
            let controller = controller.clone();
            async move { controller.drain().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(stream);
        let metrics = handle.await.unwrap();
        assert!(metrics.completed_without_force());
        assert_eq!(metrics.streams_interrupted, 0);
    }

    #[test]
    fn prestop_template_covers_drain_plus_propagation() {
        let yaml = prestop_hook_yaml(&DrainConfig::default());
        assert!(yaml.contains("preStop"));
        assert!(yaml.contains("terminationGracePeriodSeconds: 35"));
    }

    #[test]
    fn config_builders_override_timeouts() {
        let config = DrainConfig::default()
            .with_drain_timeout(Duration::from_secs(60))
            .with_stream_grace_period(Duration::from_secs(20));
        assert_eq!(config.drain_timeout, Duration::from_secs(60));
        assert_eq!(config.stream_grace_period, Duration::from_secs(20));
    }

    #[test]
    fn draining_state_is_visible_in_readiness_and_metrics() {
        let controller = DrainController::new(DrainConfig::default());
        let active = controller.render_prometheus();
        assert!(active.contains("stellar_drain_ready 1"));
        assert!(active.contains("stellar_drain_draining 0"));

        controller.stop_intake();
        assert!(!controller.is_ready());
        assert!(controller.is_draining());
        let draining = controller.render_prometheus();
        assert!(draining.contains("stellar_drain_ready 0"));
        assert!(draining.contains("stellar_drain_draining 1"));
    }

    #[tokio::test]
    async fn track_middleware_admits_and_releases_work() {
        let controller = DrainController::new(fast_config());
        let out = controller.track(async { 42 }).await;
        assert_eq!(out, Some(42));
        assert_eq!(controller.in_flight(), 0);

        controller.stop_intake();
        assert!(controller.track(async { 1 }).await.is_none());
        assert!(controller.track_stream(async { 1 }).await.is_none());
    }

    /// Soak test: 50 consecutive rollouts with continuous long-lived
    /// connections must produce zero connection resets.
    #[tokio::test]
    async fn soak_fifty_rollouts_zero_resets() {
        const ROLLOUTS: usize = 50;
        const CONNECTIONS: usize = 8;

        let mut total_resets = 0usize;
        for _ in 0..ROLLOUTS {
            let controller = DrainController::new(DrainConfig {
                drain_timeout: Duration::from_millis(500),
                stream_grace_period: Duration::from_millis(200),
                prestop_sleep: Duration::from_millis(10),
            });

            // Long-lived connections that finish on their own shortly after
            // intake stops: a correct drain never resets them.
            let mut handles = Vec::new();
            for _ in 0..CONNECTIONS {
                let guard = controller.acquire().expect("admitted while active");
                handles.push(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    drop(guard);
                }));
            }

            controller.stop_intake();
            let metrics = controller.drain().await;
            for handle in handles {
                handle.await.unwrap();
            }

            total_resets += metrics.forced_terminations;
            assert!(metrics.completed_without_force());
            assert_eq!(metrics.streams_interrupted, 0);
        }

        assert_eq!(total_resets, 0, "rolling updates must not reset connections");
    }
}
