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
//! Hot-reload acceptance tests (#1977): 1 MiB reload latency, zero dropped
//! requests across 1000 reloads under concurrent traffic, and invalid-config
//! rejection with the previous version retained.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use stellar_k8s::config_reload::{ConfigReloader, ReloadError};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cfg {
    generation: u64,
    /// Always equal to `generation`; a torn read would break this.
    mirror: u64,
    padding: String,
}

fn parse(bytes: &[u8]) -> Result<Cfg, String> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

fn validate(c: &Cfg) -> Result<(), String> {
    if c.generation != c.mirror {
        return Err("generation and mirror disagree".to_string());
    }
    Ok(())
}

fn encode(generation: u64, padding_len: usize) -> Vec<u8> {
    serde_json::to_vec(&Cfg {
        generation,
        mirror: generation,
        padding: "x".repeat(padding_len),
    })
    .unwrap()
}

fn initial() -> Cfg {
    Cfg {
        generation: 0,
        mirror: 0,
        padding: String::new(),
    }
}

#[tokio::test]
async fn one_mebibyte_reload_completes_under_200ms() {
    let r = ConfigReloader::new(initial(), validate);
    let bytes = encode(1, 1024 * 1024);
    assert!(bytes.len() >= 1024 * 1024);
    let start = Instant::now();
    r.try_reload_bytes(&bytes, parse).await.expect("reload");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(200),
        "reload took {elapsed:?}"
    );
    assert_eq!(r.get_arc().await.generation, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousand_reloads_drop_no_requests_and_never_tear() {
    let r = ConfigReloader::new(initial(), validate);
    let issued = Arc::new(AtomicU64::new(0));
    let served = Arc::new(AtomicU64::new(0));
    let torn = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut readers = Vec::new();
    for _ in 0..8 {
        let (r, issued, served, torn, stop) = (
            r.clone(),
            issued.clone(),
            served.clone(),
            torn.clone(),
            stop.clone(),
        );
        readers.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                issued.fetch_add(1, Ordering::Relaxed);
                let cfg = r.get_arc().await;
                if cfg.generation != cfg.mirror {
                    torn.fetch_add(1, Ordering::Relaxed);
                }
                // The request keeps using its snapshot across an await point.
                tokio::task::yield_now().await;
                if cfg.generation == cfg.mirror {
                    served.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    for g in 1..=1000u64 {
        r.try_reload_bytes(&encode(g, 256), parse)
            .await
            .expect("reload");
    }
    stop.store(true, Ordering::Relaxed);
    for h in readers {
        h.await.unwrap();
    }

    assert_eq!(r.version().await, 1001);
    assert_eq!(torn.load(Ordering::Relaxed), 0, "torn reads");
    assert_eq!(
        issued.load(Ordering::Relaxed),
        served.load(Ordering::Relaxed),
        "dropped requests"
    );
    assert!(issued.load(Ordering::Relaxed) > 0);
}

#[tokio::test]
async fn invalid_config_is_rejected_and_previous_version_retained() {
    let r = ConfigReloader::new(initial(), validate);
    r.try_reload_bytes(&encode(5, 8), parse).await.unwrap();

    // Parse failure.
    let err = r.try_reload_bytes(b"{not json", parse).await.unwrap_err();
    assert!(matches!(err, ReloadError::Parse(_)));

    // Validation failure.
    let bad = serde_json::to_vec(&Cfg {
        generation: 6,
        mirror: 7,
        padding: String::new(),
    })
    .unwrap();
    let err = r.try_reload_bytes(&bad, parse).await.unwrap_err();
    assert!(matches!(err, ReloadError::Invalid(_)));

    assert_eq!(r.get_arc().await.generation, 5);
    assert_eq!(r.version().await, 2);
}

#[tokio::test]
async fn fuzzed_mutations_never_break_live_config() {
    let r = ConfigReloader::new(initial(), validate);
    r.try_reload_bytes(&encode(1, 64), parse).await.unwrap();
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut accepted_gen = 1u64;
    for i in 0..2000u64 {
        // xorshift for deterministic pseudo-random mutations.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let mut bytes = encode(i + 2, 64);
        let idx = (seed as usize) % bytes.len();
        match seed % 3 {
            0 => bytes[idx] = (seed >> 8) as u8,
            1 => bytes.truncate(idx),
            _ => {}
        }
        if r.try_reload_bytes(&bytes, parse).await.is_ok() {
            accepted_gen = r.get_arc().await.generation;
        }
        let live = r.get_arc().await;
        assert_eq!(live.generation, live.mirror);
        assert_eq!(live.generation, accepted_gen);
    }
}
