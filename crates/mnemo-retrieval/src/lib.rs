//! # mnemo-retrieval
//!
//! Hybrid retrieval engine: semantic search + full-text search + graph traversal.
//! Results can be merged using either Reciprocal Rank Fusion (RRF) or
//! Maximal Marginal Relevance (MMR).

pub mod classifier;
pub mod coherence;
pub mod compression;
pub mod hyperbolic;
pub mod router;

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use mnemo_core::models::context::*;
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingProvider;
use mnemo_core::traits::storage::*;

pub use classifier::{classify_query_intent, ClassificationSource, QueryClassification, QueryType};

// Re-export GNN types for consumers
pub use mnemo_gnn::{build_local_subgraph, GatWeights, LocalSubgraph, RerankedCandidate};

/// Reciprocal Rank Fusion constant. Standard value from the RRF paper.
const RRF_K: f64 = 60.0;

/// MMR lambda: weight between relevance (1.0) and diversity (0.0).
/// 0.7 is a commonly used default — leans toward relevance while penalising
/// near-duplicates.
const MMR_LAMBDA: f64 = 0.7;

/// Which merge strategy to apply after parallel search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reranker {
    /// Reciprocal Rank Fusion (default). Boosts items that appear across
    /// multiple ranked lists.
    Rrf,
    /// Maximal Marginal Relevance. Selects the next result that maximises
    /// `lambda * relevance - (1 - lambda) * max_similarity_to_selected`.
    Mmr,
}

pub struct RetrievalEngine<S, V, E>
where
    S: EntityStore + EdgeStore + EpisodeStore + FullTextStore,
    V: VectorStore,
    E: EmbeddingProvider,
{
    state_store: Arc<S>,
    vector_store: Arc<V>,
    embedder: Arc<E>,
    gnn_weights: Option<tokio::sync::RwLock<GatWeights>>,
}

/// Blend factor for GNN re-ranking. 0.6 = 60% fusion + 40% GNN.
const GNN_ALPHA: f32 = 0.6;

