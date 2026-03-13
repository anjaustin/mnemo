//! Delta Consensus — CRDT primitives, vector clocks, and delta synchronization
//! for eventual-consistency across multiple Mnemo nodes.
//!
//! ## Design
//!
//! This module provides the foundational building blocks for multi-node memory
//! synchronization without requiring a consensus leader:
//!
//! - **CRDT types**: Conflict-free replicated data types that automatically
//!   converge when merged from any order of operations.
//! - **Vector clocks**: Causal ordering across nodes.
//! - **Hybrid Logical Clock (HLC)**: Wall-clock–aware timestamps that maintain
//!   causal ordering even with clock skew.
//! - **Delta envelopes**: Serializable deltas for efficient state transfer.
//! - **Merkle digest**: Fast state comparison for anti-entropy protocols.
//!
//! ## CRDT Types Provided
//!
//! | Type | Use Case |
//! |------|----------|
//! | `GCounter` | Monotonically increasing counters (mention_count, episode_count) |
//! | `LWWRegister<T>` | Last-Writer-Wins register (name, summary, status fields) |
//! | `ORSet<T>` | Observed-Remove set (aliases, tags, entity_ids) |
//! | `LWWMap<K, V>` | Last-Writer-Wins map (metadata fields) |
//!
//! ## Architecture
//!
//! ```text
//! Node A ──┐                    ┌── Node B
//!          │   DeltaEnvelope    │
//!          ├───────────────────►├──► merge into local state
//!          │                    │
//!          │◄───────────────────┤
//!          │   DeltaEnvelope    │
//! merge ◄──┘                    └──
//! ```
//!
//! Each node independently applies mutations, producing deltas. Deltas are
//! exchanged and merged, converging to identical state regardless of order.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

// ─── Node Identity ───────────────────────────────────────────────

/// Unique identifier for a Mnemo node in a multi-node deployment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ─── Hybrid Logical Clock (HLC) ──────────────────────────────────

/// Hybrid Logical Clock timestamp combining wall-clock time with a logical
/// counter to maintain causal ordering even under clock skew.
///
/// Ordering: (wall_time, counter, node_id) — fully deterministic total order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HlcTimestamp {
    /// Wall-clock milliseconds since Unix epoch.
    pub wall_ms: u64,
    /// Logical counter for events at the same wall_ms.
    pub counter: u32,
    /// Originating node.
    pub node_id: NodeId,
}

impl HlcTimestamp {
    /// Create a new HLC timestamp from current wall clock.
    pub fn now(node_id: &NodeId) -> Self {
        let wall_ms = Utc::now().timestamp_millis() as u64;
        Self {
            wall_ms,
            counter: 0,
            node_id: node_id.clone(),
        }
    }

    /// Create an HLC timestamp from explicit components (for testing).
    pub fn from_parts(wall_ms: u64, counter: u32, node_id: &NodeId) -> Self {
        Self {
            wall_ms,
            counter,
            node_id: node_id.clone(),
        }
    }
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HlcTimestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.wall_ms
            .cmp(&other.wall_ms)
            .then_with(|| self.counter.cmp(&other.counter))
            .then_with(|| self.node_id.cmp(&other.node_id))
    }
}

/// Hybrid Logical Clock state for a single node.
pub struct HybridClock {
    node_id: NodeId,
    last_wall_ms: u64,
    counter: u32,
}

impl HybridClock {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            last_wall_ms: 0,
            counter: 0,
        }
    }

    /// Generate a new timestamp. Monotonically increasing even if wall clock
    /// goes backward.
    pub fn now(&mut self) -> HlcTimestamp {
        let wall_ms = Utc::now().timestamp_millis() as u64;
        if wall_ms > self.last_wall_ms {
            self.last_wall_ms = wall_ms;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        HlcTimestamp {
            wall_ms: self.last_wall_ms,
            counter: self.counter,
            node_id: self.node_id.clone(),
        }
    }

    /// Receive a remote timestamp and update local clock.
    /// Returns a new local timestamp that is causally after the remote one.
    pub fn receive(&mut self, remote: &HlcTimestamp) -> HlcTimestamp {
        let wall_ms = Utc::now().timestamp_millis() as u64;
        if wall_ms > self.last_wall_ms && wall_ms > remote.wall_ms {
            // Local wall clock is ahead of both
            self.last_wall_ms = wall_ms;
            self.counter = 0;
        } else if remote.wall_ms > self.last_wall_ms {
            // Remote is ahead
            self.last_wall_ms = remote.wall_ms;
            self.counter = remote.counter + 1;
        } else if self.last_wall_ms > remote.wall_ms {
            // Local is ahead — just bump counter
            self.counter += 1;
        } else {
            // Same wall_ms — take max counter + 1
            self.counter = self.counter.max(remote.counter) + 1;
        }
        HlcTimestamp {
            wall_ms: self.last_wall_ms,
            counter: self.counter,
            node_id: self.node_id.clone(),
        }
    }

    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }
}

// ─── Vector Clock ────────────────────────────────────────────────

/// Vector clock for tracking causal relationships between nodes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VectorClock {
    /// Node → logical counter.
    pub entries: BTreeMap<NodeId, u64>,
}

impl VectorClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the counter for a node.
    pub fn increment(&mut self, node_id: &NodeId) -> u64 {
        let counter = self.entries.entry(node_id.clone()).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Get the counter for a node (0 if unseen).
    pub fn get(&self, node_id: &NodeId) -> u64 {
        self.entries.get(node_id).copied().unwrap_or(0)
    }

    /// Merge with another vector clock (point-wise max).
    pub fn merge(&mut self, other: &VectorClock) {
        for (node_id, &counter) in &other.entries {
            let entry = self.entries.entry(node_id.clone()).or_insert(0);
            *entry = (*entry).max(counter);
        }
    }

    /// Check if this clock is causally before or equal to another.
    pub fn is_before_or_equal(&self, other: &VectorClock) -> bool {
        for (node_id, &counter) in &self.entries {
            if counter > other.get(node_id) {
                return false;
            }
        }
        true
    }

    /// Check if this clock is strictly causally before another.
    pub fn is_strictly_before(&self, other: &VectorClock) -> bool {
        self.is_before_or_equal(other) && self != other
    }

    /// Check if two clocks are concurrent (neither is before the other).
    pub fn is_concurrent_with(&self, other: &VectorClock) -> bool {
        !self.is_before_or_equal(other) && !other.is_before_or_equal(self)
    }

    /// Number of nodes tracked.
    pub fn node_count(&self) -> usize {
        self.entries.len()
    }
}

