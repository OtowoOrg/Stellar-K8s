//! Multi-layered cache for Soroban RPC WASM execution results.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  L1 – In-memory LRU  (hot data, sub-microsecond)    │
//! │  Capacity: configurable entry count                  │
//! └────────────────────┬────────────────────────────────┘
//!                      │ miss
//! ┌────────────────────▼────────────────────────────────┐
//! │  L2 – Local-SSD (emptyDir / NVMe hostPath)          │
//! │  Eviction: LFU-approximated via access-count files  │
//! │  Max size: configurable bytes                        │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! On a cache **hit** the value is promoted back to L1.
//! On a cache **miss** the caller computes the value and calls [`SorobanCache::put`].
//!
//! # Eviction policies
//!
//! | Layer | Policy | Rationale |
//! |-------|--------|-----------|
//! | L1    | LRU    | O(1) via `lru` crate; evicts least-recently-used entry |
//! | L2    | LFU-approx | Each entry has a `.cnt` sidecar file; evict lowest count when over budget |

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lru::LruCache;
use serde::{Deserialize, Serialize};

// ── Config ────────────────────────────────────────────────────────────────────

/// Eviction policy for the L2 (SSD) layer.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum EvictionPolicy {
    /// Evict the entry with the lowest access count (LFU approximation).
    #[default]
    Lfu,
    /// Evict the oldest entry by file modification time (LRU approximation).
    Lru,
}

/// Configuration for the two-layer Soroban RPC cache.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct SorobanCacheConfig {
    /// Maximum number of entries in the L1 in-memory LRU cache.
    #[serde(default = "default_l1_capacity")]
    pub l1_capacity: usize,

    /// Filesystem path for the L2 SSD cache (emptyDir or hostPath mount).
    #[serde(default = "default_l2_path")]
    pub l2_path: String,

    /// Maximum total bytes for the L2 SSD cache before eviction runs.
    #[serde(default = "default_l2_max_bytes")]
    pub l2_max_bytes: u64,

    /// Eviction policy for the L2 layer.
    #[serde(default)]
    pub eviction_policy: EvictionPolicy,
}

fn default_l1_capacity() -> usize {
    1_024
}
fn default_l2_path() -> String {
    "/cache/soroban".to_string()
}
fn default_l2_max_bytes() -> u64 {
    2 * 1024 * 1024 * 1024 // 2 GiB
}

impl Default for SorobanCacheConfig {
    fn default() -> Self {
        Self {
            l1_capacity: default_l1_capacity(),
            l2_path: default_l2_path(),
            l2_max_bytes: default_l2_max_bytes(),
            eviction_policy: EvictionPolicy::Lfu,
        }
    }
}

// ── Cache ─────────────────────────────────────────────────────────────────────

/// Two-layer cache: L1 in-memory LRU + L2 local-SSD with configurable eviction.
pub struct SorobanCache {
    l1: Mutex<LruCache<String, Vec<u8>>>,
    l2_path: PathBuf,
    l2_max_bytes: u64,
    eviction_policy: EvictionPolicy,
}

impl SorobanCache {
    /// Create a new cache from config. Creates the L2 directory if it doesn't exist.
    pub fn new(config: &SorobanCacheConfig) -> std::io::Result<Self> {
        let l2_path = PathBuf::from(&config.l2_path);
        std::fs::create_dir_all(&l2_path)?;

        let capacity = NonZeroUsize::new(config.l1_capacity.max(1)).unwrap();
        Ok(Self {
            l1: Mutex::new(LruCache::new(capacity)),
            l2_path,
            l2_max_bytes: config.l2_max_bytes,
            eviction_policy: config.eviction_policy.clone(),
        })
    }

    /// Look up a value. Checks L1 first, then L2. Promotes L2 hits to L1.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        // L1 hit
        {
            let mut l1 = self.l1.lock().unwrap();
            if let Some(v) = l1.get(key) {
                return Some(v.clone());
            }
        }

        // L2 hit
        if let Some(value) = self.l2_get(key) {
            // Promote to L1
            self.l1.lock().unwrap().put(key.to_string(), value.clone());
            return Some(value);
        }