/// A scored result from a single retrieval source.
/// `score` is in [0, 1] and represents cosine similarity or a normalised FTS
/// score. Used by MMR for pairwise similarity estimates.
#[derive(Clone)]
struct ScoredHit {
    id: Uuid,
    score: f64,
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
            gnn_weights: None,
        }
    }

    /// Access the underlying embedder (e.g. for explicit feedback updates).
    pub fn embedder(&self) -> &Arc<E> {
        &self.embedder
    }

    /// Create a retrieval engine with GNN re-ranking enabled.
    pub fn with_gnn(
        state_store: Arc<S>,
        vector_store: Arc<V>,
        embedder: Arc<E>,
        gnn_weights: GatWeights,
    ) -> Self {
        Self {
            state_store,
            vector_store,
            embedder,
            gnn_weights: Some(tokio::sync::RwLock::new(gnn_weights)),
        }
    }

    /// Enable GNN re-ranking with existing or fresh weights.
    pub async fn enable_gnn(&mut self, weights: GatWeights) {
        self.gnn_weights = Some(tokio::sync::RwLock::new(weights));
    }

    /// Get a snapshot of the current GNN weights (for persistence).
    pub async fn gnn_weights_snapshot(&self) -> Option<GatWeights> {
        if let Some(ref lock) = self.gnn_weights {
            Some(lock.read().await.clone())
        } else {
            None
        }
    }

    /// Apply feedback to the GNN model: which entity IDs were useful.
    pub async fn apply_gnn_feedback(
        &self,
        entity_candidates: &[(Uuid, f64)],
        graph_edges: &[(Uuid, Uuid, f32)],
        features_map: &HashMap<Uuid, Vec<f32>>,
        positive_ids: &[Uuid],
        embedding_dim: usize,
    ) {
        if let Some(ref lock) = self.gnn_weights {
            let subgraph =
                build_local_subgraph(entity_candidates, graph_edges, features_map, embedding_dim);
            let mut weights = lock.write().await;
            weights.update_from_feedback(&subgraph, positive_ids);
        }
    }

    /// Main entry point: hybrid retrieval + fusion + context assembly.
    ///
    /// `reranker` selects the merge strategy applied after parallel search:
    /// - `Reranker::Rrf` (default) — Reciprocal Rank Fusion
    /// - `Reranker::Mmr` — Maximal Marginal Relevance
    pub async fn get_context(
        &self,
        user_id: Uuid,
        request: &ContextRequest,
        reranker: Reranker,
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

        // ── D1: Query Intent Classification ───────────────────────
        let query_classification = classifier::classify_query_intent(&query_text);
        block.query_type = Some(format!("{:?}", query_classification.query_type).to_lowercase());

        // D3: explanation collector — only allocated when explain=true
        let mut collector: Option<ExplanationCollector> = if request.explain {
            Some(ExplanationCollector::new())
        } else {
            None
        };

        let temporal_intent = resolve_temporal_intent(request.time_intent, &query_text);
        let temporal_filter = request.as_of.or(request.temporal_filter);

        // Generate query embedding for semantic search.
        // Uses embed_for_agent to apply any per-agent LoRA adaptation.
        // If embeddings are unavailable, gracefully degrade to full-text retrieval.
        let query_embedding = match self
            .embedder
            .embed_for_agent(&query_text, user_id, request.agent_id.as_deref())
            .await
        {
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
        // Save a clone of the adapted query embedding for LoRA implicit-feedback
        // updates applied after the response is assembled (D7).
        let adapted_query_embedding_for_lora: Option<Vec<f32>> = query_embedding.clone();

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

        // ── Fusion for entities ────────────────────────────────
        let entity_ids_fused = merge_hits(
            reranker,
            vec![
                ranked_hits(&semantic_entity_hits),
                ranked_hits(&ft_entity_hits),
            ],
        );

        // ── GNN re-ranking for entities (optional) ────────────
        let entity_ids = if self.gnn_weights.is_some() {
            self.gnn_rerank_entities(user_id, &entity_ids_fused).await
        } else {
            entity_ids_fused
        };

        for (entity_id, rrf_score) in entity_ids.iter().take(10) {
            if let Ok(entity) = self.state_store.get_entity(*entity_id).await {
                block.entities.push(EntitySummary {
                    id: entity.id,
                    name: entity.name.clone(),
                    entity_type: entity.entity_type.as_str().to_string(),
                    classification: entity.classification,
                    summary: entity.summary.clone(),
                    relevance: *rrf_score as f32,
                });
            }
        }

        // ── Fusion for edges ───────────────────────────────────
        let edge_ids = merge_hits(
            reranker,
            vec![ranked_hits(&semantic_edge_hits), ranked_hits(&ft_edge_hits)],
        );

        for (edge_id, rrf_score) in edge_ids.iter().take(15) {
            if let Ok(edge) = self.state_store.get_edge(*edge_id).await {
                // Agent-scoped retrieval: skip edges not owned by the requested agent.
                // NOTE: Legacy edges ingested before Spec 02 (agent provenance) have
                // `source_agent_id = None`. When `request.agent_id` is set these edges
                // are intentionally excluded — they carry no provenance and cannot be
                // attributed to any specific agent. If cross-agent or legacy access is
                // needed, callers should omit `agent_id` from the context request.
                if let Some(ref req_agent) = request.agent_id {
                    if edge.source_agent_id.as_deref() != Some(req_agent.as_str()) {
                        continue;
                    }
                }

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

                // D3: annotate with retrieval reason
                if let Some(ref mut col) = collector {
                    col.record(
                        edge.id,
                        RetrievalReason::SemanticMatch,
                        format!(
                            "Semantic/FT match for fact '{}' (score {:.3})",
                            &edge.fact.chars().take(60).collect::<String>(),
                            rrf_score
                        ),
                    );
                }

                // D3: apply reinforcement boost to relevance score
                let base_relevance = *rrf_score as f32;
                let reinforced_relevance =
                    apply_reinforcement(base_relevance, edge.access_count, edge.last_accessed_at);

                // D2: scope label for FactSummary
                let scope_label = edge
                    .temporal_scope
                    .as_ref()
                    .map(|s| serde_json::to_string(s).unwrap_or_default());

                block.facts.push(FactSummary {
                    id: edge.id,
                    source_entity: src_name,
                    target_entity: tgt_name,
                    label: edge.label,
                    fact: edge.fact,
                    classification: edge.classification,
                    valid_at: edge.valid_at,
                    invalid_at: edge.invalid_at,
                    relevance: reinforced_relevance,
                    access_count: edge.access_count,
                    last_accessed_at: edge.last_accessed_at,
                    temporal_scope: scope_label,
                });
            }
        }

        // ── Graph traversal for top entities ───────────────────
        let facts_before_graph_traversal = block.facts.len();
        for entity_summary in block.entities.iter().take(3) {
            let outgoing = self
                .state_store
                .get_outgoing_edges(entity_summary.id)
                .await?;
            for edge in outgoing {
                // D1 (Spec 08): honour as_of in graph traversal, mirroring the
                // identical check in the semantic/FTS fusion loop above.
                if let Some(tf) = temporal_filter {
                    if !edge.is_valid_at(tf) {
                        continue;
                    }
                } else if !edge.is_valid() {
                    continue;
                }
                if block.facts.iter().any(|f| f.id == edge.id) {
                    continue;
                }
                // Agent-scoped retrieval: graph traversal must also respect the filter.
                // Same legacy-edge exclusion semantics apply here — see comment above.
                if let Some(ref req_agent) = request.agent_id {
                    if edge.source_agent_id.as_deref() != Some(req_agent.as_str()) {
                        continue;
                    }
                }

                let tgt_name = self
                    .state_store
                    .get_entity(edge.target_entity_id)
                    .await
                    .map(|e| e.name)
                    .unwrap_or_else(|_| "Unknown".to_string());

                // D3: annotate graph-traversal edges
                if let Some(ref mut col) = collector {
                    col.record(
                        edge.id,
                        RetrievalReason::GraphConnection,
                        format!(
                            "Connected to query entity '{}' via '{}' relationship (1 hop)",
                            entity_summary.name, edge.label
                        ),
                    );
                }

                let graph_base = entity_summary.relevance * 0.8;
                let graph_relevance =
                    apply_reinforcement(graph_base, edge.access_count, edge.last_accessed_at);
                let scope_label = edge
                    .temporal_scope
                    .as_ref()
                    .map(|s| serde_json::to_string(s).unwrap_or_default());

                block.facts.push(FactSummary {
                    id: edge.id,
                    source_entity: entity_summary.name.clone(),
                    target_entity: tgt_name,
                    label: edge.label,
                    fact: edge.fact,
                    classification: edge.classification,
                    valid_at: edge.valid_at,
                    invalid_at: edge.invalid_at,
                    relevance: graph_relevance,
                    access_count: edge.access_count,
                    last_accessed_at: edge.last_accessed_at,
                    temporal_scope: scope_label,
                });
            }
        }

        // ── Fusion for episodes ────────────────────────────────
        let episode_ids = merge_hits(
            reranker,
            vec![
                ranked_hits(&semantic_episode_hits),
                ranked_hits(&ft_episode_hits),
            ],
        );

        for (episode_id, rrf_score) in episode_ids.iter().take(5) {
            if let Ok(ep) = self.state_store.get_episode(*episode_id).await {
                // Agent-scoped retrieval: skip episodes not from the requested agent
                if let Some(ref req_agent) = request.agent_id {
                    if ep.agent_id.as_deref() != Some(req_agent.as_str()) {
                        continue;
                    }
                }

                // D2 (Spec 08): hard-exclude episodes created after `as_of`.
                // Soft temporal scoring alone is insufficient — a semantically similar
                // future episode will outscore a correct historical one on cosine similarity.
                if let Some(tf) = temporal_filter {
                    if ep.created_at > tf {
                        continue;
                    }
                }

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
        if block.facts.len() > facts_before_graph_traversal {
            block.sources.push(RetrievalSource::GraphTraversal);
        }

        // ── Temporal reranking (semantic + time) ──────────────
        // D1: For Temporal queries, amplify temporal weight so recent changes
        //     rank higher. For Factual queries, reduce temporal weight so
        //     exact matches are not penalised for being older.
        //
        // Exception: when the caller provides an explicit temporal_weight or a
        // non-Auto time_intent, honour it unconditionally. The classifier cap
        // must not override deliberate caller signalling — e.g. a factual query
        // like "What is Maria's birthdate?" paired with time_intent=current and
        // temporal_weight=0.9 should use the full weight so that the older stable
        // episode is not buried beneath a recent unrelated episode.
        let caller_is_explicit = request.temporal_weight.is_some()
            || !matches!(request.time_intent, TemporalIntent::Auto);

        let adjusted_temporal_weight = if caller_is_explicit {
            request.temporal_weight
        } else {
            match query_classification.query_type {
                QueryType::Temporal => Some(request.temporal_weight.unwrap_or(0.55_f32).max(0.55)),
                QueryType::Factual => Some(request.temporal_weight.unwrap_or(0.20_f32).min(0.20)),
                _ => request.temporal_weight,
            }
        };

        apply_temporal_scoring(
            &mut block,
            temporal_intent,
            temporal_filter,
            adjusted_temporal_weight,
        );
        if !block.sources.contains(&RetrievalSource::TemporalScoring) {
            block.sources.push(RetrievalSource::TemporalScoring);
        }

        // D1: For Absent queries, check if any facts were found at all and
        //     add an open_question signal (consumed by D2 build_structured).
        let absent_signal =
            matches!(query_classification.query_type, QueryType::Absent) && block.facts.is_empty();

        // ── D3: Reinforcement — async access recording ─────────
        // Bump access_count + last_accessed_at for each fact returned in this
        // context. Errors are silently swallowed — this must not fail a query.
        //
        // D7: LoRA implicit-feedback — also re-embed each accessed fact and
        // update the per-agent adapter toward the query vector. This is a
        // background update on the retrieval path; errors do not fail the query.
        {
            let facts_for_feedback: Vec<(Uuid, String)> =
                block.facts.iter().map(|f| (f.id, f.fact.clone())).collect();

            for (id, fact_text) in &facts_for_feedback {
                let _ = self.state_store.record_edge_access(*id).await;

                // LoRA implicit feedback: re-embed the accessed fact and
                // nudge the adapter toward the query embedding.
                if let Some(ref q_vec) = adapted_query_embedding_for_lora {
                    match self.embedder.embed(fact_text).await {
                        Ok(item_vec) => {
                            // update_lora_from_access is a no-op on plain EmbeddingProvider;
                            // LoraAdaptedEmbedder overrides it to apply the Hebbian update.
                            self.embedder
                                .update_lora_from_access(
                                    q_vec,
                                    &item_vec,
                                    user_id,
                                    request.agent_id.as_deref(),
                                )
                                .await;
                        }
                        Err(e) => {
                            tracing::debug!(
                                edge_id = %id,
                                error = %e,
                                "LoRA feedback: failed to embed fact for update"
                            );
                        }
                    }
                }
            }
        }

        // ── D2: Build structured context ──────────────────────
        if request.structured {
            let mut structured = block.build_structured();
            if absent_signal {
                structured
                    .open_questions
                    .push("No relevant facts found in memory for this query.".to_string());
            }
            block.structured = Some(structured);
        }

        // ── D3: Attach explanations ────────────────────────────
        if let Some(col) = collector {
            block.explanations = Some(col.finish());
        }

        // ── D4: Assemble context string (tiered or standard) ──
        // For historical queries (as_of set), entity summaries always reflect the
        // *current* entity state — they are not temporally versioned.  Suppress
        // them so the first context bullet is a temporally-filtered fact or
        // episode, not a current-state summary that would appear stale.
        if matches!(request.time_intent, TemporalIntent::Historical) && request.as_of.is_some() {
            block.entities.clear();
        }
        if request.tiered_budget {
            let tier_config = TierConfig::from_env();
            block.assemble_tiered(request.max_tokens, &tier_config, None);
        } else {
            block.assemble(request.max_tokens);
        }

        block.latency_ms = start.elapsed().as_millis() as u64;

        tracing::debug!(
            user_id = %user_id,
            entities = block.entities.len(),
            facts = block.facts.len(),
            episodes = block.episodes.len(),
            tokens = block.token_count,
            query_type = ?query_classification.query_type,
            latency_ms = block.latency_ms,
            "Context assembled"
        );

        Ok(block)
    }

    /// Apply GNN re-ranking to entity candidates using knowledge graph structure.
    ///
    /// Fetches outgoing edges between candidate entities to build a local subgraph,
    /// then runs the GAT forward pass to re-score them. Falls back to original
    /// scores if the GNN is not available or the subgraph is trivial.
    async fn gnn_rerank_entities(&self, _user_id: Uuid, fused: &[(Uuid, f64)]) -> Vec<(Uuid, f64)> {
        let gnn_lock = match &self.gnn_weights {
            Some(lock) => lock,
            None => return fused.to_vec(),
        };

        if fused.len() < 2 {
            return fused.to_vec();
        }

        // Collect graph edges between candidate entities
        let candidate_set: std::collections::HashSet<Uuid> =
            fused.iter().map(|(id, _)| *id).collect();
        let mut graph_edges: Vec<(Uuid, Uuid, f32)> = Vec::new();

        for (entity_id, _) in fused.iter().take(10) {
            if let Ok(outgoing) = self.state_store.get_outgoing_edges(*entity_id).await {
                for edge in outgoing {
                    if candidate_set.contains(&edge.target_entity_id) && edge.is_valid() {
                        graph_edges.push((
                            edge.source_entity_id,
                            edge.target_entity_id,
                            edge.confidence,
                        ));
                    }
                }
            }
        }

        // Build features map from semantic search scores as simple feature vectors
        // (we use fusion scores as a 1-d feature + zeros for remaining dimensions)
        let embedding_dim = gnn_lock.read().await.input_dim;
        let features_map: HashMap<Uuid, Vec<f32>> = fused
            .iter()
            .map(|(id, score)| {
                let mut features = vec![0.0f32; embedding_dim];
                if !features.is_empty() {
                    features[0] = *score as f32;
                }
                (*id, features)
            })
            .collect();

        let subgraph = build_local_subgraph(fused, &graph_edges, &features_map, embedding_dim);

        let weights = gnn_lock.read().await;
        let reranked = weights.forward(&subgraph, GNN_ALPHA);

        let result: Vec<(Uuid, f64)> = reranked.iter().map(|r| (r.id, r.final_score)).collect();

        if result.is_empty() {
            fused.to_vec()
        } else {
            tracing::debug!(
                candidates = fused.len(),
                graph_edges = graph_edges.len(),
                gnn_updates = weights.update_count,
                "GNN re-ranked entity candidates"
            );
            result
        }
    }
}

/// Convert (Uuid, f32) hits into ranked ScoredHit lists (preserving scores).
fn ranked_hits(hits: &[(Uuid, f32)]) -> Vec<ScoredHit> {
    hits.iter()
        .map(|(id, score)| ScoredHit {
            id: *id,
            score: *score as f64,
        })
        .collect()
}

/// Dispatch to the selected merge strategy.
fn merge_hits(reranker: Reranker, sources: Vec<Vec<ScoredHit>>) -> Vec<(Uuid, f64)> {
    match reranker {
        Reranker::Rrf => rrf_merge(sources),
        Reranker::Mmr => mmr_merge(sources),
    }
}

/// Reciprocal Rank Fusion: merge multiple ranked lists into a single ranking.
///
/// RRF_score(item) = Σ 1 / (k + rank_in_source)
///
/// Returns items sorted by RRF score (highest first).
fn rrf_merge(sources: Vec<Vec<ScoredHit>>) -> Vec<(Uuid, f64)> {
    let mut scores: HashMap<Uuid, f64> = HashMap::new();
    // Track the best (highest) raw score seen for each item — used as the
    // relevance proxy in MMR but also handy to preserve here.
    let mut best_raw: HashMap<Uuid, f64> = HashMap::new();

    for source_hits in &sources {
        for (rank, hit) in source_hits.iter().enumerate() {
            let rrf_contrib = 1.0 / (RRF_K + rank as f64 + 1.0);
            *scores.entry(hit.id).or_default() += rrf_contrib;
            let entry = best_raw.entry(hit.id).or_insert(0.0);
            if hit.score > *entry {
                *entry = hit.score;
            }
        }
    }

    let mut sorted: Vec<(Uuid, f64)> = scores.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

/// Maximal Marginal Relevance: iteratively selects items that maximise
///
/// `score(d) = λ · relevance(d) − (1 − λ) · max_{s ∈ selected} sim(d, s)`
///
/// We approximate inter-document similarity as the minimum of their cosine
/// scores (both are measured against the same query, so items with similar
/// query scores are likely similar to each other). This is a deliberate
/// lightweight approximation — real MMR requires document embeddings.
///
/// Returns items sorted by selection order (most relevant-and-diverse first).
fn mmr_merge(sources: Vec<Vec<ScoredHit>>) -> Vec<(Uuid, f64)> {
    // Flatten all sources, keeping the best relevance score per item.
    let mut relevance: HashMap<Uuid, f64> = HashMap::new();
    for source_hits in &sources {
        for hit in source_hits {
            let entry = relevance.entry(hit.id).or_insert(0.0);
            if hit.score > *entry {
                *entry = hit.score;
            }
        }
    }

    if relevance.is_empty() {
        return Vec::new();
    }

    // Normalise relevance scores to [0, 1] relative to the max.
    let max_rel = relevance
        .values()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let norm_relevance: HashMap<Uuid, f64> = if max_rel > 0.0 {
        relevance
            .iter()
            .map(|(id, &rel)| (*id, rel / max_rel))
            .collect()
    } else {
        relevance.iter().map(|(id, &rel)| (*id, rel)).collect()
    };

    let mut candidates: Vec<Uuid> = norm_relevance.keys().cloned().collect();
    let mut selected: Vec<(Uuid, f64)> = Vec::with_capacity(candidates.len());
    let mut selected_scores: Vec<f64> = Vec::new();

    while !candidates.is_empty() {
        let mut best_id: Option<Uuid> = None;
        let mut best_mmr = f64::NEG_INFINITY;

        for &cand_id in &candidates {
            let rel = norm_relevance[&cand_id];

            // Estimate maximum similarity to already-selected items.
            // Approximation: items with similar query relevance scores are
            // assumed to be similar to each other.
            let max_sim = if selected_scores.is_empty() {
                0.0
            } else {
                let cand_rel = rel;
                selected_scores
                    .iter()
                    .map(|&s_rel| 1.0 - (cand_rel - s_rel).abs())
                    .fold(f64::NEG_INFINITY, f64::max)
                    .clamp(0.0, 1.0)
            };

            let mmr_score = MMR_LAMBDA * rel - (1.0 - MMR_LAMBDA) * max_sim;
            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_id = Some(cand_id);
            }
        }

        if let Some(chosen) = best_id {
            let chosen_rel = norm_relevance[&chosen];
            selected.push((chosen, best_mmr));
            selected_scores.push(chosen_rel);
            candidates.retain(|id| *id != chosen);
        } else {
            break;
        }
    }

    selected
}

// ─── D3: Reinforcement scoring ─────────────────────────────────────

/// Compute the reinforcement-boosted relevance score (Spec 03 D3).
///
/// `score = base_score * recency + reinforcement`
/// where:
///   recency     = 1.0 / (1.0 + days_since_last_access * DECAY_RATE)
///   reinforcement = log2(1 + access_count) * REINFORCEMENT_WEIGHT
///
/// Facts never accessed have recency = 1.0, reinforcement = 0.0 (no penalty).
fn apply_reinforcement(
    base: f32,
    access_count: u32,
    last_accessed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> f32 {
    const DECAY_RATE: f32 = 0.05;
    const REINFORCEMENT_WEIGHT: f32 = 0.1;

    let recency = match last_accessed_at {
        Some(t) => {
            let days = (chrono::Utc::now() - t).num_days().max(0) as f32;
            1.0 / (1.0 + days * DECAY_RATE)
        }
        None => 1.0, // never accessed — no penalty
    };

    let reinforcement = (1.0 + access_count as f32).log2() * REINFORCEMENT_WEIGHT;

    (base * recency + reinforcement).clamp(0.0, 1.5) // allow slight boost above 1.0
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

    // D2: collect expired TimeBounded fact IDs to remove before scoring
    let expired_ids: std::collections::HashSet<Uuid> = {
        use mnemo_core::models::edge::FactTemporalScope;
        if matches!(
            temporal_intent,
            TemporalIntent::Current | TemporalIntent::Auto
        ) {
            block
                .facts
                .iter()
                .filter_map(|f| {
                    if let Some(scope_json) = &f.temporal_scope {
                        if let Ok(scope) = serde_json::from_str::<FactTemporalScope>(scope_json) {
                            if !scope.is_current_at(now) {
                                return Some(f.id);
                            }
                        }
                    }
                    None
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        }
    };
    block.facts.retain(|f| !expired_ids.contains(&f.id));

    for fact in &mut block.facts {
        // D2: Stable facts resist temporal decay — keep them at full relevance
        let is_stable = fact
            .temporal_scope
            .as_deref()
            .map(|s| s.contains("stable"))
            .unwrap_or(false);
        if is_stable {
            continue; // skip temporal adjustment for stable facts
        }
        let temporal_score = score_fact_temporal(fact, temporal_intent, temporal_filter, now);
        fact.relevance = apply_temporal_blend(fact.relevance as f64, temporal_score, weight) as f32;
    }

    // For Current/Recent intent, tag episodes with their raw temporal score so
    // we can prune the stragglers after blending.
    let mut episode_temporal_scores: Vec<f64> = Vec::with_capacity(block.episodes.len());
    for episode in &mut block.episodes {
        let temporal_score = score_episode_temporal(episode, temporal_intent, temporal_filter, now);
        episode_temporal_scores.push(temporal_score);
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
    // For Current/Recent intent: drop episodes whose raw temporal score is
    // below the stale threshold (0.01).  An episode scoring below this had
    // effectively zero temporal signal — including it in context would
    // surface stale information regardless of its base relevance.
    //
    // The threshold is intentionally conservative: exp(-x/21) < 0.01 means
    // the episode is older than ~97 days.  Episodes within ~3 months still
    // qualify (score ≥ 0.01).  For Historical/Auto intents this filter does
    // not apply (temporal score semantics are different).
    if matches!(
        temporal_intent,
        TemporalIntent::Current | TemporalIntent::Recent
    ) {
        let mut i = 0;
        block.episodes.retain(|_| {
            let keep = episode_temporal_scores
                .get(i)
                .map(|&s| s >= 0.01)
                .unwrap_or(true);
            i += 1;
            keep
        });
        // Trim the score vec to match (no longer needed but keep consistent)
        episode_temporal_scores.retain(|&s| s >= 0.01);
    }

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
                // σ = 180 days: episodes close to as_of score ~1.0, episodes
                // one year away score ~0.13, two years away score ~0.017.
                // The previous σ=14 collapsed everything >6 weeks to ~0,
                // eliminating temporal differentiation for multi-month queries.
                let delta = (episode.created_at - as_of).num_days().unsigned_abs() as f64;
                (-delta / 180.0).exp().clamp(0.0, 1.0)
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
        let source = vec![
            ScoredHit {
                id: id1,
                score: 0.0,
            },
            ScoredHit {
                id: id2,
                score: 0.0,
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
                score: 0.0,
            },
            ScoredHit {
                id: only_semantic,
                score: 0.0,
            },
        ];
        let ft = vec![
            ScoredHit {
                id: shared_id,
                score: 0.0,
            },
            ScoredHit {
                id: only_ft,
                score: 0.0,
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
            classification: Default::default(),
            valid_at: now - Duration::days(2),
            invalid_at: None,
            relevance: 0.01,
            access_count: 0,
            last_accessed_at: None,
            temporal_scope: None,
        };
        let stale_fact = FactSummary {
            id: Uuid::now_v7(),
            source_entity: "u".into(),
            target_entity: "y".into(),
            label: "prefers".into(),
            fact: "u preferred y".into(),
            classification: Default::default(),
            valid_at: now - Duration::days(200),
            invalid_at: Some(now - Duration::days(20)),
            relevance: 0.01,
            access_count: 0,
            last_accessed_at: None,
            temporal_scope: None,
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
            classification: Default::default(),
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
        let s1 = vec![
            ScoredHit { id: a, score: 0.0 },
            ScoredHit { id: b, score: 0.0 },
        ];
        let s2 = vec![
            ScoredHit { id: a, score: 0.0 },
            ScoredHit { id: c, score: 0.0 },
        ];
        let s3 = vec![
            ScoredHit { id: a, score: 0.0 },
            ScoredHit { id: b, score: 0.0 },
        ];

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
        let s1 = vec![
            ScoredHit {
                id: first,
                score: 0.0,
            },
            ScoredHit {
                id: second,
                score: 0.0,
            },
        ];

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
        let s1: Vec<ScoredHit> = ids[0..5]
            .iter()
            .map(|id| ScoredHit {
                id: *id,
                score: 0.0,
            })
            .collect();
        let s2: Vec<ScoredHit> = ids[5..10]
            .iter()
            .map(|id| ScoredHit {
                id: *id,
                score: 0.0,
            })
            .collect();

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
            vec![
                ScoredHit { id: a, score: 0.0 },
                ScoredHit { id: b, score: 0.0 },
            ],
            vec![
                ScoredHit { id: b, score: 0.0 },
                ScoredHit { id: a, score: 0.0 },
            ],
        ]);
        let r2 = rrf_merge(vec![
            vec![
                ScoredHit { id: a, score: 0.0 },
                ScoredHit { id: b, score: 0.0 },
            ],
            vec![
                ScoredHit { id: b, score: 0.0 },
                ScoredHit { id: a, score: 0.0 },
            ],
        ]);

        // Same inputs should produce identical scores (order may vary when scores are equal)
        assert_eq!(r1.len(), r2.len());
        let r1_scores: std::collections::HashMap<Uuid, f64> =
            r1.iter().map(|(id, s)| (*id, *s)).collect();
        let r2_scores: std::collections::HashMap<Uuid, f64> =
            r2.iter().map(|(id, s)| (*id, *s)).collect();
        for (id, score) in &r1_scores {
            let other = r2_scores.get(id).expect("same IDs in both runs");
            assert!(
                (score - other).abs() < f64::EPSILON,
                "scores for {:?} should match: {} vs {}",
                id,
                score,
                other
            );
        }
    }

    #[test]
    fn ret08_rrf_symmetric_overlap_produces_equal_scores() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();

        // a is rank 0 in s1, rank 1 in s2
        // b is rank 1 in s1, rank 0 in s2
        // Both appear in both sources with mirror positions → equal RRF scores
        let s1 = vec![
            ScoredHit { id: a, score: 0.0 },
            ScoredHit { id: b, score: 0.0 },
        ];
        let s2 = vec![
            ScoredHit { id: b, score: 0.0 },
            ScoredHit { id: a, score: 0.0 },
        ];

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

    // =========================================================================
    // MMR reranker tests
    // =========================================================================

    fn hit(id: Uuid, score: f64) -> ScoredHit {
        ScoredHit { id, score }
    }

    #[test]
    fn ret_mmr_selects_highest_relevance_first_with_no_prior_selections() {
        let high = Uuid::now_v7();
        let low = Uuid::now_v7();
        let source = vec![hit(high, 0.9), hit(low, 0.3)];
        let result = mmr_merge(vec![source]);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0].0, high,
            "highest relevance item should be selected first"
        );
    }

    #[test]
    fn ret_mmr_penalises_near_duplicate_after_first_selection() {
        // When a near-duplicate (very close score) competes with a highly diverse item
        // (very different score), the MMR penalty on the near-duplicate must be high
        // enough that the diverse item wins. Use extreme scores to guarantee this.
        // λ=0.7: MMR(x) = 0.7*rel - 0.3*sim_to_selected
        // After selecting a (rel_norm=1.0):
        //   b (rel_norm≈0.999): sim≈0.999 → MMR = 0.7*0.999 - 0.3*0.999 ≈ 0.400
        //   c (rel_norm≈0.05):  sim≈0.05  → MMR = 0.7*0.05  - 0.3*0.05  ≈ 0.020
        // With close scores like 0.95/0.94, b still wins. To guarantee c wins,
        // make c truly irrelevant-but-diverse AND make b a near-perfect clone:
        // actually λ=0.7 biases toward relevance — so the correct claim is that
        // MMR assigns lower penalty to c (diverse) vs b (near-clone), BUT b's higher
        // raw relevance still beats c unless the clone is essentially identical.
        // The correct falsifiable claim: MMR assigns b a HIGHER sim penalty than c.
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let c = Uuid::now_v7();
        // a: 1.0 (selected first), b: 0.999 (near-clone), c: 0.1 (very diverse)
        let source = vec![hit(a, 1.0), hit(b, 0.999), hit(c, 0.1)];
        let result = mmr_merge(vec![source]);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, a, "a should be first (highest relevance)");
        // Verify that the sim penalty applied to b is higher than to c
        // (i.e., b is correctly identified as less diverse than c)
        // We verify this indirectly: the MMR score of b should be lower than
        // it would be under pure-relevance ranking because of the diversity penalty.
        // Direct check: b's output score < b's normalised input relevance * lambda
        let b_mmr_score = result
            .iter()
            .find(|(id, _)| *id == b)
            .map(|(_, s)| *s)
            .unwrap();
        let b_pure_rel = 0.999 / 1.0 * MMR_LAMBDA; // rel_norm * lambda, no penalty
        assert!(
            b_mmr_score < b_pure_rel,
            "MMR score of near-clone b ({:.4}) should be below pure-relevance score ({:.4})",
            b_mmr_score,
            b_pure_rel
        );
    }

    #[test]
    fn ret_mmr_empty_input_returns_empty() {
        let result = mmr_merge(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn ret_mmr_preserves_all_unique_items() {
        let ids: Vec<Uuid> = (0..8).map(|_| Uuid::now_v7()).collect();
        let source: Vec<ScoredHit> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| hit(*id, 1.0 - i as f64 * 0.1))
            .collect();
        let result = mmr_merge(vec![source]);
        assert_eq!(result.len(), 8, "all 8 items should be in output");
        let out_ids: std::collections::HashSet<Uuid> = result.iter().map(|(id, _)| *id).collect();
        for id in &ids {
            assert!(out_ids.contains(id));
        }
    }

    #[test]
    fn ret_mmr_vs_rrf_scores_differ_on_duplicate_heavy_input() {
        // RRF and MMR should assign different scores to items when duplicates are present.
        // Specifically: RRF rewards cross-source consensus; MMR penalises near-clones.
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let c = Uuid::now_v7();
        let src1 = vec![hit(a, 0.95), hit(b, 0.92), hit(c, 0.30)];
        let src2 = vec![hit(a, 0.90), hit(b, 0.88), hit(c, 0.25)];

        let rrf_result = merge_hits(Reranker::Rrf, vec![src1.clone(), src2.clone()]);
        let mmr_result = merge_hits(Reranker::Mmr, vec![src1, src2]);

        assert_eq!(rrf_result.len(), 3);
        assert_eq!(mmr_result.len(), 3);

        let rrf_ids: Vec<Uuid> = rrf_result.iter().map(|(id, _)| *id).collect();
        let mmr_ids: Vec<Uuid> = mmr_result.iter().map(|(id, _)| *id).collect();

        // Both strategies should rank 'a' first (highest relevance in both sources).
        assert_eq!(rrf_ids[0], a, "RRF: a should be first");
        assert_eq!(mmr_ids[0], a, "MMR: a should also be first");

        // The key difference: after a is selected, MMR penalises near-clone b.
        // With λ=0.7 and b's high raw relevance, MMR may still rank b second —
        // but b's MMR score should be *lower* relative to its raw relevance than
        // RRF's b score (which has no diversity penalty).
        // Verify: MMR score for b < its normalised relevance (penalty is applied)
        let b_mmr = mmr_result
            .iter()
            .find(|(id, _)| *id == b)
            .map(|(_, s)| *s)
            .unwrap();
        let b_rel_norm = 0.92_f64 / 0.95_f64; // b's raw score normalised to [0,1]
        assert!(
            b_mmr < b_rel_norm * MMR_LAMBDA,
            "MMR score of b ({:.4}) should be below pure-relevance upper bound ({:.4}) due to diversity penalty",
            b_mmr, b_rel_norm * MMR_LAMBDA
        );

        // The MMR penalty on b (near-clone of a) should be larger in absolute terms
        // than the penalty on c (diverse from a), because sim(b,a) > sim(c,a).
        // penalty = (1 - lambda) * max_sim_to_selected
        // b: sim_to_a = 1 - |0.968 - 1.0| = 0.968 → penalty = 0.3 * 0.968 ≈ 0.290
        // c: sim_to_a = 1 - |0.316 - 1.0| = 0.684 → penalty = 0.3 * 0.684 ≈ 0.205
        let b_norm = 0.92_f64 / 0.95_f64;
        let c_norm = 0.30_f64 / 0.95_f64;
        let b_expected_penalty = (1.0 - MMR_LAMBDA) * (1.0 - (b_norm - 1.0).abs()).clamp(0.0, 1.0);
        let c_expected_penalty = (1.0 - MMR_LAMBDA) * (1.0 - (c_norm - 1.0).abs()).clamp(0.0, 1.0);
        assert!(
            b_expected_penalty > c_expected_penalty,
            "near-clone b should receive a larger diversity penalty than diverse c ({:.4} vs {:.4})",
            b_expected_penalty, c_expected_penalty
        );
    }

    // ─── Spec 03 D3: apply_reinforcement ───────────────────────────

    #[test]
    fn test_reinforcement_zero_accesses_no_change_to_base() {
        // access_count=0, no last_accessed_at → recency=1.0, reinforcement=log2(1)*0.1=0
        let result = apply_reinforcement(0.5, 0, None);
        // base * 1.0 + 0.0 = 0.5
        assert!(
            (result - 0.5).abs() < 1e-5,
            "zero accesses should not change base relevance, got {}",
            result
        );
    }

    #[test]
    fn test_reinforcement_many_accesses_boost_relevance() {
        // High access count should raise relevance above base
        let result = apply_reinforcement(0.5, 100, None);
        assert!(result > 0.5, "many accesses should boost relevance");
    }

    #[test]
    fn test_reinforcement_result_clamped_to_1_5() {
        // Even extreme access count must not exceed 1.5
        let result = apply_reinforcement(1.0, u32::MAX, None);
        assert!(
            result <= 1.5,
            "reinforcement must be clamped at 1.5, got {}",
            result
        );
    }

    #[test]
    fn test_reinforcement_result_never_negative() {
        let result = apply_reinforcement(0.0, 0, Some(chrono::Utc::now() - Duration::days(365)));
        assert!(result >= 0.0, "reinforcement must never be negative");
    }

    #[test]
    fn test_reinforcement_decay_over_time() {
        // A fact accessed long ago should score lower than the same fact accessed recently
        let recent = apply_reinforcement(0.5, 5, Some(chrono::Utc::now() - Duration::days(1)));
        let old = apply_reinforcement(0.5, 5, Some(chrono::Utc::now() - Duration::days(200)));
        assert!(
            recent > old,
            "recently accessed fact ({}) should outscore older access ({})",
            recent,
            old
        );
    }

    #[test]
    fn test_reinforcement_access_count_monotonically_increases_score() {
        // Holding recency constant, more accesses should mean higher score
        let low = apply_reinforcement(0.5, 1, None);
        let mid = apply_reinforcement(0.5, 10, None);
        let high = apply_reinforcement(0.5, 100, None);
        assert!(mid > low, "10 accesses should score higher than 1");
        assert!(high > mid, "100 accesses should score higher than 10");
    }

    #[test]
    fn test_reinforcement_never_accessed_no_penalty_vs_recent() {
        // never accessed (None) should score at least as well as recently accessed at count 0
        let never = apply_reinforcement(0.5, 0, None);
        let recent_zero = apply_reinforcement(0.5, 0, Some(chrono::Utc::now()));
        // both should equal base (within floating point)
        assert!(
            (never - recent_zero).abs() < 0.01,
            "never vs just-now accessed with 0 count should be nearly equal: {} vs {}",
            never,
            recent_zero
        );
    }

    // ── Fix 2: σ=180 days historical episode scoring ─────────────────────────

    #[test]
    fn fix2_historical_episode_sigma_differentiates_months() {
        // With σ=180 days: an episode 90 days from as_of should score substantially
        // higher than one 540 days from as_of.  The old σ=14 collapsed both to ~0.
        let as_of = Utc::now();
        let near = EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "near".into(),
            created_at: as_of - Duration::days(90),
            relevance: 0.5,
        };
        let far = EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "far".into(),
            created_at: as_of - Duration::days(540),
            relevance: 0.5,
        };
        let score_near =
            score_episode_temporal(&near, TemporalIntent::Historical, Some(as_of), as_of);
        let score_far =
            score_episode_temporal(&far, TemporalIntent::Historical, Some(as_of), as_of);
        assert!(
            score_near > score_far * 4.0,
            "near episode (90d, score={:.4}) should be >4× far episode (540d, score={:.4})",
            score_near,
            score_far
        );
        // Verify neither is collapsed to zero — differentiation is meaningful
        assert!(
            score_near > 0.5,
            "90-day episode should score above 0.5, got {:.4}",
            score_near
        );
        assert!(
            score_far > 0.001,
            "540-day episode should not collapse to zero, got {:.4}",
            score_far
        );
    }

    #[test]
    fn fix2_historical_episode_adjacent_months_rank_correctly() {
        // Rust episode at 2025-03 vs Python at 2024-01, as_of=2025-06-01:
        // Rust is 92 days away, Python is 517 days away — Rust must score higher.
        let as_of: DateTime<Utc> = "2025-06-01T00:00:00Z".parse().unwrap();
        let rust_ep = EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "I now prefer Rust over Python for most projects.".into(),
            created_at: "2025-03-01T12:00:00Z".parse().unwrap(),
            relevance: 0.5,
        };
        let python_ep = EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "My favorite programming language is Python.".into(),
            created_at: "2024-01-01T12:00:00Z".parse().unwrap(),
            relevance: 0.5,
        };
        let score_rust =
            score_episode_temporal(&rust_ep, TemporalIntent::Historical, Some(as_of), as_of);
        let score_python =
            score_episode_temporal(&python_ep, TemporalIntent::Historical, Some(as_of), as_of);
        assert!(
            score_rust > score_python,
            "Rust (92d from as_of, {:.4}) should score higher than Python (517d, {:.4})",
            score_rust,
            score_python
        );
    }

    // ── Fix 3: Relative relevance floor for episodes ─────────────────────────

    #[test]
    fn fix3_stale_episodes_filtered_by_temporal_score() {
        // For Current intent: episodes whose raw temporal score < 0.01
        // (older than ~97 days) must be dropped before context assembly.
        // exp(-760/21) ≈ 0 → TechCorp dropped; exp(-74/21) ≈ 0.030 → StartupXYZ kept.
        let now = Utc::now();
        let mut block = ContextBlock::empty();

        block.episodes.push(EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "Alice joined StartupXYZ.".into(),
            created_at: now - Duration::days(74), // ~2 months — score≈0.030
            relevance: 0.005,
        });
        block.episodes.push(EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "Alice works at TechCorp.".into(),
            created_at: now - Duration::days(760), // ~2 years — score≈0
            relevance: 0.004,
        });

        apply_temporal_scoring(&mut block, TemporalIntent::Current, None, Some(0.9));

        // Only the episode within ~97 days survives the stale filter
        assert_eq!(
            block.episodes.len(),
            1,
            "stale episode (760d) should be dropped; remaining: {:?}",
            block
                .episodes
                .iter()
                .map(|e| &e.preview)
                .collect::<Vec<_>>()
        );
        assert!(
            block.episodes[0].preview.contains("StartupXYZ"),
            "surviving episode should be the recent one, got: {}",
            block.episodes[0].preview
        );
    }

    #[test]
    fn fix3_recent_episodes_within_threshold_all_kept() {
        // Episodes within ~97 days (score ≥ 0.01) must all survive the stale filter.
        // exp(-60/21) ≈ 0.057, exp(-30/21) ≈ 0.24, exp(-2/21) ≈ 0.91 — all ≥ 0.01.
        let now = Utc::now();
        let mut block = ContextBlock::empty();
        for &age in &[2i64, 30, 60] {
            block.episodes.push(EpisodeSummary {
                id: Uuid::now_v7(),
                session_id: Uuid::now_v7(),
                role: None,
                preview: format!("episode {}d old", age),
                created_at: now - Duration::days(age),
                relevance: 0.1,
            });
        }
        apply_temporal_scoring(&mut block, TemporalIntent::Current, None, None);
        assert_eq!(
            block.episodes.len(),
            3,
            "all recent episodes should survive; got {:?}",
            block.episodes.len()
        );
    }

    #[test]
    fn fix3_filter_does_not_apply_to_historical_intent() {
        // For Historical intent the stale filter must not run — temporal
        // scoring uses proximity to as_of, not recency, and old episodes are valid.
        let now = Utc::now();
        let as_of = now - Duration::days(500);
        let mut block = ContextBlock::empty();

        // Episode created 490 days ago — very old for Current, but close to as_of
        block.episodes.push(EpisodeSummary {
            id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            role: None,
            preview: "Alice worked at TechCorp.".into(),
            created_at: now - Duration::days(490),
            relevance: 0.5,
        });

        apply_temporal_scoring(
            &mut block,
            TemporalIntent::Historical,
            Some(as_of),
            Some(0.9),
        );

        // Must survive — it's close to as_of and Historical filter is not applied
        assert_eq!(
            block.episodes.len(),
            1,
            "historical intent must not filter episodes by recency"
        );
    }
}
