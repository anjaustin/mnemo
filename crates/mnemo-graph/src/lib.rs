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

/// A step in a shortest path between two entities.
#[derive(Debug, Clone)]
pub struct PathStep {
    pub entity: Entity,
    /// The edge that was traversed to reach this entity (None for the source).
    pub edge: Option<Edge>,
    /// Hop distance from the source entity (0 = source).
    pub depth: u32,
}

/// Result of a shortest path search.
#[derive(Debug, Clone)]
pub struct ShortestPath {
    pub steps: Vec<PathStep>,
    /// Total entities visited during the search.
    pub entities_visited: usize,
    /// Whether a path was found.
    pub found: bool,
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
    /// - `user_id`: optional user ID to filter entities/edges (P2-4 security)
    /// - `max_depth`: maximum hops from seed (1 = direct neighbors only)
    /// - `max_nodes`: maximum entities to include
    /// - `valid_only`: if true, only follow currently valid edges
    ///
    /// **Security Note (P2-4):** When `user_id` is `Some`, only entities and edges
    /// belonging to that user are included in the traversal. This prevents data
    /// leakage between users in a multi-tenant environment.
    pub async fn traverse_bfs(
        &self,
        seed_entity_id: Uuid,
        max_depth: u32,
        max_nodes: usize,
        valid_only: bool,
    ) -> StorageResult<Subgraph> {
        // Delegate to user-filtered version without filter for backward compatibility
        self.traverse_bfs_for_user(seed_entity_id, None, max_depth, max_nodes, valid_only)
            .await
    }

