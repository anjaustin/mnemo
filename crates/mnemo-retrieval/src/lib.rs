//! # mnemo-retrieval
//!
//! Hybrid retrieval engine: semantic search + full-text search + graph traversal.
//! Results are merged using Reciprocal Rank Fusion (RRF).

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use mnemo_core::models::context::*;
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingProvider;
use mnemo_core::traits::storage::*;

/// Reciprocal Rank Fusion constant. Standard value from the RRF paper.
const RRF_K: f64 = 60.0;

pub struct RetrievalEngine<S, V, E>
where
    S: EntityStore + EdgeStore + EpisodeStore + FullTextStore,
    V: VectorStore,
    E: EmbeddingProvider,
{
    state_store: Arc<S>,
    vector_store: Arc<V>,
    embedder: Arc<E>,
}

/// A scored result from a single retrieval source.
struct ScoredHit {
    id: Uuid,
}

impl<S, V, E> RetrievalEngine<S, V, E>
where
    S: EntityStore + EdgeStore + EpisodeStore + FullTextStore + Send + Sync + 'static,
    V: VectorStore + Send + Sync + 'static,
    E: EmbeddingProvider + Send + Sync + 'static,
{
    pub fn new(state_store: Arc<S>, vector_store: Arc<V>, embedder: Arc<E>) -> Self {
        Self {
            state_store,
            vector_store,
            embedder,
        }
    }

    /// Main entry point: hybrid retrieval + RRF fusion + context assembly.
    pub async fn get_context(
        &self,
        user_id: Uuid,
        request: &ContextRequest,
    ) -> StorageResult<ContextBlock> {
        let start = Instant::now();
        let mut block = ContextBlock::empty();

        let query_text = request
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        if query_text.trim().is_empty() {
            block.latency_ms = start.elapsed().as_millis() as u64;
            return Ok(block);
        }

        let temporal_intent = resolve_temporal_intent(request.time_intent, &query_text);
        let temporal_filter = request.as_of.or(request.temporal_filter);

        // Generate query embedding for semantic search.
        // If embeddings are unavailable, gracefully degrade to full-text retrieval.
        let query_embedding = match self.embedder.embed(&query_text).await {
            Ok(embedding) => Some(embedding),
            Err(err) => {
                tracing::warn!(
                    user_id = %user_id,
                    error = %err,
                    "Embedding unavailable, falling back to full-text retrieval"
                );
                None
            }
        };

        // ── Parallel search: semantic + full-text ──────────────
        // Semantic search
        let (semantic_entity_hits, semantic_edge_hits, semantic_episode_hits) =
            if let Some(query_embedding) = query_embedding {
                let entity_hits = self
                    .vector_store
                    .search_entities(user_id, query_embedding.clone(), 10, request.min_relevance)
                    .await?;
                let edge_hits = self
                    .vector_store
                    .search_edges(user_id, query_embedding.clone(), 15, request.min_relevance)
                    .await?;
                let episode_hits = self
                    .vector_store
                    .search_episodes(user_id, query_embedding, 5, request.min_relevance)
                    .await?;
                (entity_hits, edge_hits, episode_hits)
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };

        // Full-text search
        let ft_entity_hits = self
            .state_store
            .search_entities_ft(user_id, &query_text, 10)
            .await
            .unwrap_or_default();
        let ft_edge_hits = self
            .state_store
            .search_edges_ft(user_id, &query_text, 15)
            .await
            .unwrap_or_default();
        let ft_episode_hits = self
            .state_store
            .search_episodes_ft(user_id, &query_text, 5)
            .await
            .unwrap_or_default();

        // ── RRF fusion for entities ────────────────────────────
        let entity_ids = rrf_merge(vec![
            ranked_hits(&semantic_entity_hits, RetrievalSource::SemanticSearch),
            ranked_hits(&ft_entity_hits, RetrievalSource::FullTextSearch),
        ]);

        for (entity_id, rrf_score) in entity_ids.iter().take(10) {
            if let Ok(entity) = self.state_store.get_entity(*entity_id).await {
                block.entities.push(EntitySummary {
                    id: entity.id,
                    name: entity.name.clone(),
                    entity_type: entity.entity_type.as_str().to_string(),
                    summary: entity.summary.clone(),
                    relevance: *rrf_score as f32,
                });
            }
        }

        // ── RRF fusion for edges ───────────────────────────────
        let edge_ids = rrf_merge(vec![
            ranked_hits(&semantic_edge_hits, RetrievalSource::SemanticSearch),
            ranked_hits(&ft_edge_hits, RetrievalSource::FullTextSearch),
        ]);

        for (edge_id, rrf_score) in edge_ids.iter().take(15) {
            if let Ok(edge) = self.state_store.get_edge(*edge_id).await {
                if let Some(tf) = temporal_filter {
                    if !edge.is_valid_at(tf) {
                        continue;
                    }
                } else if !edge.is_valid() {
                    continue;
                }

                let src_name = self
                    .state_store
                    .get_entity(edge.source_entity_id)
                    .await
                    .map(|e| e.name)
                    .unwrap_or_else(|_| "Unknown".to_string());
                let tgt_name = self
                    .state_store
                    .get_entity(edge.target_entity_id)
                    .await
                    .map(|e| e.name)
                    .unwrap_or_else(|_| "Unknown".to_string());

                block.facts.push(FactSummary {
                    id: edge.id,
                    source_entity: src_name,
                    target_entity: tgt_name,
                    label: edge.label,
                    fact: edge.fact,
                    valid_at: edge.valid_at,
                    invalid_at: edge.invalid_at,
                    relevance: *rrf_score as f32,
                });
            }
        }

        // ── Graph traversal for top entities ───────────────────
        for entity_summary in block.entities.iter().take(3) {
            let outgoing = self
                .state_store
                .get_outgoing_edges(entity_summary.id)
                .await?;
            for edge in outgoing {
                if !edge.is_valid() {
                    continue;
                }
                if block.facts.iter().any(|f| f.id == edge.id) {
                    continue;
                }

                let tgt_name = self
                    .state_store
                    .get_entity(edge.target_entity_id)
                    .await
                    .map(|e| e.name)
                    .unwrap_or_else(|_| "Unknown".to_string());

                block.facts.push(FactSummary {
                    id: edge.id,
                    source_entity: entity_summary.name.clone(),
                    target_entity: tgt_name,
                    label: edge.label,
                    fact: edge.fact,
                    valid_at: edge.valid_at,
                    invalid_at: edge.invalid_at,
                    relevance: entity_summary.relevance * 0.8,
                });
            }
        }

        // ── RRF fusion for episodes ────────────────────────────
        let episode_ids = rrf_merge(vec![
            ranked_hits(&semantic_episode_hits, RetrievalSource::SemanticSearch),
            ranked_hits(&ft_episode_hits, RetrievalSource::FullTextSearch),
        ]);

        for (episode_id, rrf_score) in episode_ids.iter().take(5) {
            if let Ok(ep) = self.state_store.get_episode(*episode_id).await {
                let preview = if ep.content.len() > 200 {
                    format!("{}...", &ep.content[..200])
                } else {
                    ep.content.clone()
                };
                block.episodes.push(EpisodeSummary {
                    id: ep.id,
                    session_id: ep.session_id,
                    role: ep.role.map(|r| format!("{:?}", r).to_lowercase()),
                    preview,
                    created_at: ep.created_at,
                    relevance: *rrf_score as f32,
                });
            }
        }

        // ── Track retrieval sources ────────────────────────────
        if !semantic_entity_hits.is_empty() || !semantic_edge_hits.is_empty() {
            block.sources.push(RetrievalSource::SemanticSearch);
        }
        if !ft_entity_hits.is_empty() || !ft_edge_hits.is_empty() {
            block.sources.push(RetrievalSource::FullTextSearch);
        }
        if block.facts.iter().any(|f| f.relevance < 1.0) {
            block.sources.push(RetrievalSource::GraphTraversal);
        }

        // ── Temporal reranking (semantic + time) ──────────────
        apply_temporal_scoring(
            &mut block,
            temporal_intent,
            temporal_filter,
            request.temporal_weight,
        );
        if !block.sources.contains(&RetrievalSource::TemporalScoring) {
            block.sources.push(RetrievalSource::TemporalScoring);
        }

        // ── Assemble context string ────────────────────────────
        block.assemble(request.max_tokens);
        block.latency_ms = start.elapsed().as_millis() as u64;

        tracing::debug!(
            user_id = %user_id,
            entities = block.entities.len(),
            facts = block.facts.len(),
            episodes = block.episodes.len(),
            tokens = block.token_count,
            latency_ms = block.latency_ms,
            "Context assembled"
        );

        Ok(block)
    }
}

