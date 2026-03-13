//! Coherence scoring system for user knowledge graphs.
//!
//! Measures how internally consistent a user's memory is across four dimensions:
//!
//! 1. **Entity coherence**: Are connected entities semantically related?
//! 2. **Fact coherence**: Are active facts mutually consistent (no conflicts)?
//! 3. **Temporal coherence**: Do recent episodes reinforce vs. constantly supersede?
//! 4. **Structural coherence**: Is the graph well-connected or fragmented?
//!
//! The composite score is a weighted average of sub-scores, ranging from 0.0 to 1.0.

use mnemo_core::models::edge::Edge;
use mnemo_core::models::entity::Entity;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ─── Coherence Result ──────────────────────────────────────────────

/// Complete coherence assessment for a user's knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoherenceReport {
    /// Composite coherence score (0.0–1.0). Weighted average of sub-scores.
    pub score: f32,
    /// Entity coherence: are connected entities semantically related?
    pub entity_coherence: f32,
    /// Fact coherence: are active facts mutually consistent?
    pub fact_coherence: f32,
    /// Temporal coherence: do recent episodes reinforce the knowledge graph?
    pub temporal_coherence: f32,
    /// Structural coherence: is the graph well-connected?
    pub structural_coherence: f32,
    /// Human-readable recommendations for improving coherence.
    pub recommendations: Vec<String>,
    /// Diagnostic details.
    pub diagnostics: CoherenceDiagnostics,
}

/// Diagnostic counters for coherence scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoherenceDiagnostics {
    pub total_entities: usize,
    pub total_edges: usize,
    pub active_edges: usize,
    pub invalidated_edges: usize,
    pub conflicting_groups: usize,
    pub communities_detected: usize,
    pub isolated_entities: usize,
    pub recent_supersessions: usize,
    pub recent_corroborations: usize,
}

// ─── Sub-score Weights ─────────────────────────────────────────────

/// Weights for the composite coherence score.
const W_ENTITY: f32 = 0.20;
const W_FACT: f32 = 0.35;
const W_TEMPORAL: f32 = 0.20;
const W_STRUCTURAL: f32 = 0.25;

// ─── Entity Coherence ──────────────────────────────────────────────

/// Compute entity coherence based on type consistency of connected entity pairs.
///
/// For each active edge, check whether the source and target entities have
/// compatible types (e.g., Person → Organization via "works_at" makes sense).
/// A graph where most connections are between semantically related entity types
/// scores higher.
///
/// Also penalizes entities with very low mention counts (weak evidence).
pub fn compute_entity_coherence(entities: &[Entity], edges: &[Edge]) -> f32 {
    if entities.is_empty() {
        return 1.0; // vacuously coherent
    }

    let entity_map: HashMap<Uuid, &Entity> = entities.iter().map(|e| (e.id, e)).collect();

    let active_edges: Vec<&Edge> = edges.iter().filter(|e| e.is_valid()).collect();
    if active_edges.is_empty() {
        return 1.0;
    }

    let mut coherent_pairs = 0usize;
    let mut total_pairs = 0usize;

    for edge in &active_edges {
        let src = entity_map.get(&edge.source_entity_id);
        let tgt = entity_map.get(&edge.target_entity_id);

        total_pairs += 1;

        // Edges referencing entities not in the entity list are orphaned —
        // count as incoherent rather than silently skipping them.
        let (src_entity, tgt_entity) = match (src, tgt) {
            (Some(s), Some(t)) => (s, t),
            _ => continue, // incoherent: total_pairs incremented but coherent_pairs not
        };

        // Type compatibility heuristic:
        // - Different entity types connecting = good (diverse, meaningful relations)
        // - Same entity type connecting via high-confidence edge = good
        // - Low confidence edges = lower coherence contribution
        let type_diverse = src_entity.entity_type != tgt_entity.entity_type;
        let high_confidence = edge.confidence >= 0.5;
        let well_corroborated = edge.corroboration_count >= 2;

        if (type_diverse && high_confidence) || well_corroborated || edge.confidence >= 0.7 {
            coherent_pairs += 1;
        }
    }

    if total_pairs == 0 {
        return 1.0;
    }

    // Penalty for entities with very weak evidence (mention_count == 1)
    let weak_entities = entities.iter().filter(|e| e.mention_count <= 1).count();
    let weak_ratio = weak_entities as f32 / entities.len() as f32;
    let weak_penalty = (weak_ratio * 0.3).min(0.3); // max 30% penalty

    let base_score = coherent_pairs as f32 / total_pairs as f32;
    (base_score - weak_penalty).clamp(0.0, 1.0)
}

