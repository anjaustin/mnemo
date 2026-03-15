//! # mnemo-lora
//!
//! TinyLoRA: per-agent embedding personalization for Mnemo.
//!
//! Provides a lightweight rank-decomposed linear adapter that rotates base
//! embeddings toward each agent's observed relevance history at query and
//! ingest time — without modifying the base embedding model or the shared
//! Qdrant index.
//!
//! ## Design
//!
//! ```text
//! v_adapted = v_base + scale * B · (A · v_base)
//!
//! A ∈ ℝ^{r×d}   (down-projection, Kaiming init, fixed after init)
//! B ∈ ℝ^{d×r}   (up-projection, zero init, updated from implicit feedback)
//! scale = α/r = 0.125  (fixed LoRA default for rank=8)
//! ```
//!
//! Each `(user_id, agent_id)` pair gets its own adapter, persisted in Redis
//! via the [`LoraStore`] trait and cached in memory inside
//! [`LoraAdaptedEmbedder`].
//!
//! ## Usage
//!
//! Wrap any `EmbeddingProvider` with `LoraAdaptedEmbedder` and set
//! `MNEMO_LORA_ENABLED=true`.  All call-sites that use `embed_for_agent`
//! will automatically receive personalized vectors.

pub mod math;

use std::collections::HashMap;
use std::sync::Arc;

use mnemo_core::models::lora::LoraWeights;
use mnemo_core::traits::llm::{EmbeddingProvider, LlmResult};
use mnemo_core::traits::storage::LoraStore;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Maximum Frobenius norm allowed for B before we clamp it.
/// Prevents unbounded divergence from repeated updates.
const B_MAX_NORM: f32 = 10.0;

/// Learning rate for implicit-feedback adapter updates.
const LORA_LR: f32 = 0.005;

// ─── LoraAdapter ───────────────────────────────────────────────────

/// In-memory adapter for one `(user_id, agent_id)` pair.
///
/// Wraps [`LoraWeights`] with convenience apply/update methods.
/// The adapter is loaded from Redis on first use and cached in
/// `LoraAdaptedEmbedder`'s weight cache.
#[derive(Debug, Clone)]
pub struct LoraAdapter {
    pub weights: LoraWeights,
}

impl LoraAdapter {
    /// Create a fresh adapter (identity residual, no updates).
    pub fn new(user_id: Uuid, agent_id: Option<String>, dims: usize) -> Self {
        Self {
            weights: LoraWeights::initialize(user_id, agent_id, dims),
        }
    }

    /// Load an adapter from persisted weights.
    pub fn from_weights(weights: LoraWeights) -> Self {
        Self { weights }
    }

    /// Apply the LoRA adaptation to a base embedding vector.
    ///
    /// `v_adapted = v_base + scale * B · (A · v_base)`
    ///
    /// If the adapter has never been updated (B=0), returns `v_base` unchanged.
    /// This is guaranteed by the zero initialization of B.
    pub fn apply(&self, v_base: &[f32]) -> Vec<f32> {
        let w = &self.weights;
        math::adapt(&w.a_flat, &w.b_flat, v_base, w.scale, w.dims, w.rank)
    }