/// Convert (Uuid, f32) hits into ranked ScoredHit lists.
fn ranked_hits(hits: &[(Uuid, f32)], source: RetrievalSource) -> Vec<ScoredHit> {
    let _ = source;
    hits.iter()
        .map(|(id, _score)| ScoredHit { id: *id })
        .collect()
}

/// Reciprocal Rank Fusion: merge multiple ranked lists into a single ranking.
///
/// RRF_score(item) = Σ 1 / (k + rank_in_source)
///
/// Returns items sorted by RRF score (highest first).
fn rrf_merge(sources: Vec<Vec<ScoredHit>>) -> Vec<(Uuid, f64)> {
    let mut scores: HashMap<Uuid, f64> = HashMap::new();

    for source_hits in &sources {
        for (rank, hit) in source_hits.iter().enumerate() {
            let rrf_contrib = 1.0 / (RRF_K + rank as f64 + 1.0);
            *scores.entry(hit.id).or_default() += rrf_contrib;
        }
    }

    let mut sorted: Vec<(Uuid, f64)> = scores.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

fn resolve_temporal_intent(intent: TemporalIntent, query_text: &str) -> TemporalIntent {
    if intent != TemporalIntent::Auto {
        return intent;
    }

    let q = query_text.to_lowercase();
    if contains_year_hint(&q) {
        return TemporalIntent::Historical;
    }
    if q.contains("as of")
        || q.contains("back in")
        || q.contains("histor")
        || q.contains("before")
        || q.contains("during")
        || q.contains("at that time")
    {
        return TemporalIntent::Historical;
    }
    if q.contains("recent")
        || q.contains("lately")
        || q.contains("today")
        || q.contains("yesterday")
        || q.contains("last week")
        || q.contains("this month")
    {
        return TemporalIntent::Recent;
    }
    if q.contains("currently")
        || q.contains("right now")
        || q.contains("now")
        || q.contains("latest")
    {
        return TemporalIntent::Current;
    }

    TemporalIntent::Current
}

fn apply_temporal_scoring(
    block: &mut ContextBlock,
    temporal_intent: TemporalIntent,
    temporal_filter: Option<DateTime<Utc>>,
    temporal_weight: Option<f32>,
) {
    let now = Utc::now();
    let weight = temporal_weight
        .unwrap_or_else(|| default_temporal_weight(temporal_intent))
        .clamp(0.0, 1.0) as f64;

    for fact in &mut block.facts {
        let temporal_score = score_fact_temporal(fact, temporal_intent, temporal_filter, now);
        fact.relevance = apply_temporal_blend(fact.relevance as f64, temporal_score, weight) as f32;
    }

    for episode in &mut block.episodes {
        let temporal_score = score_episode_temporal(episode, temporal_intent, temporal_filter, now);
        episode.relevance =
            apply_temporal_blend(episode.relevance as f64, temporal_score, weight) as f32;
    }

    for entity in &mut block.entities {
        entity.relevance = apply_temporal_blend(entity.relevance as f64, 0.6, weight * 0.4) as f32;
    }

    block.temporal_diagnostics = Some(TemporalDiagnostics {
        resolved_intent: temporal_intent,
        temporal_weight: weight as f32,
        as_of: temporal_filter,
        entities_scored: block.entities.len() as u32,
        facts_scored: block.facts.len() as u32,
        episodes_scored: block.episodes.len() as u32,
    });

    block.entities.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    block.facts.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    block.episodes.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn contains_year_hint(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token.len() == 4 && token.starts_with("20") && token.parse::<u16>().is_ok())
}

fn default_temporal_weight(intent: TemporalIntent) -> f32 {
    match intent {
        TemporalIntent::Auto => 0.35,
        TemporalIntent::Current => 0.35,
        TemporalIntent::Recent => 0.45,
        TemporalIntent::Historical => 0.55,
    }
}

fn apply_temporal_blend(base_score: f64, temporal_score: f64, temporal_weight: f64) -> f64 {
    let multiplier = (1.0 + temporal_weight * (temporal_score - 0.5) * 1.6).clamp(0.2, 2.0);
    (base_score * multiplier).max(0.0)
}

fn score_fact_temporal(
    fact: &FactSummary,
    temporal_intent: TemporalIntent,
    temporal_filter: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> f64 {
    match temporal_intent {
        TemporalIntent::Historical => {
            if let Some(as_of) = temporal_filter {
                let valid_at_as_of =
                    fact.valid_at <= as_of && fact.invalid_at.map(|x| x > as_of).unwrap_or(true);
                if valid_at_as_of {
                    0.95
                } else {
                    0.2
                }
            } else {
                0.6
            }
        }
        TemporalIntent::Recent => {
            let age_days = (now - fact.valid_at).num_days().max(0) as f64;
            ((-age_days / 30.0).exp()).clamp(0.0, 1.0)
        }
        TemporalIntent::Current | TemporalIntent::Auto => {
            if fact.invalid_at.is_none() {
                let age_days = (now - fact.valid_at).num_days().max(0) as f64;
                (0.6 + 0.4 * (-age_days / 120.0).exp()).clamp(0.0, 1.0)
            } else {
                0.2
            }
        }
    }
}

fn score_episode_temporal(
    episode: &EpisodeSummary,
    temporal_intent: TemporalIntent,
    temporal_filter: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> f64 {
    match temporal_intent {
        TemporalIntent::Historical => {
            if let Some(as_of) = temporal_filter {
                let delta = (episode.created_at - as_of).num_days().unsigned_abs() as f64;
                (-delta / 14.0).exp().clamp(0.0, 1.0)
            } else {
                0.5
            }
        }
        TemporalIntent::Recent | TemporalIntent::Current | TemporalIntent::Auto => {
            let age_days = (now - episode.created_at).num_days().max(0) as f64;
            (-age_days / 21.0).exp().clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_rrf_merge_single_source() {
        let id1 = Uuid::now_v7();
        let id2 = Uuid::now_v7();
        let source = vec![ScoredHit { id: id1 }, ScoredHit { id: id2 }];
        let result = rrf_merge(vec![source]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, id1); // Higher ranked item first
    }

    #[test]
    fn test_rrf_merge_boost_overlapping() {
        let shared_id = Uuid::now_v7();
        let only_semantic = Uuid::now_v7();
        let only_ft = Uuid::now_v7();

        let semantic = vec![ScoredHit { id: shared_id }, ScoredHit { id: only_semantic }];
        let ft = vec![ScoredHit { id: shared_id }, ScoredHit { id: only_ft }];

        let result = rrf_merge(vec![semantic, ft]);
        // shared_id should be ranked #1 because it appears in both sources
        assert_eq!(result[0].0, shared_id);
        assert!(result[0].1 > result[1].1); // Higher RRF score
    }

    #[test]
    fn test_rrf_empty_sources() {
        let result = rrf_merge(vec![vec![], vec![]]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_temporal_intent_from_query() {
        assert_eq!(
            resolve_temporal_intent(TemporalIntent::Auto, "What do I currently prefer?"),
            TemporalIntent::Current
        );
        assert_eq!(
            resolve_temporal_intent(TemporalIntent::Auto, "What changed recently?"),
            TemporalIntent::Recent
        );
        assert_eq!(
            resolve_temporal_intent(TemporalIntent::Auto, "As of 2024 what did I prefer?"),
            TemporalIntent::Historical
        );
        assert_eq!(
            resolve_temporal_intent(TemporalIntent::Auto, "What happened in 2023?"),
            TemporalIntent::Historical
        );
    }

    #[test]
    fn test_temporal_scoring_prefers_current_fact_for_current_intent() {
        let now = Utc::now();
        let current_fact = FactSummary {
            id: Uuid::now_v7(),
            source_entity: "u".into(),
            target_entity: "x".into(),
            label: "prefers".into(),
            fact: "u prefers x".into(),
            valid_at: now - Duration::days(2),
            invalid_at: None,
            relevance: 0.01,
        };
        let stale_fact = FactSummary {
            id: Uuid::now_v7(),
            source_entity: "u".into(),
            target_entity: "y".into(),
            label: "prefers".into(),
            fact: "u preferred y".into(),
            valid_at: now - Duration::days(200),
            invalid_at: Some(now - Duration::days(20)),
            relevance: 0.01,
        };

        assert!(
            score_fact_temporal(&current_fact, TemporalIntent::Current, None, now)
                > score_fact_temporal(&stale_fact, TemporalIntent::Current, None, now)
        );
    }

    #[test]
    fn test_apply_temporal_scoring_emits_diagnostics() {
        let mut block = ContextBlock::empty();
        block.entities.push(EntitySummary {
            id: Uuid::now_v7(),
            name: "kendra".into(),
            entity_type: "person".into(),
            summary: None,
            relevance: 0.4,
        });

        apply_temporal_scoring(&mut block, TemporalIntent::Current, None, Some(0.5));
        let diag = block
            .temporal_diagnostics
            .expect("temporal diagnostics missing");
        assert_eq!(diag.resolved_intent, TemporalIntent::Current);
        assert_eq!(diag.temporal_weight, 0.5);
        assert_eq!(diag.entities_scored, 1);
        assert_eq!(diag.facts_scored, 0);
        assert_eq!(diag.episodes_scored, 0);
    }

    // =========================================================================
    // RET-08: Reranker diversity tests (RRF — MMR is not yet implemented)
    //
    // Note: The config mentions MMR as a future option (default.toml line 91:
    // `reranker = "rrf"`), but only RRF is implemented. These tests verify
    // the RRF reranker produces expected diversity behavior — items appearing
    // in multiple ranked lists are boosted, ensuring result diversity.
    // =========================================================================

    #[test]
    fn ret08_rrf_three_sources_boosts_consensus() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let c = Uuid::now_v7();

        // a appears in all 3 sources, b in 2, c in 1
        let s1 = vec![ScoredHit { id: a }, ScoredHit { id: b }];
        let s2 = vec![ScoredHit { id: a }, ScoredHit { id: c }];
        let s3 = vec![ScoredHit { id: a }, ScoredHit { id: b }];

        let result = rrf_merge(vec![s1, s2, s3]);
        assert_eq!(result[0].0, a, "a (in all 3 sources) should be #1");
        assert_eq!(result[1].0, b, "b (in 2 sources) should be #2");
        assert_eq!(result[2].0, c, "c (in 1 source) should be #3");
        // Score ordering should be strictly decreasing
        assert!(result[0].1 > result[1].1);
        assert!(result[1].1 > result[2].1);
    }

    #[test]
    fn ret08_rrf_rank_position_matters() {
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();

        // Both appear in 1 source each, but 'first' is rank 0 and 'second' is rank 1
        let s1 = vec![ScoredHit { id: first }, ScoredHit { id: second }];

        let result = rrf_merge(vec![s1]);
        assert_eq!(result[0].0, first);
        assert_eq!(result[1].0, second);
        // Rank 0 should have higher RRF score than rank 1
        let score_diff = result[0].1 - result[1].1;
        assert!(
            score_diff > 0.0,
            "Higher rank should have higher RRF score (diff={})",
            score_diff
        );
    }

    #[test]
    fn ret08_rrf_preserves_all_unique_items() {
        // Ensure no items are lost during merge
        let ids: Vec<Uuid> = (0..10).map(|_| Uuid::now_v7()).collect();
        let s1: Vec<ScoredHit> = ids[0..5].iter().map(|id| ScoredHit { id: *id }).collect();
        let s2: Vec<ScoredHit> = ids[5..10].iter().map(|id| ScoredHit { id: *id }).collect();

        let result = rrf_merge(vec![s1, s2]);
        assert_eq!(result.len(), 10, "All 10 unique items should be preserved");

        let result_ids: std::collections::HashSet<Uuid> =
            result.iter().map(|(id, _)| *id).collect();
        for id in &ids {
            assert!(result_ids.contains(id), "Item {:?} should be in result", id);
        }
    }

    #[test]
    fn ret08_rrf_score_is_deterministic() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();

        let r1 = rrf_merge(vec![
            vec![ScoredHit { id: a }, ScoredHit { id: b }],
            vec![ScoredHit { id: b }, ScoredHit { id: a }],
        ]);
        let r2 = rrf_merge(vec![
            vec![ScoredHit { id: a }, ScoredHit { id: b }],
            vec![ScoredHit { id: b }, ScoredHit { id: a }],
        ]);

        // Same inputs should produce identical scores
        assert_eq!(r1.len(), r2.len());
        for i in 0..r1.len() {
            assert_eq!(r1[i].0, r2[i].0);
            assert!((r1[i].1 - r2[i].1).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn ret08_rrf_symmetric_overlap_produces_equal_scores() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();

        // a is rank 0 in s1, rank 1 in s2
        // b is rank 1 in s1, rank 0 in s2
        // Both appear in both sources with mirror positions → equal RRF scores
        let s1 = vec![ScoredHit { id: a }, ScoredHit { id: b }];
        let s2 = vec![ScoredHit { id: b }, ScoredHit { id: a }];

        let result = rrf_merge(vec![s1, s2]);
        assert_eq!(result.len(), 2);
        // Both should have identical scores (symmetric overlap)
        assert!(
            (result[0].1 - result[1].1).abs() < f64::EPSILON,
            "Symmetric overlap should produce equal scores: {} vs {}",
            result[0].1,
            result[1].1
        );
    }
}