// ─── Fact Coherence ────────────────────────────────────────────────

/// Compute fact coherence by detecting conflicting edge groups.
///
/// Groups edges by (source_entity_id, label). A group with multiple active
/// edges simultaneously is a potential conflict. The ratio of clean groups
/// to total groups gives the fact coherence score.
pub fn compute_fact_coherence(edges: &[Edge]) -> (f32, usize) {
    if edges.is_empty() {
        return (1.0, 0);
    }

    // Group by (source_entity_id, label)
    let mut groups: HashMap<(Uuid, String), Vec<&Edge>> = HashMap::new();
    for edge in edges {
        groups
            .entry((edge.source_entity_id, edge.label.clone()))
            .or_default()
            .push(edge);
    }

    let total_groups = groups.len();
    if total_groups == 0 {
        return (1.0, 0);
    }

    let mut conflicting_groups = 0usize;
    for group_edges in groups.values() {
        let active_count = group_edges.iter().filter(|e| e.is_valid()).count();
        if active_count > 1 {
            conflicting_groups += 1;
        }
    }

    let clean_ratio = (total_groups - conflicting_groups) as f32 / total_groups as f32;
    (clean_ratio.clamp(0.0, 1.0), conflicting_groups)
}

// ─── Temporal Coherence ────────────────────────────────────────────

/// Compute temporal coherence based on the ratio of corroborations to supersessions
/// in recent edges.
///
/// A memory system where recent activity mostly confirms existing knowledge
/// (corroborations) is more coherent than one where facts are constantly being
/// replaced (supersessions).
///
/// `recent_window_days`: how many days back to look (default 30).
pub fn compute_temporal_coherence(edges: &[Edge], recent_window_days: i64) -> (f32, usize, usize) {
    let now = chrono::Utc::now();
    let window_start = now - chrono::Duration::days(recent_window_days);

    let mut recent_supersessions = 0usize;
    let mut recent_corroborations = 0usize;

    for edge in edges {
        // Count recent supersessions
        let was_recently_superseded = edge.invalid_at.map(|t| t >= window_start).unwrap_or(false);

        if was_recently_superseded {
            recent_supersessions += 1;
            // Don't also count as a corroboration — once superseded, prior
            // corroborations are moot.
            continue;
        }

        // Count recent corroborations (edges created recently with corroboration > 1)
        if edge.created_at >= window_start && edge.corroboration_count >= 2 {
            recent_corroborations += 1;
        }
    }

    let total_recent = recent_supersessions + recent_corroborations;
    if total_recent == 0 {
        // No recent activity — stable (default high coherence)
        return (0.9, recent_supersessions, recent_corroborations);
    }

    // Score: ratio of corroborations to total recent activity
    // High corroborations = stable, consistent memory
    // High supersessions = volatile, contradictory memory
    let corroboration_ratio = recent_corroborations as f32 / total_recent as f32;

    // Scale: pure corroboration = 1.0, pure supersession = 0.2 (not 0 — supersessions
    // are natural and indicate the system is self-correcting)
    let score = 0.2 + corroboration_ratio * 0.8;
    (
        score.clamp(0.0, 1.0),
        recent_supersessions,
        recent_corroborations,
    )
}

// ─── Structural Coherence ──────────────────────────────────────────