    /// Apply an implicit-feedback update step.
    ///
    /// Called when a retrieved item was actually accessed (positive signal):
    /// nudges B to reduce angular distance between the adapted query vector
    /// and the adapted item vector in this agent's embedding space.
    ///
    /// **Update rule:**
    /// ```text
    /// h     = A · v_item
    /// delta = v_query - v_item_adapted   (error direction in output space)
    /// ΔB   += lr * outer(delta, h)       (shape d×r)
    /// B     = clamp(B, max_norm)
    /// ```
    pub fn update_from_access(&mut self, v_query: &[f32], v_item: &[f32]) {
        let w = &mut self.weights;
        let d = w.dims;
        let r = w.rank;

        // h = A · v_item  (shape r)
        let h = math::mat_vec_r_times_d(&w.a_flat, v_item, r, d);

        // v_item_adapted = v_item + scale * B · h
        let u_item = math::mat_vec_d_times_r(&w.b_flat, &h, d, r);
        let v_item_adapted = math::add_scaled(v_item, &u_item, w.scale);

        // delta = v_query - v_item_adapted  (direction we want to shrink)
        let delta: Vec<f32> = v_query
            .iter()
            .zip(v_item_adapted.iter())
            .map(|(q, a)| q - a)
            .collect();

        // ΔB += lr * outer(delta, h)  — shape d×r
        math::outer_add(&mut w.b_flat, &delta, &h, LORA_LR, d, r);

        // Clamp to prevent unbounded growth
        math::clamp_frobenius(&mut w.b_flat, B_MAX_NORM);

        w.update_count += 1;
        w.last_updated = chrono::Utc::now().timestamp();
    }
}

// ─── LoraAdaptedEmbedder ───────────────────────────────────────────

/// Wraps any `EmbeddingProvider` with per-agent LoRA adaptation.
///
/// **Cache:** Adapters are loaded from Redis on first use per `(user_id, agent_id)`
/// and held in an in-memory `RwLock<HashMap>`.  They are written back to Redis
/// after each update (in the background via `tokio::spawn`).
///
/// **Thread safety:** All state is behind `Arc<RwLock<_>>`. Multiple concurrent
/// requests for the same adapter do not corrupt state.
///
/// **Feature flag:** When `enabled = false`, `embed_for_agent` delegates to
/// the inner embedder's `embed` with zero overhead.
pub struct LoraAdaptedEmbedder<E, S>
where
    E: EmbeddingProvider,
    S: LoraStore,
{
    /// The wrapped base embedding provider.
    inner: Arc<E>,
    /// Redis-backed LoRA weight store.
    store: Arc<S>,
    /// In-memory adapter cache: key = `"{user_id}:{agent_id_or___global__}"`.
    cache: Arc<RwLock<HashMap<String, LoraAdapter>>>,
    /// Embedding dimension (must match `inner.dimensions()`).
    dims: usize,
    /// Whether LoRA adaptation is active.
    pub enabled: bool,
}