        None
    }

    /// Insert a value into both L1 and L2.
    pub fn put(&self, key: &str, value: Vec<u8>) {
        self.l1.lock().unwrap().put(key.to_string(), value.clone());
        // Best-effort L2 write; ignore errors (cache is non-critical).
        let _ = self.l2_put(key, &value);
    }

    /// Explicitly evict a key from both layers.
    pub fn evict(&self, key: &str) {
        self.l1.lock().unwrap().pop(key);
        let _ = self.l2_evict_key(key);
    }

    // ── L2 helpers ────────────────────────────────────────────────────────────

    fn l2_data_path(&self, key: &str) -> PathBuf {
        self.l2_path.join(sanitise_key(key))
    }

    fn l2_cnt_path(&self, key: &str) -> PathBuf {
        self.l2_path.join(format!("{}.cnt", sanitise_key(key)))
    }

    fn l2_get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.l2_data_path(key);
        let data = std::fs::read(&path).ok()?;
        // Increment access counter (LFU approximation).
        let cnt_path = self.l2_cnt_path(key);
        let cnt = read_cnt(&cnt_path).unwrap_or(0) + 1;
        let _ = std::fs::write(&cnt_path, cnt.to_string());
        Some(data)
    }

    fn l2_put(&self, key: &str, value: &[u8]) -> std::io::Result<()> {
        // Evict if over budget before writing.
        self.l2_evict_if_needed(value.len() as u64)?;

        std::fs::write(self.l2_data_path(key), value)?;
        std::fs::write(self.l2_cnt_path(key), "1")?;
        Ok(())
    }

    fn l2_evict_key(&self, key: &str) -> std::io::Result<()> {
        let _ = std::fs::remove_file(self.l2_data_path(key));
        let _ = std::fs::remove_file(self.l2_cnt_path(key));
        Ok(())
    }

    /// Evict entries from L2 until there is room for `incoming_bytes`.
    fn l2_evict_if_needed(&self, incoming_bytes: u64) -> std::io::Result<()> {
        let current = self.l2_total_bytes()?;
        if current + incoming_bytes <= self.l2_max_bytes {
            return Ok(());
        }

        // Collect all data files with their eviction score.
        let mut candidates: Vec<(PathBuf, u64, u64)> = Vec::new(); // (path, score, size)
        for entry in std::fs::read_dir(&self.l2_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("cnt") {
                continue; // skip counter files
            }
            let size = entry.metadata()?.len();
            let score = match self.eviction_policy {
                EvictionPolicy::Lfu => {
                    let cnt_path = path.with_extension("cnt");
                    read_cnt(&cnt_path).unwrap_or(0)
                }
                EvictionPolicy::Lru => {
                    // Use mtime as score: older = lower score = evict first.
                    entry
                        .metadata()?
                        .modified()
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .map(|d| u64::MAX - d.as_secs()) // invert: older → lower
                        .unwrap_or(0)
                }
            };
            candidates.push((path, score, size));
        }

        // Sort ascending by score (lowest score evicted first).
        candidates.sort_by_key(|(_, score, _)| *score);

        let mut freed = 0u64;
        let need_to_free = (current + incoming_bytes).saturating_sub(self.l2_max_bytes);

        for (path, _, size) in candidates {
            if freed >= need_to_free {
                break;
            }
            let cnt_path = path.with_extension("cnt");
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&cnt_path);
            freed += size;
        }

        Ok(())
    }

    /// Sum of all data-file sizes in the L2 directory.
    fn l2_total_bytes(&self) -> std::io::Result<u64> {
        let mut total = 0u64;
        for entry in std::fs::read_dir(&self.l2_path)? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) != Some("cnt") {
                total += entry.metadata()?.len();
            }
        }
        Ok(total)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Replace characters that are unsafe in filenames with underscores.
