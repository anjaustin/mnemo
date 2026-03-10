//! # mnemo-ingest
//!
//! The ingestion pipeline processes episodes through entity extraction,
//! graph construction, and embedding generation.
//!
//! Features:
//! - Atomic episode claiming (safe for multiple replicas)
//! - Automatic entity deduplication against existing graph
//! - Contradiction detection and edge invalidation
//! - Exponential backoff retry on transient failures

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::sleep;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::models::edge::{Edge, EdgeFilter};
use mnemo_core::models::entity::{Entity, ExtractedEntity};
use mnemo_core::models::episode::Episode;
use mnemo_core::models::session::UpdateSessionRequest;
use mnemo_core::traits::llm::{EmbeddingProvider, LlmProvider};
use mnemo_core::traits::storage::{
    DigestStore, EdgeStore, EntityStore, EpisodeStore, SessionStore, SpanStore, StorageResult,
    VectorStore,
};

/// Re-export `LlmSpan` from mnemo-core so callers can use `mnemo_ingest::LlmSpan`.
pub use mnemo_core::models::span::LlmSpan;

/// Shared span sink. The server creates this (same VecDeque as AppState.llm_spans)
/// and passes it to the worker so ingest spans appear alongside route spans.
pub type SpanSink = Arc<RwLock<VecDeque<LlmSpan>>>;

/// Maximum spans retained in the ring buffer (must match MAX_LLM_SPANS in routes.rs).
const MAX_SPANS: usize = 500;

fn episode_request_id(episode: &Episode) -> Option<&str> {
    episode
        .metadata
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

/// Configuration for the ingestion pipeline.
pub struct IngestConfig {
    /// How often to poll for pending episodes (ms).
    pub poll_interval_ms: u64,
    /// Max episodes to process per poll cycle.
    pub batch_size: u32,
    /// Max concurrent extraction tasks.
    pub concurrency: usize,
    /// Max retries for failed episodes before marking as permanently failed.
    pub max_retries: u32,
    /// Number of episodes after which to trigger progressive session summarization.
    /// Set to 0 to disable. Default: 10.
    pub session_summary_threshold: u32,
    /// Enable background sleep-time compute. When true, the worker generates
    /// memory digests for users after they have been idle.
    pub sleep_enabled: bool,
    /// Seconds of user inactivity before triggering background digest generation.
    pub sleep_idle_window_seconds: u64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 500,
            batch_size: 10,
            concurrency: 4,
            max_retries: 3,
            session_summary_threshold: 10,
            sleep_enabled: true,
            sleep_idle_window_seconds: 300,
        }
    }
}

/// Re-export from mnemo-core so downstream crates can use `mnemo_ingest::MemoryDigest`.
pub use mnemo_core::models::digest::MemoryDigest;

/// Shared digest cache type. The server creates this and passes it to the worker.
pub type DigestCache = Arc<RwLock<HashMap<Uuid, MemoryDigest>>>;

/// The ingestion pipeline worker.
///
/// Runs as a background task, continuously polling for pending episodes
/// and processing them through the extraction → graph construction pipeline.
/// When sleep-time compute is enabled, it also detects user idle windows
/// and generates memory digests in the background.
pub struct IngestWorker<S, V, L, E>
where
    S: EpisodeStore + EntityStore + EdgeStore + SessionStore + DigestStore + SpanStore,
    V: VectorStore,
    L: LlmProvider,
    E: EmbeddingProvider,
{
    state_store: Arc<S>,
    vector_store: Arc<V>,
    llm: Arc<L>,
    embedder: Arc<E>,
    config: IngestConfig,
    /// Shared digest cache (same Arc as AppState.memory_digests).
    digest_cache: Option<DigestCache>,
    /// Shared span ring buffer (same Arc as AppState.llm_spans).
    span_sink: Option<SpanSink>,
    /// Per-user last-activity tracking for idle detection.
    user_activity: RwLock<HashMap<Uuid, Instant>>,
    /// Users whose digest has already been generated this idle window.
    /// Cleared when the user becomes active again.
    digest_generated: RwLock<std::collections::HashSet<Uuid>>,
}