    /// BFS traversal from a seed entity with user filtering (P2-4).
    ///
    /// - `user_id`: when Some, only traverse entities/edges owned by this user
    /// - `max_depth`: maximum hops from seed (1 = direct neighbors only)
    /// - `max_nodes`: maximum entities to include
    /// - `valid_only`: if true, only follow currently valid edges
    pub async fn traverse_bfs_for_user(
        &self,
        seed_entity_id: Uuid,
        user_id: Option<Uuid>,
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

            // P2-4: Skip entities belonging to other users
            if let Some(uid) = user_id {
                if entity.user_id != uid {
                    continue;
                }
            }

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

            // P2-4: Filter edges by user_id as well as validity
            let filtered_out: Vec<Edge> = outgoing
                .into_iter()
                .filter(|e| {
                    let user_ok = user_id.map_or(true, |uid| e.user_id == uid);
                    let valid_ok = !valid_only || e.is_valid();
                    user_ok && valid_ok
                })
                .collect();
            let filtered_in: Vec<Edge> = incoming
                .into_iter()
                .filter(|e| {
                    let user_ok = user_id.map_or(true, |uid| e.user_id == uid);
                    let valid_ok = !valid_only || e.is_valid();
                    user_ok && valid_ok
                })
                .collect();

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

    /// BFS shortest path from `from_id` to `to_id`.
    ///
    /// - `max_depth`: maximum hops to search (prevents runaway on large graphs)
    /// - `valid_only`: if true, only follow currently valid edges
    ///
    /// Returns a `ShortestPath` with the ordered steps from source to target,
    /// or an empty path with `found: false` if no path exists within `max_depth`.
    pub async fn find_shortest_path(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        max_depth: u32,
        valid_only: bool,
    ) -> StorageResult<ShortestPath> {
        // Delegate to user-filtered version without filter for backward compatibility
        self.find_shortest_path_for_user(from_id, to_id, None, max_depth, valid_only)
            .await
    }

    /// BFS shortest path from `from_id` to `to_id` with user filtering (P2-4).
    ///
    /// - `user_id`: when Some, only traverse entities/edges owned by this user
    /// - `max_depth`: maximum hops to search (prevents runaway on large graphs)
    /// - `valid_only`: if true, only follow currently valid edges
    ///
    /// Returns a `ShortestPath` with the ordered steps from source to target,
    /// or an empty path with `found: false` if no path exists within `max_depth`.
    pub async fn find_shortest_path_for_user(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        user_id: Option<Uuid>,
        max_depth: u32,
        valid_only: bool,
    ) -> StorageResult<ShortestPath> {
        if from_id == to_id {
            // Trivial: source == target
            let entity = match self.store.get_entity(from_id).await {
                Ok(e) => {
                    // P2-4: Verify entity belongs to user
                    if let Some(uid) = user_id {
                        if e.user_id != uid {
                            return Ok(ShortestPath {
                                steps: vec![],
                                entities_visited: 0,
                                found: false,
                            });
                        }
                    }
                    e
                }
                Err(_) => {
                    return Ok(ShortestPath {
                        steps: vec![],
                        entities_visited: 0,
                        found: false,
                    })
                }
            };
            return Ok(ShortestPath {
                steps: vec![PathStep {
                    entity,
                    edge: None,
                    depth: 0,
                }],
                entities_visited: 1,
                found: true,
            });
        }

        // BFS with parent tracking: entity_id -> (parent_entity_id, edge_used)
        let mut visited: HashMap<Uuid, (Option<Uuid>, Option<Edge>)> = HashMap::new();
        let mut queue: VecDeque<(Uuid, u32)> = VecDeque::new();

        visited.insert(from_id, (None, None));
        queue.push_back((from_id, 0));

        let mut found = false;

        while let Some((entity_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            // P2-4: Verify entity belongs to user before traversing
            if let Some(uid) = user_id {
                if let Ok(entity) = self.store.get_entity(entity_id).await {
                    if entity.user_id != uid {
                        continue;
                    }
                }
            }

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

            // P2-4: Filter edges by user_id and validity
            let filtered_out: Vec<Edge> = outgoing
                .into_iter()
                .filter(|e| {
                    let user_ok = user_id.map_or(true, |uid| e.user_id == uid);
                    let valid_ok = !valid_only || e.is_valid();
                    user_ok && valid_ok
                })
                .collect();
            let filtered_in: Vec<Edge> = incoming
                .into_iter()
                .filter(|e| {
                    let user_ok = user_id.map_or(true, |uid| e.user_id == uid);
                    let valid_ok = !valid_only || e.is_valid();
                    user_ok && valid_ok
                })
                .collect();

            // Check outgoing neighbors
            for edge in &filtered_out {
                let neighbor = edge.target_entity_id;
                if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(neighbor) {
                    e.insert((Some(entity_id), Some(edge.clone())));
                    if neighbor == to_id {
                        found = true;
                        break;
                    }
                    queue.push_back((neighbor, depth + 1));
                }
            }
            if found {
                break;
            }

            // Check incoming neighbors (bidirectional traversal)
            for edge in &filtered_in {
                let neighbor = edge.source_entity_id;
                if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(neighbor) {
                    e.insert((Some(entity_id), Some(edge.clone())));
                    if neighbor == to_id {
                        found = true;
                        break;
                    }
                    queue.push_back((neighbor, depth + 1));
                }
            }
            if found {
                break;
            }
        }

        let entities_visited = visited.len();

        if !found {
            return Ok(ShortestPath {
                steps: vec![],
                entities_visited,
                found: false,
            });
        }

        // Reconstruct path from target back to source
        let mut path_ids: Vec<(Uuid, Option<Edge>)> = Vec::new();
        let mut current = to_id;
        while let Some((parent, edge)) = visited.get(&current) {
            path_ids.push((current, edge.clone()));
            match parent {
                Some(p) => current = *p,
                None => break, // reached source
            }
        }
        path_ids.reverse();

        // Resolve entities for each step
        let mut steps = Vec::with_capacity(path_ids.len());
        for (i, (entity_id, edge)) in path_ids.into_iter().enumerate() {
            match self.store.get_entity(entity_id).await {
                Ok(entity) => {
                    // P2-4: Final verification that reconstructed path entities belong to user
                    if let Some(uid) = user_id {
                        if entity.user_id != uid {
                            continue;
                        }
                    }
                    steps.push(PathStep {
                        entity,
                        edge,
                        depth: i as u32,
                    })
                }
                Err(_) => continue,
            }
        }

        Ok(ShortestPath {
            steps,
            entities_visited,
            found: true,
        })
    }

    /// Simple label propagation community detection.
    /// Returns a map of entity_id -> community_id.
    pub async fn detect_communities(
        &self,
        user_id: Uuid,
        max_iterations: u32,
    ) -> StorageResult<HashMap<Uuid, Uuid>> {
        use mnemo_core::models::edge::EdgeFilter;

        // Load all entities for user
        let entities = self.store.list_entities(user_id, 10000, None).await?;
        if entities.is_empty() {
            return Ok(HashMap::new());
        }

        // Initialize: each entity is its own community
        let mut labels: HashMap<Uuid, Uuid> = entities.iter().map(|e| (e.id, e.id)).collect();

        // Build adjacency from edges — single batch query instead of N individual calls.
        // Use a high limit to fetch all edges; matches the entity cap of 10_000.
        let all_edges = self
            .store
            .query_edges(
                user_id,
                EdgeFilter {
                    include_invalidated: false,
                    limit: 100_000,
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_default();

        let mut adjacency: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for edge in &all_edges {
            adjacency
                .entry(edge.source_entity_id)
                .or_default()
                .push(edge.target_entity_id);
            adjacency
                .entry(edge.target_entity_id)
                .or_default()
                .push(edge.source_entity_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mnemo_core::error::MnemoError;
    use mnemo_core::models::edge::Edge;
    use mnemo_core::models::edge::EdgeFilter;
    use mnemo_core::models::entity::{Entity, EntityType};
    use std::collections::HashMap as StdHashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    // ── In-memory mock store ───────────────────────────────────────

    struct MockGraphStore {
        entities: Mutex<StdHashMap<Uuid, Entity>>,
        edges: Mutex<Vec<Edge>>,
    }

    impl MockGraphStore {
        fn new() -> Self {
            Self {
                entities: Mutex::new(StdHashMap::new()),
                edges: Mutex::new(Vec::new()),
            }
        }

        fn add_entity(&self, entity: Entity) {
            self.entities.lock().unwrap().insert(entity.id, entity);
        }

        fn add_edge(&self, edge: Edge) {
            self.edges.lock().unwrap().push(edge);
        }
    }

    fn make_entity(name: &str, user_id: Uuid) -> Entity {
        let now = Utc::now();
        Entity {
            id: Uuid::now_v7(),
            user_id,
            name: name.to_string(),
            entity_type: EntityType::Concept,
            summary: None,
            aliases: vec![],
            metadata: serde_json::Value::Null,
            classification: mnemo_core::models::classification::Classification::default(),
            mention_count: 1,
            community_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_edge(source: Uuid, target: Uuid, user_id: Uuid, valid: bool) -> Edge {
        let now = Utc::now();
        Edge {
            id: Uuid::now_v7(),
            user_id,
            source_entity_id: source,
            target_entity_id: target,
            label: "related_to".to_string(),
            fact: format!("{} is related to {}", source, target),
            valid_at: now,
            invalid_at: if valid { None } else { Some(now) },
            ingested_at: now,
            source_episode_id: Uuid::now_v7(),
            source_agent_id: None,
            invalidated_by_episode_id: None,
            confidence: 0.9,
            corroboration_count: 1,
            metadata: serde_json::Value::Null,
            classification: mnemo_core::models::classification::Classification::default(),
            created_at: now,
            updated_at: now,
            access_count: 0,
            last_accessed_at: None,
            temporal_scope: None,
        }
    }

    impl EntityStore for MockGraphStore {
        async fn create_entity(&self, entity: Entity) -> StorageResult<Entity> {
            self.entities
                .lock()
                .unwrap()
                .insert(entity.id, entity.clone());
            Ok(entity)
        }

        async fn get_entity(&self, id: Uuid) -> StorageResult<Entity> {
            self.entities
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .ok_or(MnemoError::EntityNotFound(id))
        }

        async fn update_entity(&self, entity: &Entity) -> StorageResult<()> {
            self.entities
                .lock()
                .unwrap()
                .insert(entity.id, entity.clone());
            Ok(())
        }

        async fn delete_entity(&self, id: Uuid) -> StorageResult<()> {
            self.entities.lock().unwrap().remove(&id);
            Ok(())
        }

        async fn find_entity_by_name(
            &self,
            user_id: Uuid,
            name: &str,
        ) -> StorageResult<Option<Entity>> {
            let entities = self.entities.lock().unwrap();
            Ok(entities
                .values()
                .find(|e| e.user_id == user_id && e.name.to_lowercase() == name.to_lowercase())
                .cloned())
        }

        async fn list_entities(
            &self,
            user_id: Uuid,
            limit: u32,
            _after: Option<Uuid>,
        ) -> StorageResult<Vec<Entity>> {
            let entities = self.entities.lock().unwrap();
            Ok(entities
                .values()
                .filter(|e| e.user_id == user_id)
                .take(limit as usize)
                .cloned()
                .collect())
        }
    }

    impl EdgeStore for MockGraphStore {
        async fn create_edge(&self, edge: Edge) -> StorageResult<Edge> {
            self.edges.lock().unwrap().push(edge.clone());
            Ok(edge)
        }

        async fn get_edge(&self, id: Uuid) -> StorageResult<Edge> {
            self.edges
                .lock()
                .unwrap()
                .iter()
                .find(|e| e.id == id)
                .cloned()
                .ok_or(MnemoError::EdgeNotFound(id))
        }

        async fn update_edge(&self, edge: &Edge) -> StorageResult<()> {
            let mut edges = self.edges.lock().unwrap();
            if let Some(e) = edges.iter_mut().find(|e| e.id == edge.id) {
                *e = edge.clone();
            }
            Ok(())
        }

        async fn delete_edge(&self, id: Uuid) -> StorageResult<()> {
            self.edges.lock().unwrap().retain(|e| e.id != id);
            Ok(())
        }

        async fn query_edges(&self, user_id: Uuid, filter: EdgeFilter) -> StorageResult<Vec<Edge>> {
            Ok(self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.user_id == user_id && filter.matches(e))
                .take(filter.limit as usize)
                .cloned()
                .collect())
        }

        async fn get_outgoing_edges(&self, entity_id: Uuid) -> StorageResult<Vec<Edge>> {
            Ok(self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.source_entity_id == entity_id)
                .cloned()
                .collect())
        }

        async fn get_incoming_edges(&self, entity_id: Uuid) -> StorageResult<Vec<Edge>> {
            Ok(self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.target_entity_id == entity_id)
                .cloned()
                .collect())
        }

        async fn find_conflicting_edges(
            &self,
            user_id: Uuid,
            source_entity_id: Uuid,
            target_entity_id: Uuid,
            label: &str,
        ) -> StorageResult<Vec<Edge>> {
            Ok(self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| {
                    e.user_id == user_id
                        && e.source_entity_id == source_entity_id
                        && e.target_entity_id == target_entity_id
                        && e.label == label
                })
                .cloned()
                .collect())
        }

        async fn record_edge_access(&self, _edge_id: Uuid) -> StorageResult<()> {
            Ok(())
        }
    }

    // ── GR-01: BFS returns correct subgraph at depth=1 ────────────

    #[tokio::test]
    async fn test_bfs_depth_1_returns_direct_neighbors() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // Build: A -> B -> C (chain)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());

        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(b.id, c.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine.traverse_bfs(a.id, 1, 100, false).await.unwrap();

        // At depth 1 from A, we should see A (depth=0) and B (depth=1), but NOT C
        let node_ids: HashSet<Uuid> = result.nodes.iter().map(|n| n.entity.id).collect();
        assert!(node_ids.contains(&a.id), "seed entity A must be present");
        assert!(
            node_ids.contains(&b.id),
            "direct neighbor B must be present"
        );
        assert!(
            !node_ids.contains(&c.id),
            "2-hop neighbor C must NOT be present at depth=1"
        );
        assert_eq!(result.nodes.len(), 2);
    }

    // ── GR-02: BFS respects max_nodes limit ───────────────────────

    #[tokio::test]
    async fn test_bfs_respects_max_nodes_limit() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // Build: A -> B, A -> C, A -> D, A -> E (star topology)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        let d = make_entity("D", user_id);
        let e = make_entity("E", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());
        store.add_entity(d.clone());
        store.add_entity(e.clone());

        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(a.id, c.id, user_id, true));
        store.add_edge(make_edge(a.id, d.id, user_id, true));
        store.add_edge(make_edge(a.id, e.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine.traverse_bfs(a.id, 10, 3, false).await.unwrap();

        // max_nodes=3 should cap at 3 nodes
        assert!(
            result.nodes.len() <= 3,
            "max_nodes=3 but got {} nodes",
            result.nodes.len()
        );
    }

    // ── GR-03: BFS respects depth parameter ───────────────────────

    #[tokio::test]
    async fn test_bfs_respects_depth_parameter() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // Build: A -> B -> C -> D -> E (chain of 5)
        let entities: Vec<Entity> = (0..5)
            .map(|i| make_entity(&format!("node_{}", i), user_id))
            .collect();
        for e in &entities {
            store.add_entity(e.clone());
        }
        for i in 0..4 {
            store.add_edge(make_edge(entities[i].id, entities[i + 1].id, user_id, true));
        }

        let engine = GraphEngine::new(store);
        let result = engine
            .traverse_bfs(entities[0].id, 2, 100, false)
            .await
            .unwrap();

        // depth=2: should see node_0 (depth=0), node_1 (depth=1), node_2 (depth=2)
        let node_ids: HashSet<Uuid> = result.nodes.iter().map(|n| n.entity.id).collect();
        assert!(node_ids.contains(&entities[0].id), "seed must be present");
        assert!(node_ids.contains(&entities[1].id), "1-hop must be present");
        assert!(node_ids.contains(&entities[2].id), "2-hop must be present");
        assert!(
            !node_ids.contains(&entities[3].id),
            "3-hop must NOT be present at depth=2"
        );
        assert!(
            !node_ids.contains(&entities[4].id),
            "4-hop must NOT be present at depth=2"
        );
    }

    // ── GR-04: valid_only filters out invalidated edges ───────────

    #[tokio::test]
    async fn test_bfs_valid_only_filters_invalidated_edges() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // A -> B (valid), A -> C (invalidated)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());

        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(a.id, c.id, user_id, false)); // invalidated

        let engine = GraphEngine::new(store);

        // valid_only=true should only follow valid edges
        let result = engine.traverse_bfs(a.id, 10, 100, true).await.unwrap();
        let node_ids: HashSet<Uuid> = result.nodes.iter().map(|n| n.entity.id).collect();
        assert!(node_ids.contains(&a.id));
        assert!(
            node_ids.contains(&b.id),
            "valid edge target must be present"
        );
        assert!(
            !node_ids.contains(&c.id),
            "invalidated edge target must NOT be present with valid_only=true"
        );

        // valid_only=false should follow all edges — rebuild fresh store
        let store2 = Arc::new(MockGraphStore::new());
        store2.add_entity(a.clone());
        store2.add_entity(b.clone());
        store2.add_entity(c.clone());
        store2.add_edge(make_edge(a.id, b.id, user_id, true));
        store2.add_edge(make_edge(a.id, c.id, user_id, false));
        let engine2 = GraphEngine::new(store2);
        let result2 = engine2.traverse_bfs(a.id, 10, 100, false).await.unwrap();
        let node_ids2: HashSet<Uuid> = result2.nodes.iter().map(|n| n.entity.id).collect();
        assert!(
            node_ids2.contains(&c.id),
            "invalidated edge target MUST be present with valid_only=false"
        );
    }

    // ── GR-05: Community detection produces non-trivial partitions ─

    #[tokio::test]
    async fn test_community_detection_finds_clusters() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // Cluster 1: A -- B -- C (fully connected)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        // Cluster 2: D -- E (isolated pair)
        let d = make_entity("D", user_id);
        let e = make_entity("E", user_id);

        for ent in [&a, &b, &c, &d, &e] {
            store.add_entity(ent.clone());
        }

        // Cluster 1 edges
        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(b.id, c.id, user_id, true));
        store.add_edge(make_edge(a.id, c.id, user_id, true));
        // Cluster 2 edges
        store.add_edge(make_edge(d.id, e.id, user_id, true));

        let engine = GraphEngine::new(store);
        let communities = engine.detect_communities(user_id, 10).await.unwrap();

        // All 5 entities should have community labels
        assert_eq!(communities.len(), 5);

        // Cluster 1 should share a label
        let label_a = communities[&a.id];
        let label_b = communities[&b.id];
        let label_c = communities[&c.id];
        assert_eq!(label_a, label_b, "A and B should be in same community");
        assert_eq!(label_b, label_c, "B and C should be in same community");

        // Cluster 2 should share a label
        let label_d = communities[&d.id];
        let label_e = communities[&e.id];
        assert_eq!(label_d, label_e, "D and E should be in same community");

        // Clusters should be different
        assert_ne!(
            label_a, label_d,
            "Cluster 1 and Cluster 2 should have different community labels"
        );
    }