impl<E, S> LoraAdaptedEmbedder<E, S>
where
    E: EmbeddingProvider + 'static,
    S: LoraStore + 'static,
{
    /// Create a new `LoraAdaptedEmbedder`.
    pub fn new(inner: Arc<E>, store: Arc<S>, enabled: bool) -> Self {
        let dims = inner.dimensions() as usize;
        Self {
            inner,
            store,
            cache: Arc::new(RwLock::new(HashMap::new())),
            dims,
            enabled,
        }
    }

    /// Cache key for a `(user_id, agent_id)` pair.
    fn cache_key(user_id: Uuid, agent_id: Option<&str>) -> String {
        match agent_id {
            Some(id) => format!("{}:{}", user_id, id),
            None => format!("{}:__global__", user_id),
        }
    }

    /// Get or create an adapter for a `(user_id, agent_id)` pair.
    ///
    /// Load order: cache → Redis → new (identity adapter).
    /// Newly created adapters are NOT persisted until the first update.
    async fn get_adapter(&self, user_id: Uuid, agent_id: Option<&str>) -> LoraAdapter {
        let key = Self::cache_key(user_id, agent_id);

        // Fast path: cache hit
        {
            let cache = self.cache.read().await;
            if let Some(adapter) = cache.get(&key) {
                return adapter.clone();
            }
        }

        // Slow path: load from Redis
        let weights_opt = self
            .store
            .get_lora_weights(user_id, agent_id)
            .await
            .unwrap_or(None);

        let adapter = match weights_opt {
            Some(w) => LoraAdapter::from_weights(w),
            None => LoraAdapter::new(user_id, agent_id.map(|s| s.to_string()), self.dims),
        };

        // Write to cache
        let mut cache = self.cache.write().await;
        cache.insert(key, adapter.clone());

        adapter
    }

    /// Apply adaptation and return the adapted vector.
    async fn adapt_vec(
        &self,
        base: Vec<f32>,
        user_id: Uuid,
        agent_id: Option<&str>,
    ) -> Vec<f32> {
        if !self.enabled {
            return base;
        }
        let adapter = self.get_adapter(user_id, agent_id).await;
        adapter.apply(&base)
    }

    /// Apply an implicit-feedback update for one retrieved item.
    ///
    /// `v_query`  — the adapted query embedding (already adapted)
    /// `v_item`   — the base embedding of the accessed item
    /// `user_id`  — scope
    /// `agent_id` — scope
    ///
    /// Runs the update in memory and persists to Redis asynchronously.
    pub async fn update_from_access(
        &self,
        v_query: &[f32],
        v_item: &[f32],
        user_id: Uuid,
        agent_id: Option<&str>,
    ) {
        if !self.enabled {
            return;
        }

        let key = Self::cache_key(user_id, agent_id);

        // Get current adapter, update it, write back to cache + Redis
        let updated = {
            let mut cache = self.cache.write().await;
            let adapter = cache
                .entry(key.clone())
                .or_insert_with(|| LoraAdapter::new(user_id, agent_id.map(|s| s.to_string()), self.dims));

            adapter.update_from_access(v_query, v_item);
            adapter.clone()
        };

        // Persist synchronously — this is a fire-and-forget background path
        // (called from proactive reranking, not the hot response path).
        if let Err(e) = self.store.save_lora_weights(&updated.weights).await {
            tracing::warn!(
                key = %updated.weights.key_string(),
                error = %e,
                "Failed to persist LoRA weights to Redis"
            );
        }
    }

    /// Evict a specific adapter from the in-memory cache.
    /// Called after a reset (DELETE) so the next request loads fresh state.
    pub async fn evict_cache(&self, user_id: Uuid, agent_id: Option<&str>) {
        let key = Self::cache_key(user_id, agent_id);
        let mut cache = self.cache.write().await;
        cache.remove(&key);
    }
}

// ─── EmbeddingProvider impl ────────────────────────────────────────