impl<S, V, L, E> IngestWorker<S, V, L, E>
where
    S: EpisodeStore + EntityStore + EdgeStore + SessionStore + DigestStore + SpanStore + Send + Sync + 'static,
    V: VectorStore + Send + Sync + 'static,
    L: LlmProvider + Send + Sync + 'static,
    E: EmbeddingProvider + Send + Sync + 'static,
{
    pub fn new(
        state_store: Arc<S>,
        vector_store: Arc<V>,
        llm: Arc<L>,
        embedder: Arc<E>,
        config: IngestConfig,
    ) -> Self {
        Self {
            state_store,
            vector_store,
            llm,
            embedder,
            config,
            digest_cache: None,
            span_sink: None,
            user_activity: RwLock::new(HashMap::new()),
            digest_generated: RwLock::new(std::collections::HashSet::new()),
        }
    }

    /// Attach the shared digest cache so the worker can write digests that
    /// are immediately visible via `GET /api/v1/memory/:user/digest`.
    pub fn with_digest_cache(mut self, cache: DigestCache) -> Self {
        self.digest_cache = Some(cache);
        self
    }

    /// Attach the shared LLM span sink so ingest-time spans appear alongside
    /// route-time spans in `GET /api/v1/spans/*`.
    pub fn with_span_sink(mut self, sink: SpanSink) -> Self {
        self.span_sink = Some(sink);
        self
    }

    /// Record an LLM span into the shared ring buffer (if attached) and
    /// persist to Redis via the SpanStore.
    async fn record_span(&self, span: LlmSpan) {
        // Persist to Redis (best-effort — don't fail the pipeline on span storage errors)
        if let Err(e) = self.state_store.save_span(&span).await {
            tracing::warn!("Failed to persist LLM span to Redis: {e}");
        }

        // Also push to the in-memory ring buffer for backward compatibility
        if let Some(ref sink) = self.span_sink {
            let mut spans = sink.write().await;
            if spans.len() >= MAX_SPANS {
                spans.pop_front();
            }
            spans.push_back(span);
        }
    }

    /// Run the ingestion loop. Call this in a tokio::spawn.
    pub async fn run(&self) {
        tracing::info!(
            "Ingestion worker started (max_retries={}, sleep_enabled={}, idle_window={}s)",
            self.config.max_retries,
            self.config.sleep_enabled,
            self.config.sleep_idle_window_seconds,
        );
        loop {
            match self.poll_and_process().await {
                Ok(n) if n > 0 => tracing::debug!(processed = n, "Ingestion cycle"),
                Err(e) => tracing::error!(error = %e, "Ingestion cycle failed"),
                _ => {}
            }
            // Always check for idle users, even when episodes were processed.
            // Other users may have gone idle while this cycle handled someone else's data.
            if self.config.sleep_enabled {
                self.sleep_time_consolidation().await;
            }
            sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
        }
    }

    /// Record that a user was active (episode processed).
    async fn record_user_activity(&self, user_id: Uuid) {
        {
            let mut activity = self.user_activity.write().await;
            activity.insert(user_id, Instant::now());
        } // drop write lock before acquiring the next one
        {
            // Clear the digest-generated flag so a new digest can be triggered
            // after the next idle window.
            let mut generated = self.digest_generated.write().await;
            generated.remove(&user_id);
        }
    }

    /// Check all tracked users for idle windows and generate digests.
    async fn sleep_time_consolidation(&self) {
        // Clamp minimum idle window to 30s to prevent runaway LLM calls
        let idle_secs = self.config.sleep_idle_window_seconds.max(30);
        let idle_threshold = Duration::from_secs(idle_secs);
        // Evict entries older than 24h to prevent unbounded growth
        let eviction_threshold = Duration::from_secs(86400);

        let idle_users: Vec<Uuid>;
        {
            let activity = self.user_activity.read().await;
            let generated = self.digest_generated.read().await;
            idle_users = activity
                .iter()
                .filter(|(uid, last)| last.elapsed() >= idle_threshold && !generated.contains(uid))
                .map(|(uid, _)| *uid)
                .collect();
        }

        // Evict stale entries (users inactive for >24h)
        {
            let mut activity = self.user_activity.write().await;
            activity.retain(|_, last| last.elapsed() < eviction_threshold);
        }
        {
            let activity = self.user_activity.read().await;
            let mut generated = self.digest_generated.write().await;
            generated.retain(|uid| activity.contains_key(uid));
        }

        for user_id in idle_users {
            tracing::info!(user_id = %user_id, "Sleep-time compute: generating digest for idle user");
            match self.generate_digest(user_id).await {
                Ok(digest) => {
                    // Persist to Redis first — only populate the in-memory cache
                    // and mark as generated if persistence succeeds, so that a
                    // retry occurs on the next idle cycle if Redis is down.
                    match self.state_store.save_digest(&digest).await {
                        Ok(()) => {
                            if let Some(ref cache) = self.digest_cache {
                                let mut digests = cache.write().await;
                                digests.insert(user_id, digest);
                            }
                            let mut gen = self.digest_generated.write().await;
                            gen.insert(user_id);
                            tracing::info!(user_id = %user_id, "Sleep-time digest generated and persisted");
                        }
                        Err(e) => {
                            tracing::warn!(
                                user_id = %user_id, error = %e,
                                "Sleep-time digest generated but Redis persistence failed; will retry next cycle"
                            );
                            // Do NOT mark as generated — allows retry on next idle check.
                            // Do NOT populate in-memory cache — avoids cache/Redis split-brain.
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(user_id = %user_id, error = %e, "Sleep-time digest generation failed");
                }
            }
        }
    }

    /// Generate a memory digest for a user (same logic as the HTTP handler).
    async fn generate_digest(&self, user_id: Uuid) -> Result<MemoryDigest, MnemoError> {
        let entities = self.state_store.list_entities(user_id, 200, None).await?;
        let filter = EdgeFilter {
            include_invalidated: false,
            limit: 300,
            ..Default::default()
        };
        let edges = self.state_store.query_edges(user_id, filter).await?;

        let entity_count = entities.len();
        let edge_count = edges.len();

        if entity_count == 0 {
            return Err(MnemoError::NotFound {
                resource_type: "entities".to_string(),
                id: user_id.to_string(),
            });
        }

        let entity_lines: Vec<String> = entities
            .iter()
            .take(80)
            .map(|e| {
                if let Some(ref s) = e.summary {
                    format!("- {} ({}): {}", e.name, e.entity_type.as_str(), s)
                } else {
                    format!("- {} ({})", e.name, e.entity_type.as_str())
                }
            })
            .collect();
        let edge_lines: Vec<String> = edges
            .iter()
            .take(60)
            .map(|e| format!("- {}", e.fact))
            .collect();

        let prompt = format!(
            "You are analyzing a user's long-term memory knowledge graph.\n\
            Entities ({} total, showing up to 80):\n{}\n\n\
            Key relationships ({} total, showing up to 60):\n{}\n\n\
            Write a concise 2-4 sentence prose summary of what this person knows, \
            their main areas of interest, and any dominant themes. \
            Then on a new line write: TOPICS: topic1, topic2, topic3 (list 3-6 key topics).",
            entity_count,
            entity_lines.join("\n"),
            edge_count,
            edge_lines.join("\n"),
        );

        let model_name = self.llm.model_name().to_string();
        let digest_start = chrono::Utc::now();
        let digest_t0 = Instant::now();
        let digest_result = self.llm.summarize(&prompt, 512).await;
        let digest_elapsed = digest_t0.elapsed();
        let digest_ok = digest_result.is_ok();
        self.record_span(LlmSpan {
            id: Uuid::now_v7(),
            request_id: None,
            user_id: Some(user_id),
            provider: self.llm.provider_name().to_string(),
            model: model_name.clone(),
            operation: "digest".to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            latency_ms: digest_elapsed.as_millis() as u64,
            success: digest_ok,
            error: if digest_ok {
                None
            } else {
                Some(digest_result.as_ref().err().unwrap().to_string())
            },
            started_at: digest_start,
            finished_at: chrono::Utc::now(),
        })
        .await;
        let raw = digest_result?;

        let (summary_text, dominant_topics) = if let Some(idx) = raw.find("TOPICS:") {
            let summary = raw[..idx].trim().to_string();
            let topics_raw = raw[idx + 7..].trim();
            let topics: Vec<String> = topics_raw
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .take(6)
                .collect();
            (summary, topics)
        } else {
            (raw.trim().to_string(), Vec::new())
        };

        Ok(MemoryDigest {
            user_id,
            summary: summary_text,
            entity_count,
            edge_count,
            dominant_topics,
            generated_at: chrono::Utc::now(),
            model: model_name,
        })
    }

    /// Poll for pending episodes and process them.
    async fn poll_and_process(&self) -> StorageResult<usize> {
        let pending = self
            .state_store
            .get_pending_episodes(self.config.batch_size)
            .await?;
        let mut processed = 0;
        for episode in pending {
            if !self.state_store.claim_episode(episode.id).await? {
                continue;
            }
            match self.process_episode(&episode).await {
                Ok(_) => processed += 1,
                Err(e) => {
                    self.handle_failure(episode, e).await;
                }
            }
        }
        Ok(processed)
    }

    /// Handle a processing failure with retry logic.
    async fn handle_failure(&self, episode: Episode, error: MnemoError) {
        let mut ep = episode.clone();
        match ep.requeue_for_retry(error.to_string(), self.config.max_retries) {
            Some(delay_ms) => {
                // Transient failure — schedule retry
                tracing::warn!(
                    episode_id = %ep.id,
                    request_id = ?episode_request_id(&ep),
                    retry = ep.retry_count,
                    delay_ms = delay_ms,
                    error = %error,
                    "Episode processing failed, scheduling retry"
                );
                if let Err(e) = self.state_store.update_episode(&ep).await {
                    tracing::error!(error = %e, "Failed to update episode for retry");
                    return;
                }
                if let Err(e) = self.state_store.requeue_episode(ep.id, delay_ms).await {
                    tracing::error!(error = %e, "Failed to requeue episode");
                }
            }
            None => {
                // Max retries exceeded — permanent failure
                tracing::error!(
                    episode_id = %ep.id,
                    request_id = ?episode_request_id(&ep),
                    retries = ep.retry_count,
                    error = %error,
                    "Episode permanently failed after max retries"
                );
                let _ = self.state_store.update_episode(&ep).await;
            }
        }
    }

    /// Process a single episode through the full pipeline.
    async fn process_episode(&self, episode: &Episode) -> StorageResult<()> {
        tracing::debug!(episode_id = %episode.id, request_id = ?episode_request_id(episode), "Processing episode");
        // Track user activity for sleep-time idle detection
        self.record_user_activity(episode.user_id).await;
        // 1. Get existing entities for dedup hints
        let existing = self
            .state_store
            .list_entities(episode.user_id, 100, None)
            .await?;
        let hints: Vec<ExtractedEntity> = existing
            .iter()
            .map(|e| ExtractedEntity {
                name: e.name.clone(),
                entity_type: e.entity_type.clone(),
                summary: e.summary.clone(),
            })
            .collect();

        // 2. Extract via LLM
        let extract_start = chrono::Utc::now();
        let extract_t0 = Instant::now();
        let extraction = self
            .llm
            .extract_entities_and_relationships(&episode.content, &hints)
            .await;
        let extract_elapsed = extract_t0.elapsed();
        let extract_ok = extraction.is_ok();
        self.record_span(LlmSpan {
            id: Uuid::now_v7(),
            request_id: episode_request_id(episode).map(String::from),
            user_id: Some(episode.user_id),
            provider: self.llm.provider_name().to_string(),
            model: self.llm.model_name().to_string(),
            operation: "extract".to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            latency_ms: extract_elapsed.as_millis() as u64,
            success: extract_ok,
            error: if extract_ok {
                None
            } else {
                Some(extraction.as_ref().err().unwrap().to_string())
            },
            started_at: extract_start,
            finished_at: chrono::Utc::now(),
        })
        .await;
        let extraction = extraction?;

        // 3. Resolve entities (dedup against existing graph)
        let mut name_to_id: std::collections::HashMap<String, Uuid> =
            std::collections::HashMap::new();
        let mut new_entity_ids = Vec::new();

        for ext in &extraction.entities {
            let existing = self
                .state_store
                .find_entity_by_name(episode.user_id, &ext.name)
                .await?;
            let id = if let Some(mut e) = existing {
                e.record_mention();
                if e.summary.is_none() {
                    if let Some(ref s) = ext.summary {
                        e.update_summary(s.clone());
                    }
                }
                self.state_store.update_entity(&e).await?;
                e.id
            } else {
                let entity = Entity::from_extraction(ext, episode.user_id, episode.id);
                let created = self.state_store.create_entity(entity).await?;
                new_entity_ids.push(created.id);
                let emb = self
                    .embedder
                    .embed(&format!(
                        "{} ({})",
                        created.name,
                        created.entity_type.as_str()
                    ))
                    .await?;
                self.vector_store.upsert_entity_embedding(
                    created.id, created.user_id, emb,
                    serde_json::json!({"name": created.name, "entity_type": created.entity_type.as_str()}),
                ).await?;
                created.id
            };
            name_to_id.insert(ext.name.to_lowercase(), id);
        }

        // 4. Create edges (resolve names to IDs, invalidate conflicts)
        let mut new_edge_ids = Vec::new();
        for rel in &extraction.relationships {
            let src = name_to_id.get(&rel.source_name.to_lowercase()).copied();
            let tgt = name_to_id.get(&rel.target_name.to_lowercase()).copied();
            let (src, tgt) = match (src, tgt) {
                (Some(s), Some(t)) => (s, t),
                _ => continue,
            };

            for mut c in self
                .state_store
                .find_conflicting_edges(episode.user_id, src, tgt, &rel.label)
                .await?
            {
                c.invalidate(episode.id);
                self.state_store.update_edge(&c).await?;
            }

            let edge = Edge::from_extraction(
                rel,
                episode.user_id,
                src,
                tgt,
                episode.id,
                episode.created_at,
            );
            let created = self.state_store.create_edge(edge).await?;
            new_edge_ids.push(created.id);
            let emb = self.embedder.embed(&created.fact).await?;
            self.vector_store
                .upsert_edge_embedding(
                    created.id,
                    created.user_id,
                    emb,
                    serde_json::json!({"label": created.label, "fact": created.fact}),
                )
                .await?;
        }

        // 5. Episode embedding
        let embed_start = chrono::Utc::now();
        let embed_t0 = Instant::now();
        let ep_emb = self.embedder.embed(&episode.content).await;
        let embed_elapsed = embed_t0.elapsed();
        let embed_ok = ep_emb.is_ok();
        self.record_span(LlmSpan {
            id: Uuid::now_v7(),
            request_id: episode_request_id(episode).map(String::from),
            user_id: Some(episode.user_id),
            provider: self.embedder.provider_name().to_string(),
            model: self.embedder.provider_name().to_string(),
            operation: "embed_episode".to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            latency_ms: embed_elapsed.as_millis() as u64,
            success: embed_ok,
            error: if embed_ok {
                None
            } else {
                Some(ep_emb.as_ref().err().unwrap().to_string())
            },
            started_at: embed_start,
            finished_at: chrono::Utc::now(),
        })
        .await;
        let ep_emb = ep_emb?;
        self.vector_store
            .upsert_episode_embedding(
                episode.id,
                episode.user_id,
                ep_emb,
                serde_json::json!({"session_id": episode.session_id.to_string()}),
            )
            .await?;

        // 6. Mark completed
        let mut done = episode.clone();
        done.mark_completed(new_entity_ids, new_edge_ids);
        self.state_store.update_episode(&done).await?;
        tracing::debug!(episode_id = %episode.id, request_id = ?episode_request_id(episode), "Episode completed");

        // 7. Progressive session summarization
        //    Runs if threshold is enabled and episode_count is a multiple of the threshold.
        let threshold = self.config.session_summary_threshold;
        if threshold > 0 {
            if let Ok(session) = self.state_store.get_session(episode.session_id).await {
                if session.episode_count > 0 && session.episode_count % u64::from(threshold) == 0 {
                    tracing::debug!(
                        session_id = %episode.session_id,
                        episode_count = session.episode_count,
                        threshold,
                        "Triggering progressive session summarization"
                    );
                    let sum_start = chrono::Utc::now();
                    let sum_t0 = Instant::now();
                    let sum_result = self.llm.summarize(&episode.content, 256).await;
                    let sum_elapsed = sum_t0.elapsed();
                    let sum_ok = sum_result.is_ok();
                    self.record_span(LlmSpan {
                        id: Uuid::now_v7(),
                        request_id: episode_request_id(episode).map(String::from),
                        user_id: Some(episode.user_id),
                        provider: self.llm.provider_name().to_string(),
                        model: self.llm.model_name().to_string(),
                        operation: "session_summarize".to_string(),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                        latency_ms: sum_elapsed.as_millis() as u64,
                        success: sum_ok,
                        error: if sum_ok {
                            None
                        } else {
                            Some(sum_result.as_ref().err().unwrap().to_string())
                        },
                        started_at: sum_start,
                        finished_at: chrono::Utc::now(),
                    })
                    .await;
                    match sum_result {
                        Ok(summary_text) => {
                            // Rough token count: ~4 chars/token
                            let tokens = (summary_text.len() / 4).max(1) as u32;
                            let update = UpdateSessionRequest {
                                summary: Some(summary_text),
                                summary_tokens: Some(tokens),
                                ..Default::default()
                            };
                            if let Err(e) = self
                                .state_store
                                .update_session(episode.session_id, update)
                                .await
                            {
                                // Non-fatal: log and continue
                                tracing::warn!(
                                    session_id = %episode.session_id,
                                    error = %e,
                                    "Failed to persist session summary"
                                );
                            }
                        }
                        Err(e) => {
                            // Non-fatal: summarization failure must not block ingest
                            tracing::warn!(
                                session_id = %episode.session_id,
                                error = %e,
                                "Session summarization LLM call failed"
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnemo_core::models::episode::{
        CreateEpisodeRequest, EpisodeType, MessageRole, ProcessingStatus,
    };

    #[test]
    fn test_retry_backoff_sequence() {
        let req = CreateEpisodeRequest {
            id: None,
            episode_type: EpisodeType::Message,
            content: "test".to_string(),
            role: Some(MessageRole::User),
            name: None,
            metadata: serde_json::json!({}),
            created_at: None,
        };
        let mut ep = Episode::from_request(req, Uuid::now_v7(), Uuid::now_v7());

        // Retry 1: 500ms delay
        let delay = ep.requeue_for_retry("timeout".into(), 3);
        assert_eq!(delay, Some(500));
        assert_eq!(ep.retry_count, 1);
        assert_eq!(ep.processing_status, ProcessingStatus::Pending);

        // Retry 2: 1000ms delay
        let delay = ep.requeue_for_retry("timeout".into(), 3);
        assert_eq!(delay, Some(1000));
        assert_eq!(ep.retry_count, 2);

        // Retry 3: 2000ms delay
        let delay = ep.requeue_for_retry("timeout".into(), 3);
        assert_eq!(delay, Some(2000));
        assert_eq!(ep.retry_count, 3);

        // Retry 4: exceeded max_retries → permanent failure
        let delay = ep.requeue_for_retry("timeout".into(), 3);
        assert_eq!(delay, None);
        assert_eq!(ep.processing_status, ProcessingStatus::Failed);
        assert_eq!(ep.retry_count, 3); // doesn't increment past max
    }
}