// ─── G-Counter (Grow-Only Counter) ───────────────────────────────

/// Grow-only counter. Each node increments its own slot; the global count
/// is the sum of all slots. Merging is point-wise max.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GCounter {
    counts: BTreeMap<NodeId, u64>,
}

impl GCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment by 1 for the given node.
    pub fn increment(&mut self, node_id: &NodeId) {
        let count = self.counts.entry(node_id.clone()).or_insert(0);
        *count += 1;
    }

    /// Increment by an arbitrary amount.
    pub fn increment_by(&mut self, node_id: &NodeId, amount: u64) {
        let count = self.counts.entry(node_id.clone()).or_insert(0);
        *count += amount;
    }

    /// Total count across all nodes.
    pub fn value(&self) -> u64 {
        self.counts.values().sum()
    }

    /// Get the count for a specific node.
    pub fn node_value(&self, node_id: &NodeId) -> u64 {
        self.counts.get(node_id).copied().unwrap_or(0)
    }

    /// Merge with another G-Counter (point-wise max).
    pub fn merge(&mut self, other: &GCounter) {
        for (node_id, &count) in &other.counts {
            let entry = self.counts.entry(node_id.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }
}

// ─── LWW-Register (Last-Writer-Wins) ─────────────────────────────

/// Last-Writer-Wins register. Stores a single value with a timestamp;
/// the value with the highest timestamp wins on merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LWWRegister<T: Clone + Serialize> {
    pub value: T,
    pub timestamp: HlcTimestamp,
}

impl<T: Clone + Serialize> LWWRegister<T> {
    pub fn new(value: T, timestamp: HlcTimestamp) -> Self {
        Self { value, timestamp }
    }

    /// Update the value if the new timestamp is strictly greater.
    pub fn set(&mut self, value: T, timestamp: HlcTimestamp) -> bool {
        if timestamp > self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
            true
        } else {
            false
        }
    }

    /// Merge with another LWW register. The higher timestamp wins.
    pub fn merge(&mut self, other: &LWWRegister<T>) {
        if other.timestamp > self.timestamp {
            self.value = other.value.clone();
            self.timestamp = other.timestamp.clone();
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }
}

impl<T: Clone + Serialize + PartialEq> PartialEq for LWWRegister<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.timestamp == other.timestamp
    }
}

// ─── OR-Set (Observed-Remove Set) ─────────────────────────────────

/// Unique tag for each add operation in an OR-Set.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AddTag {
    pub node_id: NodeId,
    pub seq: u64,
}

/// Observed-Remove Set. Elements can be added and removed without conflict.
/// Each add creates a unique tag; remove deletes all observed tags for an element.
/// Concurrent add + remove → element survives (add-wins semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ORSet<T: Clone + Eq + std::hash::Hash + Serialize> {
    /// Element → set of tags that added it.
    elements: HashMap<T, HashSet<AddTag>>,
    /// Per-node sequence counter for generating unique tags.
    seq_counters: HashMap<NodeId, u64>,
}

impl<T: Clone + Eq + std::hash::Hash + Serialize> ORSet<T> {
    pub fn new() -> Self {
        Self {
            elements: HashMap::new(),
            seq_counters: HashMap::new(),
        }
    }

    fn next_tag(&mut self, node_id: &NodeId) -> AddTag {
        let seq = self.seq_counters.entry(node_id.clone()).or_insert(0);
        *seq += 1;
        AddTag {
            node_id: node_id.clone(),
            seq: *seq,
        }
    }

    /// Add an element. Returns the tag created.
    pub fn add(&mut self, element: T, node_id: &NodeId) -> AddTag {
        let tag = self.next_tag(node_id);
        self.elements
            .entry(element)
            .or_default()
            .insert(tag.clone());
        tag
    }

    /// Remove an element by removing all its currently observed tags.
    /// Returns true if the element was present.
    pub fn remove(&mut self, element: &T) -> bool {
        self.elements.remove(element).is_some()
    }

    /// Check if an element is in the set.
    pub fn contains(&self, element: &T) -> bool {
        self.elements
            .get(element)
            .is_some_and(|tags| !tags.is_empty())
    }

    /// Get all elements currently in the set.
    pub fn elements(&self) -> Vec<&T> {
        self.elements
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .map(|(elem, _)| elem)
            .collect()
    }

    /// Number of elements in the set.
    pub fn len(&self) -> usize {
        self.elements
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Merge with another OR-Set. Union of all (element, tag) pairs.
    pub fn merge(&mut self, other: &ORSet<T>) {
        for (elem, other_tags) in &other.elements {
            let entry = self.elements.entry(elem.clone()).or_default();
            for tag in other_tags {
                entry.insert(tag.clone());
            }
        }
        // Merge seq counters (point-wise max)
        for (node_id, &seq) in &other.seq_counters {
            let entry = self.seq_counters.entry(node_id.clone()).or_insert(0);
            *entry = (*entry).max(seq);
        }
    }
}

impl<T: Clone + Eq + std::hash::Hash + Serialize> Default for ORSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ─── LWW-Map ──────────────────────────────────────────────────────

/// Last-Writer-Wins map. Each key has an independent LWW register.
/// Keys can be added and tombstoned independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LWWMap<K: Clone + Eq + std::hash::Hash + Serialize + Ord, V: Clone + Serialize> {
    entries: BTreeMap<K, LWWEntry<V>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LWWEntry<V: Clone + Serialize> {
    value: Option<V>,
    timestamp: HlcTimestamp,
}

impl<K: Clone + Eq + std::hash::Hash + Serialize + Ord, V: Clone + Serialize> LWWMap<K, V> {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Set a key. Returns true if the value was updated.
    pub fn set(&mut self, key: K, value: V, timestamp: HlcTimestamp) -> bool {
        match self.entries.get(&key) {
            Some(entry) if entry.timestamp >= timestamp => false,
            _ => {
                self.entries.insert(
                    key,
                    LWWEntry {
                        value: Some(value),
                        timestamp,
                    },
                );
                true
            }
        }
    }

