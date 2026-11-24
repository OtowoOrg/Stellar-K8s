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
//! Soroban RPC pagination and ledger entry caching
//!
//! Provides response pagination limits for getEvents and getLedgerEntries,
//! cursor-based streaming pagination to avoid OOM on large event streams,
//! and an LRU cache bounded by memory (cacheSizeMB) for repeated ledger entry queries.

use crate::crd::SorobanConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use tracing::{debug, info};

/// Default maximum page size for events and ledger entry queries
pub const DEFAULT_MAX_PAGE_SIZE: u32 = 100;
/// Hard ceiling limit to protect against OOM even if requested
pub const HARD_PAGE_SIZE_LIMIT: u32 = 1000;
/// Default LRU cache size for ledger entries in megabytes
pub const DEFAULT_CACHE_SIZE_MB: u32 = 256;

// ── Event types & cursor pagination ──────────────────────────────────────────

/// Deterministic cursor for event streams: {ledger:010}:{tx_index:06}:{event_index:04}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EventCursor {
    pub ledger: u64,
    pub tx_index: u32,
    pub event_index: u32,
}

impl EventCursor {
    pub fn new(ledger: u64, tx_index: u32, event_index: u32) -> Self {
        Self {
            ledger,
            tx_index,
            event_index,
        }
    }

    pub fn to_cursor_string(&self) -> String {
        format!(
            "{:010}:{:06}:{:04}",
            self.ledger, self.tx_index, self.event_index
        )
    }

    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return None;
        }
        let ledger = parts[0].parse().ok()?;
        let tx_index = parts[1].parse().ok()?;
        let event_index = parts[2].parse().ok()?;
        Some(Self {
            ledger,
            tx_index,
            event_index,
        })
    }
}

/// Filter for getEvents RPC call
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<String>>,
}

/// Soroban RPC getEvents request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetEventsRequest {
    pub start_ledger: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ledger: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<EventFilter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Individual Soroban event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SorobanEvent {
    pub ledger: u64,
    pub ledger_closed_at: String,
    pub contract_id: String,
    pub id: String,
    pub paging_token: String,
    pub in_successful_contract_call: bool,
    pub topic: Vec<String>,
    pub value: String,
    pub tx_index: u32,
    pub event_index: u32,
}

impl SorobanEvent {
    pub fn cursor(&self) -> EventCursor {
        EventCursor::new(self.ledger, self.tx_index, self.event_index)
    }

