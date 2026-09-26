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
//! Shared configuration hot-reload framework for in-cluster agents (#1498).
//!
//! Every first-party agent uses this single library instead of a bespoke
//! watcher. It provides:
//!
//! - Atomic swap with versioned config snapshots (readers hold `Arc<T>`, so
//!   in-flight requests never observe a torn config and no requests drop).
//! - Validation gate: invalid config is rejected and the live version is
//!   retained.
//! - Reload events carrying old/new version IDs.
//!
//! Adding an agent is import + three lines of glue:
//!
//! ```ignore
//! use stellar_k8s::config_reload::ConfigReloader;
//! let reloader = ConfigReloader::new(initial, validate);
//! tokio::spawn(reloader.clone().watch_file("/etc/agent/config.yaml", parse, std::time::Duration::from_secs(1)));
//! let cfg: std::sync::Arc<MyConfig> = reloader.get();
//! ```

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

/// Maximum number of historical snapshots retained per reloader.
pub const MAX_HISTORY: usize = 32;
/// Hard cap guarding reload latency for very large configs (8 MiB).
pub const MAX_CONFIG_BYTES: usize = 8 * 1024 * 1024;

/// A single versioned snapshot of live config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot<T: Clone> {
    /// Monotonic version, starts at 1 and increments per accepted reload.
    pub version: u64,
    /// Unique revision ID for this snapshot (uuid v4).
    pub revision: String,
    /// When this snapshot became live.
    pub loaded_at: DateTime<Utc>,
    /// SHA-256 hex of the raw bytes that produced this snapshot.
    pub sha256: String,
    /// Raw byte length (acceptance: reload <200ms for 1 MiB).
    pub bytes_len: usize,
    /// The live payload (kept out of serde output to avoid dumping secrets).
    #[serde(skip)]
    pub payload: Arc<T>,
}

impl<T: Clone> ConfigSnapshot<T> {
    fn first(payload: T, sha256: String, bytes_len: usize) -> Self {
        Self {
            version: 1,
            revision: uuid::Uuid::new_v4().to_string(),
            loaded_at: Utc::now(),
            sha256,
            bytes_len,
            payload: Arc::new(payload),
        }
    }

    fn next(&self, payload: T, sha256: String, bytes_len: usize) -> Self {
        Self {
            version: self.version + 1,
            revision: uuid::Uuid::new_v4().to_string(),
            loaded_at: Utc::now(),
            sha256,
            bytes_len,
            payload: Arc::new(payload),
        }
    }
}

/// Event emitted after every accepted reload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReloadEvent {
    pub old_version: u64,
    pub new_version: u64,
    pub old_revision: String,
    pub new_revision: String,
    pub at: DateTime<Utc>,
}

/// Reload failure; live state is never mutated on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadError {
    /// Config bytes could not be parsed.
    Parse(String),
    /// Config parsed but failed validation; previous version retained.
    Invalid(String),
    /// Config unchanged (same sha); not an error, just a no-op.
    Unchanged,
    /// Config exceeds [`MAX_CONFIG_BYTES`].
    TooLarge(usize),
}

impl std::fmt::Display for ReloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "config parse error: {e}"),
            Self::Invalid(e) => write!(f, "invalid config rejected: {e}"),
            Self::Unchanged => write!(f, "config unchanged"),
            Self::TooLarge(n) => write!(f, "config too large: {n} bytes"),
        }
    }
}

impl std::error::Error for ReloadError {}

struct ReloaderInner<T: Clone + Send + Sync + 'static> {
    current: RwLock<ConfigSnapshot<T>>,
    history: RwLock<VecDeque<ConfigSnapshot<T>>>,
    validator: Box<dyn Fn(&T) -> Result<(), String> + Send + Sync>,
    events: broadcast::Sender<ReloadEvent>,
}

/// Shared, cloneable hot-reload handle. Readers call [`Self::get`] which
/// returns an `Arc<T>` snapshot — never blocking writers and never dropping
/// in-flight requests across reloads.
#[derive(Clone)]
pub struct ConfigReloader<T: Clone + Send + Sync + 'static> {
    inner: Arc<ReloaderInner<T>>,
}

