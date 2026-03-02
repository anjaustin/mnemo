//! # mnemo-retrieval
//!
//! Hybrid retrieval engine: semantic search + full-text search + graph traversal.
//! Results are merged using Reciprocal Rank Fusion (RRF).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
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
    score: f32,
    source: RetrievalSource,
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
                if let Some(tf) = request.temporal_filter {
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
    hits.iter()
        .map(|(id, score)| ScoredHit {
            id: *id,
            score: *score,
            source,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_merge_single_source() {
        let id1 = Uuid::now_v7();
        let id2 = Uuid::now_v7();
        let source = vec![
            ScoredHit {
                id: id1,
                score: 0.95,
                source: RetrievalSource::SemanticSearch,
            },
            ScoredHit {
                id: id2,
                score: 0.80,
                source: RetrievalSource::SemanticSearch,
            },
        ];
        let result = rrf_merge(vec![source]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, id1); // Higher ranked item first
    }

    #[test]
    fn test_rrf_merge_boost_overlapping() {
        let shared_id = Uuid::now_v7();
        let only_semantic = Uuid::now_v7();
        let only_ft = Uuid::now_v7();

        let semantic = vec![
            ScoredHit {
                id: shared_id,
                score: 0.9,
                source: RetrievalSource::SemanticSearch,
            },
            ScoredHit {
                id: only_semantic,
                score: 0.8,
                source: RetrievalSource::SemanticSearch,
            },
        ];
        let ft = vec![
            ScoredHit {
                id: shared_id,
                score: 0.85,
                source: RetrievalSource::FullTextSearch,
            },
            ScoredHit {
                id: only_ft,
                score: 0.7,
                source: RetrievalSource::FullTextSearch,
            },
        ];

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
}