    pub fn matches_filter(&self, filter: &EventFilter) -> bool {
        if let Some(ref contract_ids) = filter.contract_ids {
            if !contract_ids.contains(&self.contract_id) {
                return false;
            }
        }
        if let Some(ref req_topics) = filter.topics {
            for (idx, req_t) in req_topics.iter().enumerate() {
                if req_t == "*" {
                    continue;
                }
                if let Some(actual_t) = self.topic.get(idx) {
                    if actual_t != req_t {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
        true
    }
}

/// Soroban RPC getEvents response with cursor-based pagination
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GetEventsResponse {
    pub events: Vec<SorobanEvent>,
    pub latest_ledger: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

// ── Ledger Entry types & LRU cache ───────────────────────────────────────────

/// Soroban RPC getLedgerEntries request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetLedgerEntriesRequest {
    pub keys: Vec<String>,
}

/// Result entry for a single ledger key
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntryResult {
    pub key: String,
    pub xdr: String,
    pub last_modified_ledger_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_until_ledger_seq: Option<u64>,
}

/// Soroban RPC getLedgerEntries response
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GetLedgerEntriesResponse {
    pub entries: Vec<LedgerEntryResult>,
    pub latest_ledger: u64,
}

/// Cached ledger entry with byte accounting
#[derive(Debug, Clone)]
struct CachedEntry {
    entry: LedgerEntryResult,
    size_bytes: usize,
    last_accessed: u64,
}

/// LRU Cache for ledger entry queries with memory bounding in MB and hit ratio tracking
#[derive(Debug)]
pub struct LedgerEntryLruCache {
    max_bytes: usize,
    current_bytes: usize,
    access_counter: u64,
    entries: HashMap<String, CachedEntry>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl LedgerEntryLruCache {
    pub fn new(max_size_mb: u32) -> Self {
        let max_bytes = (max_size_mb as usize) * 1024 * 1024;
        Self {
            max_bytes,
            current_bytes: 0,
            access_counter: 0,
            entries: HashMap::new(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<LedgerEntryResult> {
        if let Some(cached) = self.entries.get_mut(key) {
            self.access_counter += 1;
            cached.last_accessed = self.access_counter;
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(cached.entry.clone())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    pub fn insert(&mut self, key: String, entry: LedgerEntryResult) {
        let entry_size = key.len() + entry.xdr.len() + 64; // approximate heap footprint

        // Evict if incoming entry alone exceeds budget or to make space
        while self.current_bytes + entry_size > self.max_bytes && !self.entries.is_empty() {
            // Find LRU entry
            if let Some(lru_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.last_accessed)
                .map(|(k, _)| k.clone())
            {
                if let Some(evicted) = self.entries.remove(&lru_key) {
                    self.current_bytes = self.current_bytes.saturating_sub(evicted.size_bytes);
                }
            } else {
                break;
            }
        }

        self.access_counter += 1;
        let cached = CachedEntry {
            entry,
            size_bytes: entry_size,
            last_accessed: self.access_counter,
        };

        if let Some(old) = self.entries.insert(key, cached) {
            self.current_bytes = self.current_bytes.saturating_sub(old.size_bytes);
        }
        self.current_bytes += entry_size;
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    pub fn hit_ratio(&self) -> f64 {
        let h = self.hits();
        let m = self.misses();
        let total = h + m;
        if total == 0 {
            0.0
        } else {
            (h as f64) / (total as f64)
        }
    }

    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// ── Soroban RPC Handler ──────────────────────────────────────────────────────

/// Handler implementing pagination and caching for Soroban RPC calls
pub struct SorobanRpcHandler {
    pub max_page_size: u32,
    pub cache_size_mb: u32,
    ledger_cache: RwLock<LedgerEntryLruCache>,
}

impl SorobanRpcHandler {
    pub fn new(max_page_size: u32, cache_size_mb: u32) -> Self {
        let max_page_size = max_page_size.clamp(1, HARD_PAGE_SIZE_LIMIT);
        let cache_size_mb = cache_size_mb.max(1);
        Self {
            max_page_size,
            cache_size_mb,
            ledger_cache: RwLock::new(LedgerEntryLruCache::new(cache_size_mb)),
        }
    }

    pub fn from_config(config: &SorobanConfig) -> Self {
        Self::new(
            config.effective_max_page_size(),
            config.effective_cache_size_mb(),
        )
    }

    /// Process getEvents with cursor-based pagination and strict response limits
    /// to guarantee memory remains bounded even over 100k ledgers.
    pub fn handle_get_events<I>(
        &self,
        req: &GetEventsRequest,
        event_stream: I,
        latest_ledger: u64,
    ) -> GetEventsResponse
    where
        I: Iterator<Item = SorobanEvent>,
    {
        // Calculate bounded page limit
        let page_limit = match req.limit {
            Some(lim) => lim.min(self.max_page_size).max(1),
            None => self.max_page_size,
        } as usize;

        let parsed_cursor = req.cursor.as_deref().and_then(EventCursor::parse);

        let mut collected = Vec::with_capacity(page_limit);
        let mut next_cursor: Option<String> = None;

        for event in event_stream {
            // Ledger range check
            if event.ledger < req.start_ledger {
                continue;
            }
            if let Some(end) = req.end_ledger {
                if event.ledger > end {
                    break;
                }
            }

            // Cursor filter: skip events strictly at or before cursor
            if let Some(ref cur) = parsed_cursor {
                if event.cursor() <= *cur {
                    continue;
                }
            }

            // Custom event filters
            if let Some(ref filters) = req.filters {
                let matches_any = filters.iter().any(|f| event.matches_filter(f));
                if !matches_any {
                    continue;
                }
            }

            // Check if page limit reached
            if collected.len() < page_limit {
                collected.push(event);
            } else {
                // Peeked one event past page_limit: generate nextCursor from last collected event
                if let Some(last) = collected.last() {
                    next_cursor = Some(last.cursor().to_cursor_string());
                }
                break;
            }
        }

        GetEventsResponse {
            events: collected,
            latest_ledger,
            cursor: next_cursor,
        }
    }

    /// Process getLedgerEntries checking LRU cache and populating missing keys
    pub fn handle_get_ledger_entries<F>(
        &self,
        req: &GetLedgerEntriesRequest,
        latest_ledger: u64,
        loader: F,
    ) -> GetLedgerEntriesResponse
    where
        F: Fn(&str) -> Option<LedgerEntryResult>,
    {
        let bounded_keys: Vec<_> = req
            .keys
            .iter()
            .take(self.max_page_size as usize)
            .cloned()
            .collect();

        let mut results = Vec::with_capacity(bounded_keys.len());
        let mut to_fetch = Vec::new();

        // 1. Try reading from cache
        {
            let mut cache = self.ledger_cache.write().unwrap();
            for key in bounded_keys {
                if let Some(entry) = cache.get(&key) {
                    results.push(entry);
                } else {
                    to_fetch.push(key);
                }
            }
        }

        // 2. Fetch cache misses and populate cache
        if !to_fetch.is_empty() {
            let mut cache = self.ledger_cache.write().unwrap();
            for key in to_fetch {
                if let Some(entry) = loader(&key) {
                    cache.insert(key, entry.clone());
                    results.push(entry);
                }
            }
        }

        GetLedgerEntriesResponse {
            entries: results,
            latest_ledger,
        }
    }

    /// Cache hit ratio metric: hits / (hits + misses)
    pub fn cache_hit_ratio(&self) -> f64 {
        self.ledger_cache.read().unwrap().hit_ratio()
    }

    /// Total cache hits
    pub fn cache_hits(&self) -> u64 {
        self.ledger_cache.read().unwrap().hits()
    }

    /// Total cache misses
    pub fn cache_misses(&self) -> u64 {
        self.ledger_cache.read().unwrap().misses()
    }

    /// Current memory usage of the cache in bytes
    pub fn cache_size_bytes(&self) -> usize {
        self.ledger_cache.read().unwrap().current_bytes()
    }

    /// Number of cached entries
    pub fn cache_entry_count(&self) -> usize {
        self.ledger_cache.read().unwrap().entry_count()
    }
}