    // ── GR-06: Empty graph returns empty subgraph ─────────────────

    #[tokio::test]
    async fn test_bfs_nonexistent_seed_returns_empty_subgraph() {
        let store = Arc::new(MockGraphStore::new());
        let engine = GraphEngine::new(store);
        let result = engine
            .traverse_bfs(Uuid::now_v7(), 5, 100, false)
            .await
            .unwrap();
        assert!(
            result.nodes.is_empty(),
            "nonexistent seed should yield empty subgraph"
        );
        assert!(result.edges.is_empty());
    }

    // ── GR-07: Community detection on empty graph returns empty ────

    #[tokio::test]
    async fn test_community_detection_empty_graph() {
        let store = Arc::new(MockGraphStore::new());
        let engine = GraphEngine::new(store);
        let communities = engine.detect_communities(Uuid::now_v7(), 10).await.unwrap();
        assert!(communities.is_empty());
    }

    // ── GR-08: BFS incoming edges are followed bidirectionally ─────

    #[tokio::test]
    async fn test_bfs_follows_incoming_edges() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // B -> A (A only has incoming edge, not outgoing)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_edge(make_edge(b.id, a.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine.traverse_bfs(a.id, 1, 100, false).await.unwrap();

        let node_ids: HashSet<Uuid> = result.nodes.iter().map(|n| n.entity.id).collect();
        assert!(
            node_ids.contains(&b.id),
            "BFS must follow incoming edges (B -> A means A's traversal finds B)"
        );
    }

    // ── GR-09: BFS edge deduplication ─────────────────────────────

    #[tokio::test]
    async fn test_bfs_deduplicates_edges() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // A -> B, B -> A (bidirectional — same edge seen from both sides)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());

        let edge = make_edge(a.id, b.id, user_id, true);
        store.add_edge(edge.clone());

        let engine = GraphEngine::new(store);
        let result = engine.traverse_bfs(a.id, 1, 100, false).await.unwrap();

        // The same edge should appear only once despite being visible from both A (outgoing) and B (incoming)
        let edge_ids: Vec<Uuid> = result.edges.iter().map(|e| e.id).collect();
        let unique: HashSet<Uuid> = edge_ids.iter().cloned().collect();
        assert_eq!(
            edge_ids.len(),
            unique.len(),
            "edges must be deduplicated in subgraph result"
        );
    }

    // ── GR-10: Node depth is correctly recorded ───────────────────

    #[tokio::test]
    async fn test_bfs_records_correct_depth() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // A -> B -> C
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());
        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(b.id, c.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine.traverse_bfs(a.id, 5, 100, false).await.unwrap();

        for node in &result.nodes {
            if node.entity.id == a.id {
                assert_eq!(node.depth, 0, "seed entity depth must be 0");
            } else if node.entity.id == b.id {
                assert_eq!(node.depth, 1, "direct neighbor depth must be 1");
            } else if node.entity.id == c.id {
                assert_eq!(node.depth, 2, "2-hop neighbor depth must be 2");
            }
        }
    }

    // ── SP-01: Shortest path finds direct connection ──────────────

    #[tokio::test]
    async fn test_shortest_path_direct_connection() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_edge(make_edge(a.id, b.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine
            .find_shortest_path(a.id, b.id, 10, false)
            .await
            .unwrap();

        assert!(result.found, "path should be found");
        assert_eq!(result.steps.len(), 2, "path A->B has 2 steps");
        assert_eq!(result.steps[0].entity.id, a.id);
        assert_eq!(result.steps[1].entity.id, b.id);
        assert!(
            result.steps[0].edge.is_none(),
            "source step has no incoming edge"
        );
        assert!(result.steps[1].edge.is_some(), "target step has edge");
    }

    // ── SP-02: Shortest path finds multi-hop route ────────────────

    #[tokio::test]
    async fn test_shortest_path_multi_hop() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // A -> B -> C -> D
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        let d = make_entity("D", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());
        store.add_entity(d.clone());
        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(b.id, c.id, user_id, true));
        store.add_edge(make_edge(c.id, d.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine
            .find_shortest_path(a.id, d.id, 10, false)
            .await
            .unwrap();

        assert!(result.found);
        assert_eq!(result.steps.len(), 4, "path A->B->C->D has 4 steps");
        assert_eq!(result.steps[0].entity.id, a.id);
        assert_eq!(result.steps[3].entity.id, d.id);
    }

    // ── SP-03: Shortest path returns not found for disconnected ───

    #[tokio::test]
    async fn test_shortest_path_disconnected() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        // No edge between A and B

        let engine = GraphEngine::new(store);
        let result = engine
            .find_shortest_path(a.id, b.id, 10, false)
            .await
            .unwrap();

        assert!(!result.found, "path should not be found");
        assert!(result.steps.is_empty());
    }

    // ── SP-04: Shortest path same source and target ───────────────

    #[tokio::test]
    async fn test_shortest_path_same_node() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        let a = make_entity("A", user_id);
        store.add_entity(a.clone());

        let engine = GraphEngine::new(store);
        let result = engine
            .find_shortest_path(a.id, a.id, 10, false)
            .await
            .unwrap();

        assert!(result.found);
        assert_eq!(result.steps.len(), 1, "same node path has 1 step");
        assert_eq!(result.steps[0].entity.id, a.id);
    }

    // ── SP-05: Shortest path respects max_depth ───────────────────

    #[tokio::test]
    async fn test_shortest_path_max_depth() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // A -> B -> C -> D (3 hops)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        let d = make_entity("D", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());
        store.add_entity(d.clone());
        store.add_edge(make_edge(a.id, b.id, user_id, true));
        store.add_edge(make_edge(b.id, c.id, user_id, true));
        store.add_edge(make_edge(c.id, d.id, user_id, true));

        let engine = GraphEngine::new(store);

        // max_depth=2 should fail (3 hops needed)
        let result = engine
            .find_shortest_path(a.id, d.id, 2, false)
            .await
            .unwrap();
        assert!(
            !result.found,
            "should not find path at depth 2 when 3 hops needed"
        );

        // max_depth=3 should succeed
        let store2 = Arc::new(MockGraphStore::new());
        store2.add_entity(a.clone());
        store2.add_entity(b.clone());
        store2.add_entity(c.clone());
        store2.add_entity(d.clone());
        store2.add_edge(make_edge(a.id, b.id, user_id, true));
        store2.add_edge(make_edge(b.id, c.id, user_id, true));
        store2.add_edge(make_edge(c.id, d.id, user_id, true));
        let engine2 = GraphEngine::new(store2);
        let result2 = engine2
            .find_shortest_path(a.id, d.id, 3, false)
            .await
            .unwrap();
        assert!(result2.found, "should find path at depth 3");
    }

    // ── SP-06: Shortest path valid_only filters invalidated edges ──

    #[tokio::test]
    async fn test_shortest_path_valid_only() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // A -> B (invalidated), A -> C -> B (valid path)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());
        store.add_edge(make_edge(a.id, b.id, user_id, false)); // invalidated
        store.add_edge(make_edge(a.id, c.id, user_id, true));
        store.add_edge(make_edge(c.id, b.id, user_id, true));

        let engine = GraphEngine::new(store);

        // valid_only=true should find A->C->B (2 hops)
        let result = engine
            .find_shortest_path(a.id, b.id, 10, true)
            .await
            .unwrap();
        assert!(result.found);
        assert_eq!(result.steps.len(), 3, "valid path is A->C->B (3 steps)");

        // valid_only=false should find A->B (1 hop, the direct but invalidated edge)
        let store2 = Arc::new(MockGraphStore::new());
        store2.add_entity(a.clone());
        store2.add_entity(b.clone());
        store2.add_entity(c.clone());
        store2.add_edge(make_edge(a.id, b.id, user_id, false));
        store2.add_edge(make_edge(a.id, c.id, user_id, true));
        store2.add_edge(make_edge(c.id, b.id, user_id, true));
        let engine2 = GraphEngine::new(store2);
        let result2 = engine2
            .find_shortest_path(a.id, b.id, 10, false)
            .await
            .unwrap();
        assert!(result2.found);
        assert_eq!(
            result2.steps.len(),
            2,
            "with invalidated edges included, direct path A->B (2 steps)"
        );
    }

    // ── SP-07: Shortest path follows incoming edges ────────────────

    #[tokio::test]
    async fn test_shortest_path_follows_incoming() {
        let user_id = Uuid::now_v7();
        let store = Arc::new(MockGraphStore::new());

        // B -> A, B -> C (path from A to C goes A<-B->C)
        let a = make_entity("A", user_id);
        let b = make_entity("B", user_id);
        let c = make_entity("C", user_id);
        store.add_entity(a.clone());
        store.add_entity(b.clone());
        store.add_entity(c.clone());
        store.add_edge(make_edge(b.id, a.id, user_id, true)); // incoming for A
        store.add_edge(make_edge(b.id, c.id, user_id, true));

        let engine = GraphEngine::new(store);
        let result = engine
            .find_shortest_path(a.id, c.id, 10, false)
            .await
            .unwrap();

        assert!(result.found, "should find path via incoming edge: A<-B->C");
        assert_eq!(result.steps.len(), 3);
    }

    // ── SP-08: Shortest path nonexistent entities ─────────────────

    #[tokio::test]
    async fn test_shortest_path_nonexistent() {
        let store = Arc::new(MockGraphStore::new());
        let engine = GraphEngine::new(store);

        let result = engine
            .find_shortest_path(Uuid::now_v7(), Uuid::now_v7(), 10, false)
            .await
            .unwrap();

        assert!(!result.found);
        assert!(result.steps.is_empty());
    }
}
