//! # mnemo-graph
//!
//! Graph operations for Mnemo's temporal knowledge graph:
//! - Multi-hop traversal with temporal awareness
//! - Community detection (label propagation)
//! - Subgraph extraction for context assembly

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use uuid::Uuid;

use mnemo_core::models::edge::Edge;
use mnemo_core::models::entity::Entity;
use mnemo_core::traits::storage::{EdgeStore, EntityStore, StorageResult};

/// A node + its edges in a subgraph extraction.
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub entity: Entity,
    pub outgoing: Vec<Edge>,
    pub incoming: Vec<Edge>,
    /// Hop distance from the seed entity (0 = seed).
    pub depth: u32,
}

/// Result of a graph traversal.
#[derive(Debug, Clone)]
pub struct Subgraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
    /// Total entities visited during traversal.
    pub entities_visited: usize,
}

/// Graph traversal engine.
pub struct GraphEngine<S: EntityStore + EdgeStore> {
    store: Arc<S>,
}

impl<S: EntityStore + EdgeStore + Send + Sync + 'static> GraphEngine<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// BFS traversal from a seed entity, collecting connected subgraph.
    ///
    /// - `max_depth`: maximum hops from seed (1 = direct neighbors only)
    /// - `max_nodes`: maximum entities to include
    /// - `valid_only`: if true, only follow currently valid edges
    pub async fn traverse_bfs(
        &self,
        seed_entity_id: Uuid,
        max_depth: u32,
        max_nodes: usize,
        valid_only: bool,
    ) -> StorageResult<Subgraph> {
        let mut visited: HashSet<Uuid> = HashSet::new();
        let mut queue: VecDeque<(Uuid, u32)> = VecDeque::new();
        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut all_edges: Vec<Edge> = Vec::new();

        queue.push_back((seed_entity_id, 0));
        visited.insert(seed_entity_id);

        while let Some((entity_id, depth)) = queue.pop_front() {
            if nodes.len() >= max_nodes {
                break;
            }

            let entity = match self.store.get_entity(entity_id).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            let outgoing = self
                .store
                .get_outgoing_edges(entity_id)
                .await
                .unwrap_or_default();
            let incoming = self
                .store
                .get_incoming_edges(entity_id)
                .await
                .unwrap_or_default();

            let filtered_out: Vec<Edge> = if valid_only {
                outgoing.into_iter().filter(|e| e.is_valid()).collect()
            } else {
                outgoing
            };
            let filtered_in: Vec<Edge> = if valid_only {
                incoming.into_iter().filter(|e| e.is_valid()).collect()
            } else {
                incoming
            };

            // Queue neighbors for next depth
            if depth < max_depth {
                for edge in &filtered_out {
                    if !visited.contains(&edge.target_entity_id) {
                        visited.insert(edge.target_entity_id);
                        queue.push_back((edge.target_entity_id, depth + 1));
                    }
                }
                for edge in &filtered_in {
                    if !visited.contains(&edge.source_entity_id) {
                        visited.insert(edge.source_entity_id);
                        queue.push_back((edge.source_entity_id, depth + 1));
                    }
                }
            }

            all_edges.extend(filtered_out.iter().cloned());
            all_edges.extend(filtered_in.iter().cloned());

            nodes.push(GraphNode {
                entity,
                outgoing: filtered_out,
                incoming: filtered_in,
                depth,
            });
        }

        // Deduplicate edges
        let mut seen_edges: HashSet<Uuid> = HashSet::new();
        all_edges.retain(|e| seen_edges.insert(e.id));

        Ok(Subgraph {
            entities_visited: visited.len(),
            nodes,
            edges: all_edges,
        })
    }

    /// Simple label propagation community detection.
    /// Returns a map of entity_id -> community_id.
    pub async fn detect_communities(
        &self,
        user_id: Uuid,
        max_iterations: u32,
    ) -> StorageResult<HashMap<Uuid, Uuid>> {
        // Load all entities for user
        let entities = self.store.list_entities(user_id, 10000, None).await?;
        if entities.is_empty() {
            return Ok(HashMap::new());
        }

        // Initialize: each entity is its own community
        let mut labels: HashMap<Uuid, Uuid> = entities.iter().map(|e| (e.id, e.id)).collect();

        // Build adjacency from edges
        let mut adjacency: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for entity in &entities {
            let outgoing = self
                .store
                .get_outgoing_edges(entity.id)
                .await
                .unwrap_or_default();
            for edge in &outgoing {
                if !edge.is_valid() {
                    continue;
                }
                adjacency
                    .entry(entity.id)
                    .or_default()
                    .push(edge.target_entity_id);
                adjacency
                    .entry(edge.target_entity_id)
                    .or_default()
                    .push(entity.id);
            }
        }

        // Iterate label propagation
        for _ in 0..max_iterations {
            let mut changed = false;
            for entity in &entities {
                let neighbors = match adjacency.get(&entity.id) {
                    Some(n) => n,
                    None => continue,
                };

                // Count neighbor labels
                let mut label_counts: HashMap<Uuid, usize> = HashMap::new();
                for neighbor_id in neighbors {
                    if let Some(&label) = labels.get(neighbor_id) {
                        *label_counts.entry(label).or_default() += 1;
                    }
                }

                // Pick most frequent label
                if let Some((&best_label, _)) = label_counts.iter().max_by_key(|(_, &count)| count)
                {
                    let current = labels[&entity.id];
                    if best_label != current {
                        labels.insert(entity.id, best_label);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        Ok(labels)
    }
}
