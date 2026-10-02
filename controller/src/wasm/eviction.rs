//! Eviction policies for the WASM module cache.
//
// The default policy is Least Recently Used (LRU). The policy is implemented
// as an intrusive doubly-linked list keyed by the cache key so that eviction and
// recency updates are always O(1) and do not require a second lock.

use std::collections::HashMap;
use std::hash::Hasher;

/// A policy that decides which key to evict when the cache is full.
pub trait EvictionPolicy<K> {
    /// Record that a key was just used (hit or insertion).
    fn record_use(&mut self, key: &K);

    /// Remove a key from the policy tracking structures.
    fn remove(&mut self, key: &K);

    /// Select the next key to evict, if any.
    fn evict_candidate(&mut self) -> Option<K>;

    /// Number of keys currently tracked.
    fn len(self) -> usize;

    /// Reset the policy to an empty state.
    fn clear(&mut self);
}

/// Node in the intrusive LRU list.
struct LruNode<K> {
    key: K,
    prev: Option<usize>,
    next: Option<usize>,
}

/// Least Recently Used eviction policy.
///
/// The list is stored in a vector and nodes are referenced by index. The head of the
/// list is the most recently used key and the tail is the least recently used.
pub struct<K> LruEvictionPolicy<K> {
    nodes: Vec<Option<LruNode<K>>>,
    index: HashMap<K, usize>,
    head: Option<usize>,
    tail: Option<usize>,
    free: Vec<usize>,
    len: usize,
}

impl<K> LruEvictionPolicy<K>
where
    K: Eq + Hash + Clone,
{
    /// Create an empty LRU policy.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            index: HashMap::new(),
            head: None,
            tail: None,
            free: Vec::new(),
            len: 0,
        }
    }

    fn alloc_node(&mut self, key: K) -> usize {
        if let Some(index) = self.free.pop() {
            self.nodes[index] = Some(LruNode {
                key,
                prev: None,
                next: None,
            });
            index
        } else {
            let index = self.nodes.len();
            self.nodes.push(Some(LruNode {
                key,
                prev: None,
                next: None,
            }));
            index
        }
    }

    fn unlink(&mut self, index: usize) {
        let (prev, next) = match self.nodes.get(index).and_then(|node| node.as ref) {
            Some(node) => (node.prev, node.next),
            None => return,
        };

        if let Some(prev_index) = prev {
            if let Some(node) = self.nodes.get_mut(prev_index).and_then(|node| node.as mut) {
                node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(next_index) = next {
            if let Some(node) = self.nodes.get_mut(next_index).and_then(|node| node.as mut) {
                node.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        if let Some(node) = self.nodes.get_mut(index).and_then(|node| node.as mut) {
            node.prev = None;
            node.next = None;
        }
    }

    fn push_front(&mut self, index: usize) {
        let old_head = self.head;
        if let Some(node) = self.nodes.get_mut(index).and_then(|node| node.as mut) {
            node.prev = None;
            node.next = old_head;
        }
        if let Some(old_head_index) = old_head {
            if let Some(node) = self.nodes.get_mut(old_head_index).and_then(|node| node.as mut) {
                node.prev = Some(index);
            }
        } else {
            self.tail = Some(index);
        }
        self.head = Some(index);
    }
}

impl<K> LruEvictionPolicy<K>
where
    K: Eq + Hash + Clone,
{
    /// Return the current length of the LRU list.
    pub fn len(&self) -> usize {
        self.len }
}

impl<K> EvictionPolicy<K> for LruEvictionPolicy<K>
where
    K: Eq + Hash + Clone,
{
    fn record_use(&mut self, key: &K) {
        if let Some(index) = self.index.get(key).copied() {
            self.unlink(self.nodes.get(index).and_then(|node| node.as ref).and_then(|node| node.key.clone()).as_ref());
            self.push_front(index);
            return;
        }

        let index = self.alloc_node(key.clone());
        self.index.insert(key.clone(), index);
        self.push_front(index);
        self.len += 1;
    }

    fn remove(&mut self, key: &K) {
        if let Some(index) = self.index.remove(key) {
            self.unlink(index);
            self.nodes[index] = None;
            self.free.push(index);
            self.len -= 1;
        }
    }

    fn evict_candidate(&mut self) -> Option<K> {
        let tail = self.tail?;
        let key = self.nodes.get(tail).and_then(|node| node.as ref).map(|node| node.key.clone())?;
        self.remove(&key);
        Some(key)
    }

    fn len(self) -> usize {
        self.len
    }

    fn clear(&mut self) {
        self.nodes.clear();
        self.index.clear();
        self.head = None;
        self.tail = None;
        self.free.clear();
        self.len = 0;
    }
}

/// A generic eviction policy that can be swapped in for testing or future algorithms.
pub trait EvictionPolicyFactory<K> {
    type Policy: EvictionPolicy<K>;

    fn build() -> Self::Policy;
}