    /// Remove a key (tombstone). Returns true if applied.
    pub fn remove(&mut self, key: &K, timestamp: HlcTimestamp) -> bool {
        match self.entries.get(key) {
            Some(entry) if entry.timestamp >= timestamp => false,
            _ => {
                self.entries.insert(
                    key.clone(),
                    LWWEntry {
                        value: None,
                        timestamp,
                    },
                );
                true
            }
        }
    }

    /// Get a value by key (None if absent or tombstoned).
    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key).and_then(|entry| entry.value.as_ref())
    }

    /// Get all live (non-tombstoned) entries.
    pub fn entries(&self) -> Vec<(&K, &V)> {
        self.entries
            .iter()
            .filter_map(|(k, entry)| entry.value.as_ref().map(|v| (k, v)))
            .collect()
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.entries.values().filter(|e| e.value.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Merge with another LWW-Map. Per-key LWW semantics.
    pub fn merge(&mut self, other: &LWWMap<K, V>) {
        for (key, other_entry) in &other.entries {
            match self.entries.get(key) {
                Some(my_entry) if my_entry.timestamp >= other_entry.timestamp => {
                    // Local wins — do nothing
                }
                _ => {
                    self.entries.insert(key.clone(), other_entry.clone());
                }
            }
        }
    }
}

impl<K: Clone + Eq + std::hash::Hash + Serialize + Ord, V: Clone + Serialize> Default
    for LWWMap<K, V>
{
    fn default() -> Self {
        Self::new()
    }
}

// ─── Delta Envelope ──────────────────────────────────────────────

/// The type of resource a delta applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaResourceType {
    Entity,
    Edge,
    Episode,
    User,
    Session,
    AgentIdentity,
}

/// A single delta operation that can be applied to converge state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaOp {
    /// What type of resource this delta affects.
    pub resource_type: DeltaResourceType,
    /// UUID of the affected resource.
    pub resource_id: uuid::Uuid,
    /// The field being updated (e.g., "mention_count", "summary", "aliases").
    pub field: String,
    /// Serialized CRDT state for this field.
    pub crdt_state: serde_json::Value,
}

/// Envelope wrapping one or more deltas for transport between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaEnvelope {
    /// Unique ID for this envelope.
    pub id: uuid::Uuid,
    /// The node that produced these deltas.
    pub source_node: NodeId,
    /// Vector clock at the time of delta production.
    pub vector_clock: VectorClock,
    /// Individual delta operations.
    pub deltas: Vec<DeltaOp>,
    /// When the envelope was created.
    pub created_at: DateTime<Utc>,
}

impl DeltaEnvelope {
    pub fn new(source_node: NodeId, vector_clock: VectorClock, deltas: Vec<DeltaOp>) -> Self {
        Self {
            id: uuid::Uuid::now_v7(),
            source_node,
            vector_clock,
            deltas,
            created_at: Utc::now(),
        }
    }

    /// Number of delta operations in this envelope.
    pub fn delta_count(&self) -> usize {
        self.deltas.len()
    }

    /// Estimated size in bytes (for budget/throttle decisions).
    pub fn estimated_size_bytes(&self) -> usize {
        // Rough estimate: JSON serialization size
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(0)
    }
}

// ─── Merkle Digest (Anti-Entropy) ─────────────────────────────────

/// A Merkle tree node for efficient state comparison between nodes.
/// Leaf nodes hash individual resources; interior nodes hash their children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleNode {
    /// SHA-256 hash of this node's content.
    pub hash: String,
    /// Key prefix this node covers (e.g., "entity:", "edge:user_123:").
    pub prefix: String,
    /// Number of leaf resources under this node.
    pub count: u64,
}

/// Merkle digest for a specific resource type within a user's data.
/// Used for anti-entropy: nodes exchange digests and sync only the subtrees
/// where hashes diverge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleDigest {
    /// The resource type this digest covers.
    pub resource_type: DeltaResourceType,
    /// User ID scope (None = global).
    pub user_id: Option<uuid::Uuid>,
    /// Root hash of the Merkle tree.
    pub root_hash: String,
    /// Total leaf count.
    pub total_items: u64,
    /// When this digest was computed.
    pub computed_at: DateTime<Utc>,
    /// Top-level tree nodes for selective sync.
    pub nodes: Vec<MerkleNode>,
}

impl MerkleDigest {
    /// Compute a Merkle digest from a set of (key, hash) pairs.
    pub fn from_items(
        resource_type: DeltaResourceType,
        user_id: Option<uuid::Uuid>,
        items: &[(String, String)],
    ) -> Self {
        if items.is_empty() {
            return Self {
                resource_type,
                user_id,
                root_hash: compute_sha256(b"empty"),
                total_items: 0,
                computed_at: Utc::now(),
                nodes: Vec::new(),
            };
        }

        // Build leaf nodes
        let nodes: Vec<MerkleNode> = items
            .iter()
            .map(|(key, hash)| MerkleNode {
                hash: hash.clone(),
                prefix: key.clone(),
                count: 1,
            })
            .collect();

        // Compute root hash from all leaf hashes
        let mut hasher = Sha256::new();
        for node in &nodes {
            hasher.update(node.hash.as_bytes());
        }
        let root_hash = hex::encode(hasher.finalize());

        // For the digest we keep only a summary — bucket by first byte of key
        let mut buckets: BTreeMap<String, (Vec<String>, u64)> = BTreeMap::new();
        for node in &nodes {
            let prefix = node
                .prefix
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string());
            let bucket = buckets.entry(prefix).or_insert_with(|| (Vec::new(), 0));
            bucket.0.push(node.hash.clone());
            bucket.1 += 1;
        }