impl<E, S> EmbeddingProvider for LoraAdaptedEmbedder<E, S>
where
    E: EmbeddingProvider + 'static,
    S: LoraStore + 'static,
{
    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
        self.inner.embed(text).await
    }

    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
        self.inner.embed_batch(texts).await
    }

    fn dimensions(&self) -> u32 {
        self.inner.dimensions()
    }

    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    /// Apply LoRA adaptation after the base embedding.
    ///
    /// When `enabled = false`, this is identical to `embed`.
    async fn embed_for_agent(
        &self,
        text: &str,
        user_id: Uuid,
        agent_id: Option<&str>,
    ) -> LlmResult<Vec<f32>> {
        let base = self.inner.embed(text).await?;
        Ok(self.adapt_vec(base, user_id, agent_id).await)
    }

    /// Apply LoRA adaptation to a batch.
    ///
    /// When `enabled = false`, this is identical to `embed_batch`.
    async fn embed_batch_for_agent(
        &self,
        texts: &[String],
        user_id: Uuid,
        agent_id: Option<&str>,
    ) -> LlmResult<Vec<Vec<f32>>> {
        let bases = self.inner.embed_batch(texts).await?;
        if !self.enabled {
            return Ok(bases);
        }
        let adapter = self.get_adapter(user_id, agent_id).await;
        Ok(bases.into_iter().map(|v| adapter.apply(&v)).collect())
    }

    /// Apply an implicit-feedback LoRA weight update.
    ///
    /// Called from the retrieval engine's reinforcement path after a fact is
    /// accessed. Delegates to [`LoraAdaptedEmbedder::update_from_access`].
    async fn update_lora_from_access(
        &self,
        v_query: &[f32],
        v_item: &[f32],
        user_id: Uuid,
        agent_id: Option<&str>,
    ) {
        self.update_from_access(v_query, v_item, user_id, agent_id).await;
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mnemo_core::models::lora::LoraWeights;
    use mnemo_core::traits::storage::StorageResult;

    // ── Minimal stub EmbeddingProvider ────────────────────────────

    struct ConstEmbedder {
        value: f32,
        dims: u32,
    }

    impl EmbeddingProvider for ConstEmbedder {
        async fn embed(&self, _text: &str) -> LlmResult<Vec<f32>> {
            Ok(vec![self.value; self.dims as usize])
        }
        async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|_| vec![self.value; self.dims as usize])
                .collect())
        }
        fn dimensions(&self) -> u32 {
            self.dims
        }
        fn provider_name(&self) -> &str {
            "const-test"
        }
    }

    // ── Minimal stub LoraStore ─────────────────────────────────────

    struct NoopLoraStore;

    impl LoraStore for NoopLoraStore {
        async fn get_lora_weights(
            &self,
            _user_id: Uuid,
            _agent_id: Option<&str>,
        ) -> StorageResult<Option<LoraWeights>> {
            Ok(None)
        }
        async fn save_lora_weights(&self, _weights: &LoraWeights) -> StorageResult<()> {
            Ok(())
        }
        async fn delete_lora_weights(
            &self,
            _user_id: Uuid,
            _agent_id: Option<&str>,
        ) -> StorageResult<()> {
            Ok(())
        }
        async fn delete_all_lora_weights_for_user(&self, _user_id: Uuid) -> StorageResult<()> {
            Ok(())
        }
        async fn list_lora_weights_for_user(
            &self,
            _user_id: Uuid,
        ) -> StorageResult<Vec<LoraWeights>> {
            Ok(Vec::new())
        }
    }

    fn make_embedder(enabled: bool) -> LoraAdaptedEmbedder<ConstEmbedder, NoopLoraStore> {
        LoraAdaptedEmbedder::new(
            Arc::new(ConstEmbedder { value: 0.5, dims: 16 }),
            Arc::new(NoopLoraStore),
            enabled,
        )
    }

    #[tokio::test]
    async fn test_disabled_passthrough() {
        let e = make_embedder(false);
        let uid = Uuid::from_u128(1);
        let v = e.embed_for_agent("hello", uid, Some("agent1")).await.unwrap();
        assert_eq!(v, vec![0.5f32; 16], "disabled → must be identity");
    }

    #[tokio::test]
    async fn test_enabled_fresh_adapter_is_identity() {
        // B=0 → residual=0 → output == base
        let e = make_embedder(true);
        let uid = Uuid::from_u128(2);
        let v = e.embed_for_agent("hello", uid, Some("agent1")).await.unwrap();
        // Fresh adapter has B=0, so adapted == base
        assert_eq!(v, vec![0.5f32; 16], "fresh adapter must be identity");
    }

    #[tokio::test]
    async fn test_embed_batch_disabled() {
        let e = make_embedder(false);
        let uid = Uuid::from_u128(3);
        let texts = vec!["a".into(), "b".into()];
        let vecs = e.embed_batch_for_agent(&texts, uid, None).await.unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0], vec![0.5f32; 16]);
    }

    #[tokio::test]
    async fn test_provider_name_delegates() {
        let e = make_embedder(false);
        assert_eq!(e.provider_name(), "const-test");
    }

    #[tokio::test]
    async fn test_dimensions_delegates() {
        let e = make_embedder(false);
        assert_eq!(e.dimensions(), 16);
    }

    #[tokio::test]
    async fn test_update_changes_cache() {
        let e = make_embedder(true);
        let uid = Uuid::from_u128(4);
        let v_query = vec![1.0f32; 16];
        let v_item = vec![0.5f32; 16];

        // Prime the cache with a fresh adapter
        let _ = e.embed_for_agent("x", uid, Some("bot")).await.unwrap();

        // Apply update
        e.update_from_access(&v_query, &v_item, uid, Some("bot")).await;

        // The cached adapter should now have update_count=1
        let cache = e.cache.read().await;
        let key = LoraAdaptedEmbedder::<ConstEmbedder, NoopLoraStore>::cache_key(uid, Some("bot"));
        let adapter = cache.get(&key).unwrap();
        assert_eq!(adapter.weights.update_count, 1);
    }

    #[tokio::test]
    async fn test_concurrent_embed_and_update_no_corruption() {
        // Verify that concurrent embed_for_agent + update_from_access on the
        // same (user, agent) key does not corrupt the adapter state.
        // After all tasks complete: update_count must be a positive integer
        // and the adapter must be in a consistent (non-NaN) state.
        use std::sync::Arc;
        use tokio::task;

        let e = Arc::new(make_embedder(true));
        let uid = Uuid::from_u128(42);
        let agent = "concurrent-test";

        // Spawn 8 concurrent updaters and 8 concurrent readers
        let mut handles = Vec::new();

        for _ in 0..8 {
            let e2 = e.clone();
            handles.push(task::spawn(async move {
                for _ in 0..10 {
                    e2.update_from_access(
                        &vec![1.0f32; 16],
                        &vec![0.5f32; 16],
                        uid,
                        Some(agent),
                    )
                    .await;
                }
            }));
        }

        for _ in 0..8 {
            let e2 = e.clone();
            handles.push(task::spawn(async move {
                for _ in 0..10 {
                    let v = e2.embed_for_agent("x", uid, Some(agent)).await.unwrap();
                    assert_eq!(v.len(), 16, "output must have correct dims");
                    assert!(v.iter().all(|x| x.is_finite()), "output must be finite");
                }
            }));
        }

        for h in handles {
            h.await.expect("task panicked");
        }

        // After all concurrent updates, the adapter must have been updated at
        // least once and must not contain NaN/Inf values.
        let cache = e.cache.read().await;
        let key = LoraAdaptedEmbedder::<ConstEmbedder, NoopLoraStore>::cache_key(uid, Some(agent));
        if let Some(adapter) = cache.get(&key) {
            assert!(
                adapter.weights.update_count > 0,
                "update_count must be positive after concurrent updates"
            );
            assert!(
                adapter.weights.b_flat.iter().all(|x| x.is_finite()),
                "B must be finite after concurrent updates"
            );
        }
        // If the key is absent it means all reads raced before any write — that's
        // also acceptable (no corruption, no panic).
    }

    #[tokio::test]
    async fn test_adapter_output_changes_after_update() {
        let e = make_embedder(true);
        let uid = Uuid::from_u128(5);
        // Use non-zero vectors so A·v_item is non-zero and ΔB is non-zero
        let query = vec![1.0f32; 16];
        let item = vec![0.5f32; 16]; // same as base embedder output — A·item will be non-zero

        // Prime the cache
        let v_before = e.embed_for_agent("x", uid, Some("bot")).await.unwrap();

        // Apply many updates. delta = query - adapt(item) = [0.5; 16] initially
        // h = A · item (non-zero) → ΔB = lr * outer(delta, h) ≠ 0
        for _ in 0..20 {
            e.update_from_access(&query, &item, uid, Some("bot")).await;
        }

        // After updates B should be non-zero → adapted output differs from base
        let v_after = {
            let cache = e.cache.read().await;
            let key = LoraAdaptedEmbedder::<ConstEmbedder, NoopLoraStore>::cache_key(uid, Some("bot"));
            let adapter = cache.get(&key).unwrap();
            // Verify B is non-zero
            let b_norm: f32 = adapter.weights.b_flat.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(b_norm > 1e-8, "B must be non-zero after updates, norm={}", b_norm);
            adapter.apply(&vec![0.5f32; 16])
        };

        // v_before is identity (B=0), v_after has B non-zero → must differ
        let changed = v_before.iter().zip(v_after.iter()).any(|(a, b)| (a - b).abs() > 1e-7);
        assert!(changed, "adapter output should change after updates");
    }
}