/// Compute structural coherence based on graph connectivity.
///
/// Uses community structure: a well-connected graph has fewer communities
/// relative to its entity count. A highly fragmented graph (many tiny
/// disconnected components) is less coherent.
///
/// `community_map`: entity_id → community_id (from GraphEngine::detect_communities).
/// `total_entities`: total number of entities.
pub fn compute_structural_coherence(
    entities: &[Entity],
    edges: &[Edge],
    community_map: &HashMap<Uuid, Uuid>,
) -> (f32, usize, usize) {
    let total_entities = entities.len();
    if total_entities <= 1 {
        return (1.0, 0, 0); // trivially coherent
    }

    // Count unique communities
    let unique_communities: HashSet<&Uuid> = community_map.values().collect();
    let num_communities = unique_communities.len().max(1);

    // Count isolated entities (those with no edges at all)
    let connected_entities: HashSet<Uuid> = edges
        .iter()
        .filter(|e| e.is_valid())
        .flat_map(|e| [e.source_entity_id, e.target_entity_id])
        .collect();
    let isolated_entities = entities
        .iter()
        .filter(|e| !connected_entities.contains(&e.id))
        .count();

    // Ideal: few communities relative to entity count
    // community_ratio → 0 is best (one big connected component)
    // community_ratio → 1 is worst (every entity is its own community)
    let community_ratio = if total_entities > 0 {
        num_communities as f32 / total_entities as f32
    } else {
        0.0
    };

    // Isolation penalty: isolated entities reduce structural coherence
    let isolation_ratio = isolated_entities as f32 / total_entities as f32;

    // Score: inverse of fragmentation
    // community_ratio=0.05 (5% of entities are separate communities) → ~0.95
    // community_ratio=0.5 (half are separate) → ~0.5
    // community_ratio=1.0 (all separate) → ~0.2
    let connectivity_score = 1.0 - community_ratio * 0.8;
    let isolation_penalty = isolation_ratio * 0.4; // max 40% penalty

    let score = (connectivity_score - isolation_penalty).clamp(0.0, 1.0);
    (score, num_communities, isolated_entities)
}

// ─── Composite Score + Report ──────────────────────────────────────