        let summary_nodes: Vec<MerkleNode> = buckets
            .into_iter()
            .map(|(prefix, (hashes, count))| {
                let mut h = Sha256::new();
                for hash in &hashes {
                    h.update(hash.as_bytes());
                }
                MerkleNode {
                    hash: hex::encode(h.finalize()),
                    prefix,
                    count,
                }
            })
            .collect();

        Self {
            resource_type,
            user_id,
            root_hash,
            total_items: items.len() as u64,
            computed_at: Utc::now(),
            nodes: summary_nodes,
        }
    }

    /// Compare two digests and return the prefixes that differ.
    pub fn diff_prefixes(&self, other: &MerkleDigest) -> Vec<String> {
        if self.root_hash == other.root_hash {
            return Vec::new();
        }

        let my_nodes: HashMap<&str, &str> = self
            .nodes
            .iter()
            .map(|n| (n.prefix.as_str(), n.hash.as_str()))
            .collect();
        let other_nodes: HashMap<&str, &str> = other
            .nodes
            .iter()
            .map(|n| (n.prefix.as_str(), n.hash.as_str()))
            .collect();

        let mut diffs = Vec::new();

        // Check all prefixes from both sides
        let all_prefixes: HashSet<&str> =
            my_nodes.keys().chain(other_nodes.keys()).copied().collect();

        for prefix in all_prefixes {
            let my_hash = my_nodes.get(prefix);
            let other_hash = other_nodes.get(prefix);
            if my_hash != other_hash {
                diffs.push(prefix.to_string());
            }
        }

        diffs.sort();
        diffs
    }
}

fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ─── Sync Status (for ops endpoint) ──────────────────────────────

/// Overall sync status for the ops endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    /// This node's identity.
    pub node_id: NodeId,
    /// Current vector clock state.
    pub vector_clock: VectorClock,
    /// Known peer nodes.
    pub known_peers: Vec<NodeId>,
    /// Number of deltas produced by this node.
    pub deltas_produced: u64,
    /// Number of deltas received from peers.
    pub deltas_received: u64,
    /// Number of merge conflicts resolved.
    pub conflicts_resolved: u64,
    /// Last sync timestamp per peer.
    pub last_sync: BTreeMap<NodeId, DateTime<Utc>>,
    /// Whether sync is enabled.
    pub enabled: bool,
}

