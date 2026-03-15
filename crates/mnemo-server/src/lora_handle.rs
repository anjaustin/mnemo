//! `LoraEmbedderHandle` — a concrete enum that unifies the base `EmbedderKind`
//! and the `LoraAdaptedEmbedder<EmbedderKind, RedisStateStore>` behind a single
//! type that implements `EmbeddingProvider`.
//!
//! This lets `RetrievalEngine` and `IngestWorker` remain monomorphic over a
//! single concrete embedder type regardless of whether TinyLoRA is enabled,
//! while keeping `AppState` free of extra generic parameters.
//!
//! When LoRA is **disabled**, `Base` is used and `embed_for_agent` / other
//! LoRA methods delegate to `EmbedderKind`'s default no-op implementations.
//!
//! When LoRA is **enabled**, `Lora` is used and all `embed_for_agent` calls
//! produce personalized vectors via the in-memory LoRA adapter cache.

use std::sync::Arc;

use mnemo_core::traits::llm::{EmbeddingProvider, LlmResult};
use mnemo_llm::EmbedderKind;
use mnemo_lora::LoraAdaptedEmbedder;
use mnemo_storage::RedisStateStore;
use uuid::Uuid;

/// A handle to whichever embedding backend is active.
///
/// Constructed once at server startup and shared via `Arc`.
pub enum LoraEmbedderHandle {
    /// LoRA disabled — pass-through to the base embedder.
    Base(Arc<EmbedderKind>),
    /// LoRA enabled — wraps the base embedder with a per-agent adapter layer.
    Lora(Arc<LoraAdaptedEmbedder<EmbedderKind, RedisStateStore>>),
}

impl EmbeddingProvider for LoraEmbedderHandle {
    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
        match self {
            LoraEmbedderHandle::Base(e) => e.embed(text).await,
            LoraEmbedderHandle::Lora(e) => e.embed(text).await,
        }
    }

    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
        match self {
            LoraEmbedderHandle::Base(e) => e.embed_batch(texts).await,
            LoraEmbedderHandle::Lora(e) => e.embed_batch(texts).await,
        }
    }

    fn dimensions(&self) -> u32 {
        match self {
            LoraEmbedderHandle::Base(e) => e.dimensions(),
            LoraEmbedderHandle::Lora(e) => e.dimensions(),
        }
    }

    fn provider_name(&self) -> &str {
        match self {
            LoraEmbedderHandle::Base(e) => e.provider_name(),
            LoraEmbedderHandle::Lora(e) => e.provider_name(),
        }
    }

    async fn embed_for_agent(
        &self,
        text: &str,
        user_id: Uuid,
        agent_id: Option<&str>,
    ) -> LlmResult<Vec<f32>> {
        match self {
            LoraEmbedderHandle::Base(e) => e.embed_for_agent(text, user_id, agent_id).await,
            LoraEmbedderHandle::Lora(e) => e.embed_for_agent(text, user_id, agent_id).await,
        }
    }

    async fn embed_batch_for_agent(
        &self,
        texts: &[String],
        user_id: Uuid,
        agent_id: Option<&str>,
    ) -> LlmResult<Vec<Vec<f32>>> {
        match self {
            LoraEmbedderHandle::Base(e) => e.embed_batch_for_agent(texts, user_id, agent_id).await,
            LoraEmbedderHandle::Lora(e) => e.embed_batch_for_agent(texts, user_id, agent_id).await,
        }
    }

    async fn update_lora_from_access(
        &self,
        v_query: &[f32],
        v_item: &[f32],
        user_id: Uuid,
        agent_id: Option<&str>,
    ) {
        match self {
            LoraEmbedderHandle::Base(e) => {
                e.update_lora_from_access(v_query, v_item, user_id, agent_id)
                    .await
            }
            LoraEmbedderHandle::Lora(e) => {
                e.update_lora_from_access(v_query, v_item, user_id, agent_id)
                    .await
            }
        }
    }

    async fn update_lora_with_rating(
        &self,
        v_query: &[f32],
        v_item: &[f32],
        rating: f32,
        user_id: Uuid,
        agent_id: Option<&str>,
    ) {
        match self {
            LoraEmbedderHandle::Base(e) => {
                e.update_lora_with_rating(v_query, v_item, rating, user_id, agent_id)
                    .await
            }
            LoraEmbedderHandle::Lora(e) => {
                e.update_lora_with_rating(v_query, v_item, rating, user_id, agent_id)
                    .await
            }
        }
    }
}