/// Compute the full coherence report from entities, edges, and community structure.
pub fn compute_coherence_report(
    entities: &[Entity],
    edges: &[Edge],
    community_map: &HashMap<Uuid, Uuid>,
) -> CoherenceReport {
    let entity_coherence = compute_entity_coherence(entities, edges);
    let (fact_coherence, conflicting_groups) = compute_fact_coherence(edges);
    let (temporal_coherence, recent_supersessions, recent_corroborations) =
        compute_temporal_coherence(edges, 30);
    let (structural_coherence, communities_detected, isolated_entities) =
        compute_structural_coherence(entities, edges, community_map);

    let active_edges = edges.iter().filter(|e| e.is_valid()).count();
    let invalidated_edges = edges.len() - active_edges;

    let composite = W_ENTITY * entity_coherence
        + W_FACT * fact_coherence
        + W_TEMPORAL * temporal_coherence
        + W_STRUCTURAL * structural_coherence;

    let mut recommendations = Vec::new();
    if fact_coherence < 0.7 {
        recommendations.push(format!(
            "Resolve {} conflicting fact groups to improve consistency",
            conflicting_groups
        ));
    }
    if structural_coherence < 0.6 {
        recommendations.push(format!(
            "Graph is fragmented into {} communities with {} isolated entities — consider enriching connections",
            communities_detected, isolated_entities
        ));
    }
    if temporal_coherence < 0.5 {
        recommendations.push(format!(
            "High churn: {} supersessions vs {} corroborations in last 30 days",
            recent_supersessions, recent_corroborations
        ));
    }
    if entity_coherence < 0.6 {
        recommendations
            .push("Many low-confidence or weakly evidenced entity connections".to_string());
    }
    if composite >= 0.8 && recommendations.is_empty() {
        recommendations.push("Knowledge graph is healthy and internally consistent".to_string());
    }

    CoherenceReport {
        score: composite.clamp(0.0, 1.0),
        entity_coherence,
        fact_coherence,
        temporal_coherence,
        structural_coherence,
        recommendations,
        diagnostics: CoherenceDiagnostics {
            total_entities: entities.len(),
            total_edges: edges.len(),
            active_edges,
            invalidated_edges,
            conflicting_groups,
            communities_detected,
            isolated_entities,
            recent_supersessions,
            recent_corroborations,
        },
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use mnemo_core::models::entity::EntityType;

    fn make_entity(name: &str, entity_type: EntityType, mentions: u64) -> Entity {
        Entity {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            name: name.into(),
            entity_type,
            summary: None,
            aliases: vec![],
            metadata: serde_json::json!({}),
            classification: mnemo_core::models::classification::Classification::default(),
            mention_count: mentions,
            community_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_edge(
        src: Uuid,
        tgt: Uuid,
        label: &str,
        confidence: f32,
        corroboration: u32,
        age_days: i64,
        invalidated: bool,
    ) -> Edge {
        let created = Utc::now() - Duration::days(age_days);
        Edge {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            source_entity_id: src,
            target_entity_id: tgt,
            label: label.into(),
            fact: format!("{} {} something", label, src),
            valid_at: created,
            invalid_at: if invalidated {
                Some(Utc::now() - Duration::days(age_days.saturating_sub(1).max(0)))
            } else {
                None
            },
            ingested_at: created,
            source_episode_id: Uuid::now_v7(),
            invalidated_by_episode_id: if invalidated {
                Some(Uuid::now_v7())
            } else {
                None
            },
            confidence,
            corroboration_count: corroboration,
            metadata: serde_json::json!({}),
            classification: mnemo_core::models::classification::Classification::default(),
            created_at: created,
            updated_at: Utc::now(),
        }
    }

    // ─── Entity Coherence ──────────────────────────────────────

    #[test]
    fn test_entity_coherence_empty() {
        assert!((compute_entity_coherence(&[], &[]) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_entity_coherence_high_quality_connections() {
        let alice = make_entity("Alice", EntityType::Person, 5);
        let acme = make_entity("Acme Corp", EntityType::Organization, 3);
        let edge = make_edge(alice.id, acme.id, "works_at", 0.9, 3, 5, false);

        let score = compute_entity_coherence(&[alice, acme], &[edge]);
        assert!(
            score > 0.7,
            "high-quality connection should score well: {}",
            score
        );
    }

    #[test]
    fn test_entity_coherence_penalizes_weak_entities() {
        let entities: Vec<Entity> = (0..10)
            .map(|i| make_entity(&format!("Entity{}", i), EntityType::Concept, 1))
            .collect();
        let edge = make_edge(entities[0].id, entities[1].id, "related", 0.8, 1, 2, false);

        let score = compute_entity_coherence(&entities, &[edge]);
        // Should be penalized because 100% of entities have mention_count=1
        assert!(score < 0.9, "weak entities should reduce score: {}", score);
    }

    #[test]
    fn test_entity_coherence_no_active_edges() {
        let e1 = make_entity("Alice", EntityType::Person, 5);
        let e2 = make_entity("Bob", EntityType::Person, 3);
        let edge = make_edge(e1.id, e2.id, "knows", 0.8, 1, 5, true); // invalidated

        let score = compute_entity_coherence(&[e1, e2], &[edge]);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "no active edges = vacuously coherent"
        );
    }

    // ─── Fact Coherence ────────────────────────────────────────

    #[test]
    fn test_fact_coherence_no_conflicts() {
        let src = Uuid::now_v7();
        let tgt1 = Uuid::now_v7();
        let tgt2 = Uuid::now_v7();

        let edges = vec![
            make_edge(src, tgt1, "works_at", 0.9, 2, 5, false),
            make_edge(src, tgt2, "lives_in", 0.8, 1, 3, false),
        ];

        let (score, conflicts) = compute_fact_coherence(&edges);
        assert!((score - 1.0).abs() < f32::EPSILON, "no conflicts = 1.0");
        assert_eq!(conflicts, 0);
    }

    #[test]
    fn test_fact_coherence_with_conflict() {
        let src = Uuid::now_v7();
        let tgt1 = Uuid::now_v7();
        let tgt2 = Uuid::now_v7();

        // Two active edges with same source + label = conflict
        let edges = vec![
            make_edge(src, tgt1, "works_at", 0.9, 2, 5, false),
            make_edge(src, tgt2, "works_at", 0.7, 1, 2, false),
        ];

        let (score, conflicts) = compute_fact_coherence(&edges);
        assert!(score < 1.0, "conflicts should reduce score: {}", score);
        assert_eq!(conflicts, 1);
    }

    #[test]
    fn test_fact_coherence_resolved_conflict_ok() {
        let src = Uuid::now_v7();
        let tgt1 = Uuid::now_v7();
        let tgt2 = Uuid::now_v7();

        // One invalidated, one active = no conflict
        let edges = vec![
            make_edge(src, tgt1, "works_at", 0.9, 2, 30, true),
            make_edge(src, tgt2, "works_at", 0.8, 1, 5, false),
        ];

        let (score, conflicts) = compute_fact_coherence(&edges);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "resolved conflict should be clean"
        );
        assert_eq!(conflicts, 0);
    }

    #[test]
    fn test_fact_coherence_empty() {
        let (score, conflicts) = compute_fact_coherence(&[]);
        assert!((score - 1.0).abs() < f32::EPSILON);
        assert_eq!(conflicts, 0);
    }

    // ─── Temporal Coherence ────────────────────────────────────

    #[test]
    fn test_temporal_coherence_stable_graph() {
        // All edges are old, no recent activity
        let src = Uuid::now_v7();
        let tgt = Uuid::now_v7();
        let edges = vec![make_edge(src, tgt, "knows", 0.9, 3, 60, false)];

        let (score, sups, corrs) = compute_temporal_coherence(&edges, 30);
        assert!(
            score >= 0.8,
            "stable graph should have high temporal coherence: {}",
            score
        );
        assert_eq!(sups, 0);
        assert_eq!(corrs, 0);
    }

    #[test]
    fn test_temporal_coherence_high_churn() {
        let src = Uuid::now_v7();
        let tgt1 = Uuid::now_v7();
        let tgt2 = Uuid::now_v7();
        let tgt3 = Uuid::now_v7();

        // Many recent supersessions, no corroborations
        let edges = vec![
            make_edge(src, tgt1, "works_at", 0.8, 1, 5, true),
            make_edge(src, tgt2, "works_at", 0.7, 1, 3, true),
            make_edge(src, tgt3, "works_at", 0.9, 1, 1, false),
        ];

        let (score, sups, _) = compute_temporal_coherence(&edges, 30);
        assert!(
            score < 0.7,
            "high churn should reduce temporal coherence: {}",
            score
        );
        assert!(sups >= 2, "should have recent supersessions: {}", sups);
    }

    #[test]
    fn test_temporal_coherence_lots_of_corroborations() {
        let src = Uuid::now_v7();
        let tgt1 = Uuid::now_v7();
        let tgt2 = Uuid::now_v7();

        let edges = vec![
            make_edge(src, tgt1, "knows", 0.9, 5, 5, false),
            make_edge(src, tgt2, "likes", 0.8, 3, 2, false),
        ];

        let (score, sups, corrs) = compute_temporal_coherence(&edges, 30);
        assert!(
            score > 0.8,
            "corroborations should boost temporal coherence: {}",
            score
        );
        assert_eq!(sups, 0);
        assert!(corrs >= 2, "should have corroborations: {}", corrs);
    }

    // ─── Structural Coherence ──────────────────────────────────

    #[test]
    fn test_structural_coherence_one_community() {
        let e1 = make_entity("Alice", EntityType::Person, 5);
        let e2 = make_entity("Bob", EntityType::Person, 3);
        let e3 = make_entity("Carol", EntityType::Person, 4);
        let entities = vec![e1.clone(), e2.clone(), e3.clone()];

        let community_id = Uuid::now_v7();
        let mut community_map = HashMap::new();
        community_map.insert(e1.id, community_id);
        community_map.insert(e2.id, community_id);
        community_map.insert(e3.id, community_id);

        let edges = vec![
            make_edge(e1.id, e2.id, "knows", 0.9, 2, 5, false),
            make_edge(e2.id, e3.id, "knows", 0.8, 1, 3, false),
        ];

        let (score, communities, isolated) =
            compute_structural_coherence(&entities, &edges, &community_map);
        assert!(
            score > 0.7,
            "one community should have high structural coherence: {}",
            score
        );
        assert_eq!(communities, 1);
        assert_eq!(isolated, 0);
    }

    #[test]
    fn test_structural_coherence_fragmented() {
        let entities: Vec<Entity> = (0..10)
            .map(|i| make_entity(&format!("E{}", i), EntityType::Concept, 2))
            .collect();

        // Every entity is its own community (fully disconnected)
        let mut community_map = HashMap::new();
        for e in &entities {
            community_map.insert(e.id, Uuid::now_v7()); // unique community per entity
        }

        let (score, communities, isolated) =
            compute_structural_coherence(&entities, &[], &community_map);
        assert!(
            score < 0.5,
            "fully fragmented should have low structural coherence: {}",
            score
        );
        assert_eq!(communities, 10);
        assert_eq!(isolated, 10);
    }

    #[test]
    fn test_structural_coherence_empty() {
        let (score, _, _) = compute_structural_coherence(&[], &[], &HashMap::new());
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    // ─── Composite Report ──────────────────────────────────────

    #[test]
    fn test_coherence_report_empty_graph() {
        let report = compute_coherence_report(&[], &[], &HashMap::new());
        assert!(
            report.score >= 0.9,
            "empty graph should be vacuously coherent: {}",
            report.score
        );
        assert!(report.recommendations.iter().any(|r| r.contains("healthy")));
    }

    #[test]
    fn test_coherence_report_healthy_graph() {
        let alice = make_entity("Alice", EntityType::Person, 5);
        let acme = make_entity("Acme Corp", EntityType::Organization, 3);
        let nyc = make_entity("New York", EntityType::Location, 4);

        let community_id = Uuid::now_v7();
        let mut community_map = HashMap::new();
        community_map.insert(alice.id, community_id);
        community_map.insert(acme.id, community_id);
        community_map.insert(nyc.id, community_id);

        let edges = vec![
            make_edge(alice.id, acme.id, "works_at", 0.9, 3, 5, false),
            make_edge(acme.id, nyc.id, "located_in", 0.95, 2, 10, false),
        ];

        let report = compute_coherence_report(&[alice, acme, nyc], &edges, &community_map);
        assert!(
            report.score > 0.7,
            "healthy graph should score well: {}",
            report.score
        );
        assert_eq!(report.diagnostics.total_entities, 3);
        assert_eq!(report.diagnostics.active_edges, 2);
        assert_eq!(report.diagnostics.conflicting_groups, 0);
    }

    #[test]
    fn test_coherence_report_conflicted_graph() {
        let alice = make_entity("Alice", EntityType::Person, 5);
        let acme = make_entity("Acme Corp", EntityType::Organization, 3);
        let globex = make_entity("Globex Corp", EntityType::Organization, 2);

        let community_id = Uuid::now_v7();
        let mut community_map = HashMap::new();
        community_map.insert(alice.id, community_id);
        community_map.insert(acme.id, community_id);
        community_map.insert(globex.id, community_id);

        // Conflict: Alice works_at both Acme and Globex simultaneously
        let edges = vec![
            make_edge(alice.id, acme.id, "works_at", 0.9, 2, 5, false),
            make_edge(alice.id, globex.id, "works_at", 0.7, 1, 2, false),
        ];

        let report = compute_coherence_report(&[alice, acme, globex], &edges, &community_map);
        assert!(
            report.fact_coherence < 1.0,
            "conflicted graph should have lower fact coherence"
        );
        assert_eq!(report.diagnostics.conflicting_groups, 1);
        assert!(report
            .recommendations
            .iter()
            .any(|r| r.contains("conflicting")));
    }

    #[test]
    fn test_coherence_report_score_bounded() {
        let entities: Vec<Entity> = (0..20)
            .map(|i| make_entity(&format!("E{}", i), EntityType::Concept, 1))
            .collect();

        let mut community_map = HashMap::new();
        for e in &entities {
            community_map.insert(e.id, Uuid::now_v7());
        }

        // Worst case: all isolated, all conflicting, high churn
        let edges: Vec<Edge> = (0..10)
            .flat_map(|i| {
                vec![
                    make_edge(
                        entities[0].id,
                        entities[i + 1].id,
                        "same_label",
                        0.3,
                        1,
                        3,
                        true,
                    ),
                    make_edge(
                        entities[0].id,
                        entities[i + 1].id,
                        "same_label",
                        0.2,
                        1,
                        1,
                        false,
                    ),
                ]
            })
            .collect();

        let report = compute_coherence_report(&entities, &edges, &community_map);
        assert!(
            report.score >= 0.0 && report.score <= 1.0,
            "score must be in [0,1]: {}",
            report.score
        );
    }

    #[test]
    fn test_coherence_weights_sum_to_1() {
        let total = W_ENTITY + W_FACT + W_TEMPORAL + W_STRUCTURAL;
        assert!(
            (total - 1.0).abs() < f32::EPSILON,
            "weights must sum to 1.0: {}",
            total
        );
    }

    // ═══════════════════════════════════════════════════════════
    // Falsification tests — adversarial / boundary cases
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn falsify_temporal_no_double_count_superseded_corroborated_edge() {
        // Bug 6: An edge that was corroborated AND THEN superseded should count
        // as a supersession only — not inflate both counters.
        let src = Uuid::now_v7();
        let tgt = Uuid::now_v7();

        // Edge created 5 days ago, corroborated 3 times, then invalidated 2 days ago
        let mut edge = make_edge(src, tgt, "works_at", 0.9, 3, 5, false);
        edge.invalid_at = Some(Utc::now() - Duration::days(2));

        let (score, sups, corrs) = compute_temporal_coherence(&[edge], 30);
        // Must count as supersession only, not also corroboration
        assert_eq!(sups, 1, "should be 1 supersession");
        assert_eq!(corrs, 0, "should NOT also count as corroboration");
        // Pure supersession => score should be low (0.2 + 0*0.8 = 0.2)
        assert!(
            (score - 0.2).abs() < 0.01,
            "pure supersession score should be ~0.2: {}",
            score
        );
    }

    #[test]
    fn falsify_entity_coherence_orphaned_edges_are_incoherent() {
        // Bug 2: Active edges referencing entities NOT in the entity list
        // should lower coherence, not be silently skipped.
        let alice = make_entity("Alice", EntityType::Person, 5);
        let ghost_id = Uuid::now_v7(); // not in entity list

        // Edge from Alice to a ghost entity
        let edge = make_edge(alice.id, ghost_id, "knows", 0.9, 3, 5, false);

        let score = compute_entity_coherence(&[alice], &[edge]);
        // Ghost entity means the edge is orphaned — should count as incoherent
        assert!(
            score < 1.0,
            "orphaned edge should reduce coherence, got: {}",
            score
        );
    }

    #[test]
    fn falsify_entity_coherence_all_orphaned_edges() {
        // All edges reference entities not in the list — should be 0 coherence
        let e1 = make_entity("Alice", EntityType::Person, 5);
        let ghost1 = Uuid::now_v7();
        let ghost2 = Uuid::now_v7();

        let edges = vec![
            make_edge(ghost1, ghost2, "knows", 0.9, 3, 5, false),
            make_edge(ghost1, e1.id, "likes", 0.8, 2, 3, false),
        ];

        let score = compute_entity_coherence(&[e1], &edges);
        // edge 1: both missing → orphaned, edge 2: source missing → orphaned
        // total_pairs=2, coherent_pairs=0 → base_score=0.0
        assert!(
            score < 0.1,
            "all orphaned edges should yield near-zero coherence: {}",
            score
        );
    }

    #[test]
    fn falsify_structural_coherence_empty_community_map_with_entities() {
        // If detect_communities returns empty map, structural coherence should
        // still work and reflect that we can't determine community structure.
        let entities: Vec<Entity> = (0..5)
            .map(|i| make_entity(&format!("E{}", i), EntityType::Concept, 3))
            .collect();
        let edges = vec![make_edge(
            entities[0].id,
            entities[1].id,
            "related",
            0.8,
            2,
            5,
            false,
        )];

        let (score, communities, isolated) =
            compute_structural_coherence(&entities, &edges, &HashMap::new());
        // Empty map → num_communities = max(0,1) = 1
        // community_ratio = 1/5 = 0.2, connectivity_score = 1.0 - 0.16 = 0.84
        // 3 isolated entities (E2, E3, E4), isolation_ratio = 3/5 = 0.6
        // isolation_penalty = 0.6 * 0.4 = 0.24
        // score = 0.84 - 0.24 = 0.60
        assert!(
            (0.0..=1.0).contains(&score),
            "score must be bounded: {}",
            score
        );
        assert_eq!(communities, 1, "empty map should yield 1 pseudo-community");
        assert_eq!(isolated, 3, "3 entities with no edges should be isolated");
    }

    #[test]
    fn falsify_fact_coherence_all_invalidated_same_group() {
        // Many invalidated edges in same group, zero active → should be clean (not conflicting)
        let src = Uuid::now_v7();
        let edges: Vec<Edge> = (0..10)
            .map(|i| make_edge(src, Uuid::now_v7(), "works_at", 0.5, 1, i + 1, true))
            .collect();

        let (score, conflicts) = compute_fact_coherence(&edges);
        assert_eq!(conflicts, 0, "all invalidated = no active conflict");
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "all invalidated should be clean: {}",
            score
        );
    }

    #[test]
    fn falsify_composite_score_bounded_under_worst_case() {
        // Construct the absolute worst case: all dimensions should be at minimum.
        // Entity: all low-confidence same-type edges with weak entities
        // Fact: all conflicting
        // Temporal: all supersessions
        // Structural: fully fragmented
        let entities: Vec<Entity> = (0..10)
            .map(|i| make_entity(&format!("E{}", i), EntityType::Concept, 1))
            .collect();

        // Each entity pair has same label → conflicts
        let mut edges = Vec::new();
        for i in 0..5 {
            // Two active edges per (source, label) = conflict
            edges.push(make_edge(
                entities[0].id,
                entities[i + 1].id,
                "related",
                0.2,
                1,
                2,
                false,
            ));
        }
        // Add superseded edges for temporal churn
        for i in 0..5 {
            let mut e = make_edge(
                entities[0].id,
                entities[i + 5].id,
                "other",
                0.3,
                1,
                3,
                false,
            );
            e.invalid_at = Some(Utc::now() - Duration::days(1));
            edges.push(e);
        }

        let mut community_map = HashMap::new();
        for e in &entities {
            community_map.insert(e.id, Uuid::now_v7()); // each entity is its own community
        }

        let report = compute_coherence_report(&entities, &edges, &community_map);
        assert!(
            report.score >= 0.0,
            "composite must be >= 0.0: {}",
            report.score
        );
        assert!(
            report.score <= 1.0,
            "composite must be <= 1.0: {}",
            report.score
        );
        // Under worst case, score should be genuinely low (below 0.5)
        assert!(
            report.score < 0.5,
            "worst-case graph should score low: {}",
            report.score
        );
        // Should have recommendations
        assert!(
            !report.recommendations.is_empty(),
            "should have recommendations for a bad graph"
        );
    }

    #[test]
    fn falsify_temporal_window_zero_days() {
        // Edge case: window_days = 0 should mean "right now only"
        let src = Uuid::now_v7();
        let tgt = Uuid::now_v7();
        let edges = vec![make_edge(src, tgt, "knows", 0.9, 3, 1, false)];

        let (score, sups, corrs) = compute_temporal_coherence(&edges, 0);
        // A 0-day window means window_start = now. Edge created 1 day ago is outside window.
        assert_eq!(sups, 0);
        assert_eq!(corrs, 0);
        assert!(
            (score - 0.9).abs() < f32::EPSILON,
            "no recent activity should return 0.9: {}",
            score
        );
    }

    #[test]
    fn falsify_entity_coherence_same_type_low_confidence() {
        // Same entity type + low confidence (<0.5) + low corroboration (1) → incoherent
        let e1 = make_entity("A", EntityType::Person, 5);
        let e2 = make_entity("B", EntityType::Person, 5);
        let edge = make_edge(e1.id, e2.id, "knows", 0.3, 1, 5, false);

        let score = compute_entity_coherence(&[e1, e2], &[edge]);
        // Same type (not diverse), low confidence (<0.5), low corroboration (<2),
        // confidence < 0.7 → NOT coherent. Score = 0/1 = 0.0
        assert!(
            score < 0.1,
            "same-type low-confidence should be incoherent: {}",
            score
        );
    }
}