impl SyncStatus {
    pub fn disabled() -> Self {
        Self {
            node_id: NodeId::new("standalone"),
            vector_clock: VectorClock::new(),
            known_peers: Vec::new(),
            deltas_produced: 0,
            deltas_received: 0,
            conflicts_resolved: 0,
            last_sync: BTreeMap::new(),
            enabled: false,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> NodeId {
        NodeId::new(id)
    }

    fn ts(wall_ms: u64, counter: u32, node_name: &str) -> HlcTimestamp {
        HlcTimestamp::from_parts(wall_ms, counter, &node(node_name))
    }

    // ─── NodeId tests ─────────────────────────────────────────────

    #[test]
    fn test_node_id_display() {
        let n = node("us-east-1");
        assert_eq!(n.to_string(), "us-east-1");
    }

    #[test]
    fn test_node_id_ordering() {
        let a = node("a");
        let b = node("b");
        assert!(a < b);
    }

    #[test]
    fn test_node_id_serde_roundtrip() {
        let n = node("eu-west-2");
        let json = serde_json::to_string(&n).unwrap();
        let back: NodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
    }

    // ─── HLC Timestamp tests ──────────────────────────────────────

    #[test]
    fn test_hlc_ordering_by_wall_time() {
        let t1 = ts(1000, 0, "a");
        let t2 = ts(2000, 0, "a");
        assert!(t1 < t2);
    }

    #[test]
    fn test_hlc_ordering_by_counter() {
        let t1 = ts(1000, 0, "a");
        let t2 = ts(1000, 1, "a");
        assert!(t1 < t2);
    }

    #[test]
    fn test_hlc_ordering_by_node_id() {
        let t1 = ts(1000, 0, "a");
        let t2 = ts(1000, 0, "b");
        assert!(t1 < t2);
    }

    #[test]
    fn test_hlc_now_creates_timestamp() {
        let n = node("test");
        let t = HlcTimestamp::now(&n);
        assert!(t.wall_ms > 0);
        assert_eq!(t.counter, 0);
        assert_eq!(t.node_id, n);
    }

    #[test]
    fn test_hlc_serde_roundtrip() {
        let t = ts(12345, 7, "node-x");
        let json = serde_json::to_string(&t).unwrap();
        let back: HlcTimestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    // ─── Hybrid Clock tests ───────────────────────────────────────

    #[test]
    fn test_hybrid_clock_monotonic() {
        let mut clock = HybridClock::new(node("n1"));
        let t1 = clock.now();
        let t2 = clock.now();
        let t3 = clock.now();
        assert!(t1 < t2);
        assert!(t2 < t3);
    }

    #[test]
    fn test_hybrid_clock_receive_advances() {
        let mut clock_a = HybridClock::new(node("a"));
        let mut clock_b = HybridClock::new(node("b"));

        let ta = clock_a.now();
        let tb = clock_b.receive(&ta);
        // tb should be causally after ta
        assert!(tb > ta);
    }

    #[test]
    fn test_hybrid_clock_receive_remote_ahead() {
        let mut clock = HybridClock::new(node("slow"));
        // Simulate a remote timestamp far in the future
        let remote = ts(u64::MAX / 2, 5, "fast");
        let local = clock.receive(&remote);
        assert!(local > remote);
    }

    // ─── Vector Clock tests ───────────────────────────────────────

    #[test]
    fn test_vector_clock_new_is_empty() {
        let vc = VectorClock::new();
        assert_eq!(vc.node_count(), 0);
        assert_eq!(vc.get(&node("a")), 0);
    }

    #[test]
    fn test_vector_clock_increment() {
        let mut vc = VectorClock::new();
        assert_eq!(vc.increment(&node("a")), 1);
        assert_eq!(vc.increment(&node("a")), 2);
        assert_eq!(vc.increment(&node("b")), 1);
        assert_eq!(vc.get(&node("a")), 2);
        assert_eq!(vc.get(&node("b")), 1);
    }

    #[test]
    fn test_vector_clock_merge() {
        let mut vc1 = VectorClock::new();
        vc1.increment(&node("a"));
        vc1.increment(&node("a"));
        vc1.increment(&node("b"));

        let mut vc2 = VectorClock::new();
        vc2.increment(&node("a"));
        vc2.increment(&node("c"));
        vc2.increment(&node("c"));

        vc1.merge(&vc2);
        assert_eq!(vc1.get(&node("a")), 2); // max(2, 1)
        assert_eq!(vc1.get(&node("b")), 1); // only in vc1
        assert_eq!(vc1.get(&node("c")), 2); // only in vc2
    }

    #[test]
    fn test_vector_clock_before_or_equal() {
        let mut vc1 = VectorClock::new();
        vc1.increment(&node("a"));

        let mut vc2 = VectorClock::new();
        vc2.increment(&node("a"));
        vc2.increment(&node("a"));

        assert!(vc1.is_before_or_equal(&vc2));
        assert!(!vc2.is_before_or_equal(&vc1));
    }

    #[test]
    fn test_vector_clock_concurrent() {
        let mut vc1 = VectorClock::new();
        vc1.increment(&node("a"));

        let mut vc2 = VectorClock::new();
        vc2.increment(&node("b"));

        assert!(vc1.is_concurrent_with(&vc2));
        assert!(vc2.is_concurrent_with(&vc1));
    }

    #[test]
    fn test_vector_clock_strictly_before() {
        let mut vc1 = VectorClock::new();
        vc1.increment(&node("a"));

        let mut vc2 = vc1.clone();
        vc2.increment(&node("a"));

        assert!(vc1.is_strictly_before(&vc2));
        assert!(!vc2.is_strictly_before(&vc1));
        assert!(!vc1.is_strictly_before(&vc1));
    }

    #[test]
    fn test_vector_clock_serde_roundtrip() {
        let mut vc = VectorClock::new();
        vc.increment(&node("us-east"));
        vc.increment(&node("eu-west"));
        let json = serde_json::to_string(&vc).unwrap();
        let back: VectorClock = serde_json::from_str(&json).unwrap();
        assert_eq!(vc, back);
    }

    // ─── G-Counter tests ──────────────────────────────────────────

    #[test]
    fn test_gcounter_empty() {
        let gc = GCounter::new();
        assert_eq!(gc.value(), 0);
    }

    #[test]
    fn test_gcounter_increment() {
        let mut gc = GCounter::new();
        gc.increment(&node("a"));
        gc.increment(&node("a"));
        gc.increment(&node("b"));
        assert_eq!(gc.value(), 3);
        assert_eq!(gc.node_value(&node("a")), 2);
        assert_eq!(gc.node_value(&node("b")), 1);
    }

    #[test]
    fn test_gcounter_increment_by() {
        let mut gc = GCounter::new();
        gc.increment_by(&node("a"), 10);
        gc.increment_by(&node("b"), 5);
        assert_eq!(gc.value(), 15);
    }

    #[test]
    fn test_gcounter_merge() {
        let mut gc1 = GCounter::new();
        gc1.increment_by(&node("a"), 5);
        gc1.increment_by(&node("b"), 3);

        let mut gc2 = GCounter::new();
        gc2.increment_by(&node("a"), 3);
        gc2.increment_by(&node("c"), 7);

        gc1.merge(&gc2);
        assert_eq!(gc1.value(), 15); // max(5,3) + 3 + 7 = 15
        assert_eq!(gc1.node_value(&node("a")), 5);
        assert_eq!(gc1.node_value(&node("b")), 3);
        assert_eq!(gc1.node_value(&node("c")), 7);
    }

    #[test]
    fn test_gcounter_merge_idempotent() {
        let mut gc1 = GCounter::new();
        gc1.increment_by(&node("a"), 5);

        let gc2 = gc1.clone();
        gc1.merge(&gc2);
        assert_eq!(gc1.value(), 5); // Idempotent
    }

    #[test]
    fn test_gcounter_merge_commutative() {
        let mut gc1 = GCounter::new();
        gc1.increment_by(&node("a"), 5);

        let mut gc2 = GCounter::new();
        gc2.increment_by(&node("b"), 3);

        let mut result_ab = gc1.clone();
        result_ab.merge(&gc2);

        let mut result_ba = gc2.clone();
        result_ba.merge(&gc1);

        assert_eq!(result_ab.value(), result_ba.value());
    }

    #[test]
    fn test_gcounter_serde_roundtrip() {
        let mut gc = GCounter::new();
        gc.increment_by(&node("x"), 42);
        let json = serde_json::to_string(&gc).unwrap();
        let back: GCounter = serde_json::from_str(&json).unwrap();
        assert_eq!(gc.value(), back.value());
    }

    // ─── LWW-Register tests ──────────────────────────────────────

    #[test]
    fn test_lww_register_basic() {
        let reg = LWWRegister::new("hello".to_string(), ts(100, 0, "a"));
        assert_eq!(reg.get(), "hello");
    }

    #[test]
    fn test_lww_register_set_newer_wins() {
        let mut reg = LWWRegister::new("old".to_string(), ts(100, 0, "a"));
        assert!(reg.set("new".to_string(), ts(200, 0, "a")));
        assert_eq!(reg.get(), "new");
    }

    #[test]
    fn test_lww_register_set_older_ignored() {
        let mut reg = LWWRegister::new("current".to_string(), ts(200, 0, "a"));
        assert!(!reg.set("stale".to_string(), ts(100, 0, "a")));
        assert_eq!(reg.get(), "current");
    }

    #[test]
    fn test_lww_register_merge() {
        let mut reg1 = LWWRegister::new("old".to_string(), ts(100, 0, "a"));
        let reg2 = LWWRegister::new("new".to_string(), ts(200, 0, "b"));
        reg1.merge(&reg2);
        assert_eq!(reg1.get(), "new");
    }

    #[test]
    fn test_lww_register_merge_commutative() {
        let reg1 = LWWRegister::new("val_a".to_string(), ts(100, 0, "a"));
        let reg2 = LWWRegister::new("val_b".to_string(), ts(200, 0, "b"));

        let mut r1 = reg1.clone();
        r1.merge(&reg2);

        let mut r2 = reg2.clone();
        r2.merge(&reg1);

        assert_eq!(r1.get(), r2.get());
    }

    // ─── OR-Set tests ─────────────────────────────────────────────

    #[test]
    fn test_orset_empty() {
        let set: ORSet<String> = ORSet::new();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn test_orset_add_contains() {
        let mut set = ORSet::new();
        set.add("hello".to_string(), &node("a"));
        assert!(set.contains(&"hello".to_string()));
        assert!(!set.contains(&"world".to_string()));
    }

    #[test]
    fn test_orset_remove() {
        let mut set = ORSet::new();
        set.add("item".to_string(), &node("a"));
        assert!(set.remove(&"item".to_string()));
        assert!(!set.contains(&"item".to_string()));
    }

    #[test]
    fn test_orset_add_wins_over_concurrent_remove() {
        // Node A adds "x", node B concurrently removes "x"
        // After merge, "x" should be present (add-wins)
        let mut set_a = ORSet::new();
        set_a.add("x".to_string(), &node("a"));

        let mut set_b = set_a.clone();
        // B removes "x" (removes A's tag)
        set_b.remove(&"x".to_string());
        // A concurrently adds "x" again (new tag)
        set_a.add("x".to_string(), &node("a"));

        // Merge B into A — A's new tag survives
        set_a.merge(&set_b);
        assert!(set_a.contains(&"x".to_string()));
    }

    #[test]
    fn test_orset_merge_union() {
        let mut set_a = ORSet::new();
        set_a.add("a".to_string(), &node("n1"));

        let mut set_b = ORSet::new();
        set_b.add("b".to_string(), &node("n2"));

        set_a.merge(&set_b);
        assert!(set_a.contains(&"a".to_string()));
        assert!(set_a.contains(&"b".to_string()));
        assert_eq!(set_a.len(), 2);
    }

    #[test]
    fn test_orset_merge_idempotent() {
        let mut set = ORSet::new();
        set.add("x".to_string(), &node("a"));
        let copy = set.clone();
        set.merge(&copy);
        assert_eq!(set.len(), 1);
    }

    // ─── LWW-Map tests ───────────────────────────────────────────

    #[test]
    fn test_lwwmap_empty() {
        let map: LWWMap<String, String> = LWWMap::new();
        assert!(map.is_empty());
    }

    #[test]
    fn test_lwwmap_set_get() {
        let mut map = LWWMap::new();
        map.set("key".to_string(), "val".to_string(), ts(100, 0, "a"));
        assert_eq!(map.get(&"key".to_string()), Some(&"val".to_string()));
    }

    #[test]
    fn test_lwwmap_newer_overwrites() {
        let mut map = LWWMap::new();
        map.set("k".to_string(), "v1".to_string(), ts(100, 0, "a"));
        map.set("k".to_string(), "v2".to_string(), ts(200, 0, "a"));
        assert_eq!(map.get(&"k".to_string()), Some(&"v2".to_string()));
    }

    #[test]
    fn test_lwwmap_older_ignored() {
        let mut map = LWWMap::new();
        map.set("k".to_string(), "v2".to_string(), ts(200, 0, "a"));
        assert!(!map.set("k".to_string(), "v1".to_string(), ts(100, 0, "a")));
        assert_eq!(map.get(&"k".to_string()), Some(&"v2".to_string()));
    }

    #[test]
    fn test_lwwmap_remove_tombstones() {
        let mut map = LWWMap::new();
        map.set("k".to_string(), "v".to_string(), ts(100, 0, "a"));
        map.remove(&"k".to_string(), ts(200, 0, "a"));
        assert!(map.get(&"k".to_string()).is_none());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_lwwmap_remove_older_than_set_ignored() {
        let mut map = LWWMap::new();
        map.set("k".to_string(), "v".to_string(), ts(200, 0, "a"));
        assert!(!map.remove(&"k".to_string(), ts(100, 0, "a")));
        assert_eq!(map.get(&"k".to_string()), Some(&"v".to_string()));
    }

    #[test]
    fn test_lwwmap_merge() {
        let mut map1 = LWWMap::new();
        map1.set("a".to_string(), "v1".to_string(), ts(100, 0, "n1"));

        let mut map2 = LWWMap::new();
        map2.set("a".to_string(), "v2".to_string(), ts(200, 0, "n2"));
        map2.set("b".to_string(), "vb".to_string(), ts(100, 0, "n2"));

        map1.merge(&map2);
        assert_eq!(map1.get(&"a".to_string()), Some(&"v2".to_string())); // newer wins
        assert_eq!(map1.get(&"b".to_string()), Some(&"vb".to_string())); // merged in
    }

    #[test]
    fn test_lwwmap_merge_commutative() {
        let mut m1 = LWWMap::new();
        m1.set("k".to_string(), "from_1".to_string(), ts(100, 0, "a"));

        let mut m2 = LWWMap::new();
        m2.set("k".to_string(), "from_2".to_string(), ts(200, 0, "b"));

        let mut r1 = m1.clone();
        r1.merge(&m2);
        let mut r2 = m2.clone();
        r2.merge(&m1);

        assert_eq!(r1.get(&"k".to_string()), r2.get(&"k".to_string()));
    }

    // ─── Delta Envelope tests ─────────────────────────────────────

    #[test]
    fn test_delta_envelope_creation() {
        let env = DeltaEnvelope::new(
            node("n1"),
            VectorClock::new(),
            vec![DeltaOp {
                resource_type: DeltaResourceType::Entity,
                resource_id: uuid::Uuid::from_u128(1),
                field: "mention_count".to_string(),
                crdt_state: serde_json::json!({"a": 5}),
            }],
        );
        assert_eq!(env.delta_count(), 1);
        assert_eq!(env.source_node, node("n1"));
    }

    #[test]
    fn test_delta_envelope_serializes() {
        let env = DeltaEnvelope::new(
            node("n1"),
            VectorClock::new(),
            vec![DeltaOp {
                resource_type: DeltaResourceType::Edge,
                resource_id: uuid::Uuid::from_u128(2),
                field: "confidence".to_string(),
                crdt_state: serde_json::json!(0.95),
            }],
        );
        let json = serde_json::to_value(&env).unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("source_node").is_some());
        assert!(json.get("vector_clock").is_some());
        assert!(json.get("deltas").is_some());
    }

    #[test]
    fn test_delta_envelope_estimated_size() {
        let env = DeltaEnvelope::new(node("n1"), VectorClock::new(), Vec::new());
        let size = env.estimated_size_bytes();
        assert!(size > 0);
    }

    // ─── Merkle Digest tests ──────────────────────────────────────

    #[test]
    fn test_merkle_digest_empty() {
        let digest = MerkleDigest::from_items(DeltaResourceType::Entity, None, &[]);
        assert_eq!(digest.total_items, 0);
        assert!(digest.nodes.is_empty());
    }

    #[test]
    fn test_merkle_digest_single_item() {
        let items = vec![("entity:abc".to_string(), compute_sha256(b"data"))];
        let digest = MerkleDigest::from_items(DeltaResourceType::Entity, None, &items);
        assert_eq!(digest.total_items, 1);
        assert!(!digest.root_hash.is_empty());
    }

    #[test]
    fn test_merkle_digest_diff_identical() {
        let items = vec![
            ("a:1".to_string(), compute_sha256(b"x")),
            ("b:1".to_string(), compute_sha256(b"y")),
        ];
        let d1 = MerkleDigest::from_items(DeltaResourceType::Entity, None, &items);
        let d2 = MerkleDigest::from_items(DeltaResourceType::Entity, None, &items);
        assert!(d1.diff_prefixes(&d2).is_empty());
    }

    #[test]
    fn test_merkle_digest_diff_detects_changes() {
        let items1 = vec![
            ("a:1".to_string(), compute_sha256(b"x")),
            ("b:1".to_string(), compute_sha256(b"y")),
        ];
        let items2 = vec![
            ("a:1".to_string(), compute_sha256(b"x")),
            ("b:1".to_string(), compute_sha256(b"CHANGED")),
        ];
        let d1 = MerkleDigest::from_items(DeltaResourceType::Entity, None, &items1);
        let d2 = MerkleDigest::from_items(DeltaResourceType::Entity, None, &items2);
        let diffs = d1.diff_prefixes(&d2);
        assert!(!diffs.is_empty());
        assert!(diffs.contains(&"b".to_string()));
    }

    // ─── Sync Status tests ────────────────────────────────────────

    #[test]
    fn test_sync_status_disabled() {
        let status = SyncStatus::disabled();
        assert!(!status.enabled);
        assert_eq!(status.node_id, node("standalone"));
    }

    #[test]
    fn test_sync_status_serializes() {
        let status = SyncStatus::disabled();
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["enabled"], false);
        assert!(json.get("node_id").is_some());
        assert!(json.get("vector_clock").is_some());
    }

    // ─── DeltaResourceType tests ──────────────────────────────────

    #[test]
    fn test_delta_resource_type_serde_roundtrip() {
        let types = vec![
            DeltaResourceType::Entity,
            DeltaResourceType::Edge,
            DeltaResourceType::Episode,
            DeltaResourceType::User,
            DeltaResourceType::Session,
            DeltaResourceType::AgentIdentity,
        ];
        for rt in types {
            let json = serde_json::to_string(&rt).unwrap();
            let back: DeltaResourceType = serde_json::from_str(&json).unwrap();
            assert_eq!(rt, back);
        }
    }

    // ─── Falsification / Adversarial Tests ────────────────────────

    #[test]
    fn test_falsify_gcounter_merge_associative() {
        // (A merge B) merge C == A merge (B merge C)
        let mut a = GCounter::new();
        a.increment_by(&node("a"), 10);
        let mut b = GCounter::new();
        b.increment_by(&node("b"), 20);
        let mut c = GCounter::new();
        c.increment_by(&node("c"), 30);

        let mut ab_c = a.clone();
        ab_c.merge(&b);
        ab_c.merge(&c);

        let mut a_bc = a.clone();
        let mut bc = b.clone();
        bc.merge(&c);
        a_bc.merge(&bc);

        assert_eq!(ab_c.value(), a_bc.value());
        assert_eq!(ab_c.value(), 60);
    }

    #[test]
    fn test_falsify_lww_register_same_timestamp_tiebreak_by_node() {
        // Same wall_ms, same counter — node_id breaks the tie deterministically.
        let mut reg = LWWRegister::new("from_a".to_string(), ts(100, 0, "a"));
        // "b" > "a" so ts(100,0,"b") > ts(100,0,"a") — b should win
        reg.merge(&LWWRegister::new("from_b".to_string(), ts(100, 0, "b")));
        assert_eq!(reg.get(), "from_b");

        // Reverse: start with b, merge a → b still wins
        let mut reg2 = LWWRegister::new("from_b".to_string(), ts(100, 0, "b"));
        reg2.merge(&LWWRegister::new("from_a".to_string(), ts(100, 0, "a")));
        assert_eq!(reg2.get(), "from_b");
    }

    #[test]
    fn test_falsify_orset_three_node_convergence() {
        // Node A adds "x", Node B adds "y", Node C removes "x" (from A's view)
        // After all merges, both "x" and "y" should survive if C only saw A's tags
        let mut set_a = ORSet::new();
        set_a.add("x".to_string(), &node("a"));

        let mut set_b = ORSet::new();
        set_b.add("y".to_string(), &node("b"));

        // C gets A's state, then removes "x"
        let mut set_c = set_a.clone();
        set_c.remove(&"x".to_string());

        // A adds "x" again concurrently (new tag)
        set_a.add("x".to_string(), &node("a"));

        // Now merge all: A <- B <- C
        set_a.merge(&set_b);
        set_a.merge(&set_c);

        // "x" survives (A's second add-tag was not in C's remove)
        assert!(set_a.contains(&"x".to_string()));
        // "y" present from B
        assert!(set_a.contains(&"y".to_string()));
    }

    #[test]
    fn test_falsify_lwwmap_tombstone_resurrection() {
        // Set, remove, then set again with even higher timestamp → value resurrected
        let mut map = LWWMap::new();
        map.set("k".to_string(), "v1".to_string(), ts(100, 0, "a"));
        map.remove(&"k".to_string(), ts(200, 0, "a"));
        assert!(map.get(&"k".to_string()).is_none());

        // Resurrect with timestamp > remove
        map.set("k".to_string(), "v2".to_string(), ts(300, 0, "a"));
        assert_eq!(map.get(&"k".to_string()), Some(&"v2".to_string()));
    }

    #[test]
    fn test_falsify_vector_clock_100_nodes_no_panic() {
        let mut vc1 = VectorClock::new();
        let mut vc2 = VectorClock::new();
        for i in 0..100 {
            let n = node(&format!("node-{}", i));
            vc1.increment(&n);
            if i % 2 == 0 {
                vc2.increment(&n);
                vc2.increment(&n);
            }
        }
        vc1.merge(&vc2);
        assert_eq!(vc1.node_count(), 100);
        // Even nodes: max(1, 2) = 2. Odd nodes: max(1, 0) = 1.
        assert_eq!(vc1.get(&node("node-0")), 2);
        assert_eq!(vc1.get(&node("node-1")), 1);
    }

    #[test]
    fn test_falsify_hlc_clock_skew_recovery() {
        // Local clock is way behind. Remote sends a timestamp far ahead.
        // After receive, local should generate timestamps after the remote.
        let mut clock = HybridClock::new(node("slow"));
        let remote_far_future = ts(9_999_999_999_000, 0, "fast");
        let local_after = clock.receive(&remote_far_future);
        assert!(local_after > remote_far_future);
        // Subsequent local timestamps should also be monotonic
        let next = clock.now();
        assert!(next > local_after);
    }

    #[test]
    fn test_falsify_merkle_digest_duplicate_keys() {
        // Same key appearing twice — should not panic
        let items = vec![
            ("same-key".to_string(), compute_sha256(b"data1")),
            ("same-key".to_string(), compute_sha256(b"data2")),
        ];
        let digest = MerkleDigest::from_items(DeltaResourceType::Entity, None, &items);
        assert_eq!(digest.total_items, 2);
        assert!(!digest.root_hash.is_empty());
    }

    #[test]
    fn test_falsify_delta_envelope_empty_deltas_valid() {
        let env = DeltaEnvelope::new(node("n1"), VectorClock::new(), Vec::new());
        assert_eq!(env.delta_count(), 0);
        let json = serde_json::to_value(&env).unwrap();
        let deltas = json["deltas"].as_array().unwrap();
        assert!(deltas.is_empty());
    }

    #[test]
    fn test_falsify_gcounter_increment_by_zero() {
        let mut gc = GCounter::new();
        gc.increment_by(&node("a"), 5);
        gc.increment_by(&node("a"), 0);
        assert_eq!(gc.value(), 5);
        assert_eq!(gc.node_value(&node("a")), 5);
    }

    #[test]
    fn test_falsify_orset_remove_nonexistent() {
        let mut set: ORSet<String> = ORSet::new();
        assert!(!set.remove(&"phantom".to_string()));
        assert!(set.is_empty());
    }

    #[test]
    fn test_falsify_lwwmap_merge_tombstone_vs_live() {
        // Node A has live value at t=200. Node B has tombstone at t=300.
        // After merge, tombstone wins (newer).
        let mut map_a = LWWMap::new();
        map_a.set("k".to_string(), "alive".to_string(), ts(200, 0, "a"));

        let mut map_b = LWWMap::new();
        map_b.set("k".to_string(), "doomed".to_string(), ts(100, 0, "b"));
        map_b.remove(&"k".to_string(), ts(300, 0, "b"));

        map_a.merge(&map_b);
        assert!(
            map_a.get(&"k".to_string()).is_none(),
            "tombstone at t=300 should win over live at t=200"
        );
    }

    #[test]
    fn test_falsify_vector_clock_not_concurrent_with_self() {
        let mut vc = VectorClock::new();
        vc.increment(&node("a"));
        assert!(
            !vc.is_concurrent_with(&vc),
            "a clock should not be concurrent with itself"
        );
        assert!(vc.is_before_or_equal(&vc));
        assert!(!vc.is_strictly_before(&vc));
    }
}
