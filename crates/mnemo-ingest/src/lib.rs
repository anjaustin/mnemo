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

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::models::edge::Edge;
use mnemo_core::models::entity::{Entity, ExtractedEntity};
use mnemo_core::models::episode::Episode;
use mnemo_core::traits::llm::{EmbeddingProvider, LlmProvider};
use mnemo_core::traits::storage::{
    EdgeStore, EntityStore, EpisodeStore, StorageResult, VectorStore,
};

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
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 500,
            batch_size: 10,
            concurrency: 4,
            max_retries: 3,
        }
    }
}

/// The ingestion pipeline worker.
///
/// Runs as a background task, continuously polling for pending episodes
/// and processing them through the extraction → graph construction pipeline.
pub struct IngestWorker<S, V, L, E>
where
    S: EpisodeStore + EntityStore + EdgeStore,
    V: VectorStore,
    L: LlmProvider,
    E: EmbeddingProvider,
{
    state_store: Arc<S>,
    vector_store: Arc<V>,
    llm: Arc<L>,
    embedder: Arc<E>,
    config: IngestConfig,
}

impl<S, V, L, E> IngestWorker<S, V, L, E>
where
    S: EpisodeStore + EntityStore + EdgeStore + Send + Sync + 'static,
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
        }
    }

    /// Run the ingestion loop. Call this in a tokio::spawn.
    pub async fn run(&self) {
        tracing::info!(
            "Ingestion worker started (max_retries={})",
            self.config.max_retries
        );
        loop {
            match self.poll_and_process().await {
                Ok(n) if n > 0 => tracing::debug!(processed = n, "Ingestion cycle"),
                Err(e) => tracing::error!(error = %e, "Ingestion cycle failed"),
                _ => {}
            }
            sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
        }
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
        let extraction = self
            .llm
            .extract_entities_and_relationships(&episode.content, &hints)
            .await?;

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
        let ep_emb = self.embedder.embed(&episode.content).await?;
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