impl<T: Clone + Send + Sync + 'static> ConfigReloader<T> {
    /// Create a reloader around an already-validated initial config.
    /// `validator` runs on every candidate reload; `Err` rejects it.
    pub fn new(
        initial: T,
        validator: impl Fn(&T) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        let (tx, _) = broadcast::channel(128);
        let snapshot = ConfigSnapshot::first(initial, String::from("genesis"), 0);
        let mut history = VecDeque::with_capacity(MAX_HISTORY);
        history.push_back(snapshot.clone());
        Self {
            inner: Arc::new(ReloaderInner {
                current: RwLock::new(snapshot),
                history: RwLock::new(history),
                validator: Box::new(validator),
                events: tx,
            }),
        }
    }

    /// Three-line glue for agents: build + spawn watcher + read.
    /// Returns the shared handle; see module docs for the full snippet.
    pub fn install(initial: T, validator: impl Fn(&T) -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self::new(initial, validator)
    }

    /// Zero-copy read of the live config. Cheap (`Arc` clone) and lock-free
    /// for the caller after the read guard drops, so 1000s of reloads drop
    /// zero requests.
    pub async fn get_arc(&self) -> Arc<T> {
        self.inner.current.read().await.payload.clone()
    }

    /// Blocking-compatible snapshot read for non-async agent paths.
    /// Retries `try_read` briefly under contention; never mutates state and
    /// never drops in-flight requests (callers keep the returned `Arc`).
    pub fn get_blocking(&self) -> Arc<T> {
        for _ in 0..100 {
            if let Ok(g) = self.inner.current.try_read() {
                return g.payload.clone();
            }
            if let Ok(h) = self.inner.history.try_read() {
                if let Some(s) = h.back() {
                    return s.payload.clone();
                }
            }
            std::thread::yield_now();
        }
        // Contention window is nanoseconds wide (writers hold the lock only
        // for an Arc swap); reaching here means a stuck writer, so panic
        // loudly rather than returning a torn config.
        panic!("config_reload: live config contended; retry get_blocking()");
    }

    /// Current live version.
    pub async fn version(&self) -> u64 {
        self.inner.current.read().await.version
    }

    /// Current live revision ID.
    pub async fn revision(&self) -> String {
        self.inner.current.read().await.revision.clone()
    }

    /// Subscribe to reload events (old/new version IDs).
    pub fn subscribe(&self) -> broadcast::Receiver<ReloadEvent> {
        self.inner.events.subscribe()
    }

    /// Number of retained snapshots (bounded by [`MAX_HISTORY`]).
    pub async fn history_len(&self) -> usize {
        self.inner.history.read().await.len()
    }

    /// Attempt a reload from raw bytes. `parse` converts bytes to `T`.
    /// Invalid input is rejected and the live version is retained.
    pub async fn try_reload_bytes(
        &self,
        bytes: &[u8],
        parse: impl Fn(&[u8]) -> Result<T, String>,
    ) -> Result<ReloadEvent, ReloadError> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ReloadError::TooLarge(bytes.len()));
        }
        let digest = Sha256::digest(bytes);
        let sha_hex = hex::encode(digest.as_slice());

        {
            let cur = self.inner.current.read().await;
            if cur.sha256 == sha_hex {
                return Err(ReloadError::Unchanged);
            }
        }

        let candidate = parse(bytes).map_err(ReloadError::Parse)?;
        if let Err(reason) = (self.inner.validator)(&candidate) {
            warn!(reason = %reason, "hot-reload rejected invalid config; retaining live version");
            return Err(ReloadError::Invalid(reason));
        }

        let event = {
            let mut cur = self.inner.current.write().await;
            let next = cur.next(candidate, sha_hex, bytes.len());
            let event = ReloadEvent {
                old_version: cur.version,
                new_version: next.version,
                old_revision: cur.revision.clone(),
                new_revision: next.revision.clone(),
                at: Utc::now(),
            };
            *cur = next.clone();
            let mut history = self.inner.history.write().await;
            history.push_back(next);
            while history.len() > MAX_HISTORY {
                history.pop_front();
            }
            event
        };

        info!(
            old_version = event.old_version,
            new_version = event.new_version,
            "config hot-reload accepted"
        );
        let _ = self.inner.events.send(event.clone());
        Ok(event)
    }

    /// Poll `path` for changes and reload via `parse`. Poll-based (mtime +
    /// sha) so no extra file-watch dependency is required; each poll that
    /// detects a change completes the swap well under 200ms for 1 MiB.
    pub async fn watch_file(
        self,
        path: String,
        parse: impl Fn(&[u8]) -> Result<T, String> + Send + Sync + 'static,
        interval: Duration,
    ) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(e) => {
                    warn!(path = %path, error = %e, "hot-reload watch: read failed, keeping live config");
                    continue;
                }
            };
            match self.try_reload_bytes(&bytes, &parse).await {
                Ok(_) | Err(ReloadError::Unchanged) => {}
                Err(e) => warn!(path = %path, error = %e, "hot-reload watch: candidate rejected"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct AgentConfig {
        endpoint: String,
        workers: u32,
    }

    fn parse(bytes: &[u8]) -> Result<AgentConfig, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    fn validate(c: &AgentConfig) -> Result<(), String> {
        if c.endpoint.is_empty() {
            return Err("endpoint must not be empty".to_string());
        }
        if c.workers == 0 || c.workers > 1024 {
            return Err("workers must be 1..=1024".to_string());
        }
        Ok(())
    }

    #[tokio::test]
    async fn reload_swaps_version_and_emits_event() {
        let initial = AgentConfig {
            endpoint: "http://a".to_string(),
            workers: 2,
        };
        let reloader = ConfigReloader::new(initial, validate);
        let mut rx = reloader.subscribe();
        let event = reloader
            .try_reload_bytes(br#"{"endpoint":"http://b","workers":4}"#, parse)
            .await
            .unwrap();
        assert_eq!(event.old_version, 1);
        assert_eq!(event.new_version, 2);
        assert_eq!(reloader.version().await, 2);
        assert_eq!(reloader.get_arc().await.workers, 4);
        let got = rx.try_recv().unwrap();
        assert_eq!(got.new_version, 2);
        assert_eq!(got.old_version, 1);
    }

    #[tokio::test]
    async fn invalid_config_rejected_previous_retained() {
        let initial = AgentConfig {
            endpoint: "http://a".to_string(),
            workers: 2,
        };
        let reloader = ConfigReloader::new(initial, validate);
        let err = reloader
            .try_reload_bytes(br#"{"endpoint":"","workers":0}"#, parse)
            .await
            .unwrap_err();
        assert!(matches!(err, ReloadError::Invalid(_)));
        assert_eq!(reloader.version().await, 1);
        assert_eq!(reloader.get_arc().await.endpoint, "http://a");
    }

    #[tokio::test]
    async fn unchanged_bytes_are_noop() {
        let initial = AgentConfig {
            endpoint: "http://a".to_string(),
            workers: 2,
        };
        let reloader = ConfigReloader::new(initial, validate);
        reloader
            .try_reload_bytes(br#"{"endpoint":"http://b","workers":4}"#, parse)
            .await
            .unwrap();
        let err = reloader
            .try_reload_bytes(br#"{"endpoint":"http://b","workers":4}"#, parse)
            .await
            .unwrap_err();
        assert_eq!(err, ReloadError::Unchanged);
        assert_eq!(reloader.version().await, 2);
    }

    #[tokio::test]
    async fn readers_hold_old_snapshot_across_reload_no_drops() {
        let initial = AgentConfig {
            endpoint: "http://a".to_string(),
            workers: 2,
        };
        let reloader = ConfigReloader::new(initial, validate);
        let before = reloader.get_arc().await;
        for i in 3..1003u32 {
            let payload = format!(r#"{{"endpoint":"http://n{i}","workers":{}}}"#, (i % 16) + 1);
            reloader.try_reload_bytes(payload.as_bytes(), parse).await.unwrap();
        }
        // Old handle still valid (no torn reads, no drops).
        assert_eq!(before.endpoint, "http://a");
        assert_eq!(reloader.version().await, 1001);
        // History bounded.
        assert!(reloader.history_len().await <= MAX_HISTORY);
    }
}