fn sanitise_key(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

fn read_cnt(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_cache(dir: &TempDir) -> SorobanCache {
        let cfg = SorobanCacheConfig {
            l1_capacity: 4,
            l2_path: dir.path().to_str().unwrap().to_string(),
            l2_max_bytes: 1024,
            eviction_policy: EvictionPolicy::Lfu,
        };
        SorobanCache::new(&cfg).unwrap()
    }

    #[test]
    fn l1_hit() {
        let dir = TempDir::new().unwrap();
        let cache = make_cache(&dir);
        cache.put("k1", b"hello".to_vec());
        assert_eq!(cache.get("k1").unwrap(), b"hello");
    }

    #[test]
    fn l2_hit_after_l1_eviction() {
        let dir = TempDir::new().unwrap();
        let cfg = SorobanCacheConfig {
            l1_capacity: 2, // tiny L1
            l2_path: dir.path().to_str().unwrap().to_string(),
            l2_max_bytes: 64 * 1024,
            eviction_policy: EvictionPolicy::Lfu,
        };
        let cache = SorobanCache::new(&cfg).unwrap();

        cache.put("a", b"aaa".to_vec());
        cache.put("b", b"bbb".to_vec());
        // "a" is evicted from L1 when "c" and "d" are inserted
        cache.put("c", b"ccc".to_vec());
        cache.put("d", b"ddd".to_vec());

        // "a" should still be in L2
        assert_eq!(cache.get("a").unwrap(), b"aaa");
    }

    #[test]
    fn miss_returns_none() {
        let dir = TempDir::new().unwrap();
        let cache = make_cache(&dir);
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn explicit_evict() {
        let dir = TempDir::new().unwrap();
        let cache = make_cache(&dir);
        cache.put("k", b"val".to_vec());
        cache.evict("k");
        assert!(cache.get("k").is_none());
    }

    #[test]
    fn l2_lfu_eviction_removes_lowest_count() {
        let dir = TempDir::new().unwrap();
        let cfg = SorobanCacheConfig {
            l1_capacity: 1,
            l2_path: dir.path().to_str().unwrap().to_string(),
            l2_max_bytes: 20, // very small to force eviction
            eviction_policy: EvictionPolicy::Lfu,
        };
        let cache = SorobanCache::new(&cfg).unwrap();

        // Write two entries that together exceed the budget.
        // "hot" is accessed more times so it should survive.
        cache.put("hot", b"AAAAAAAAAA".to_vec()); // 10 bytes
        // Access "hot" multiple times to raise its count
        cache.get("hot");
        cache.get("hot");
        cache.get("hot");

        cache.put("cold", b"BBBBBBBBBB".to_vec()); // 10 bytes → total 20, at limit

        // Writing "new" (10 bytes) should evict "cold" (count=1) not "hot" (count>1)
        cache.put("new", b"CCCCCCCCCC".to_vec());

        // "hot" should still be retrievable from L2
        assert!(cache.get("hot").is_some(), "hot entry should survive LFU eviction");
    }

    #[test]
    fn sanitise_key_replaces_slashes() {
        assert_eq!(sanitise_key("contract/abc:123"), "contract_abc_123");
        assert_eq!(sanitise_key("simple-key"), "simple-key");
    }

    #[test]
    fn default_config_values() {
        let cfg = SorobanCacheConfig::default();
        assert_eq!(cfg.l1_capacity, 1_024);
        assert_eq!(cfg.l2_max_bytes, 2 * 1024 * 1024 * 1024);
        assert_eq!(cfg.eviction_policy, EvictionPolicy::Lfu);
    }

    // ── Benchmark: WASM execution time improvement ────────────────────────────
    //
    // Simulates the latency difference between a cache miss (compute) and a
    // cache hit (L1 lookup). Run with `cargo test -- --nocapture bench_wasm`
    // to see timing output.

    #[test]
    fn bench_wasm_cache_speedup() {
        use std::time::{Duration, Instant};

        let dir = TempDir::new().unwrap();
        let cfg = SorobanCacheConfig {
            l1_capacity: 256,
            l2_path: dir.path().to_str().unwrap().to_string(),
            l2_max_bytes: 64 * 1024 * 1024,
            eviction_policy: EvictionPolicy::Lfu,
        };
        let cache = SorobanCache::new(&cfg).unwrap();

        // Simulate WASM execution: 1 ms per invocation (realistic for simple contracts).
        let simulate_wasm_exec = || -> Vec<u8> {
            std::thread::sleep(Duration::from_millis(1));
            vec![0xDE, 0xAD, 0xBE, 0xEF; 256]
        };

        let key = "contract_abc123_invoke_transfer";
        const ITERATIONS: u32 = 10;

        // ── Cold path (cache miss) ────────────────────────────────────────────
        let cold_start = Instant::now();
        for _ in 0..ITERATIONS {
            let result = simulate_wasm_exec();
            cache.put(key, result);
        }
        let cold_elapsed = cold_start.elapsed();

        // ── Warm path (L1 cache hit) ──────────────────────────────────────────
        let warm_start = Instant::now();
        for _ in 0..ITERATIONS {
            let hit = cache.get(key);
            assert!(hit.is_some(), "expected L1 cache hit");
        }
        let warm_elapsed = warm_start.elapsed();

        let speedup = cold_elapsed.as_secs_f64() / warm_elapsed.as_secs_f64();
        println!(
            "\n[bench_wasm_cache_speedup] cold={cold_elapsed:?} warm={warm_elapsed:?} speedup={speedup:.0}x"
        );

        // The cache should be at least 100× faster than simulated WASM execution.
        assert!(
            speedup > 100.0,
            "expected >100x speedup, got {speedup:.1}x"
        );
    }
}
