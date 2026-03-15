//! LoRA adapter weight storage model.
//!
//! Each `LoraWeights` record stores the two low-rank projection matrices
//! (A and B) for a specific `(user_id, agent_id)` pair.  A `None` agent_id
//! means a user-level adapter shared by all agents for that user.
//!
//! **Matrix shapes** (d=embedding dimensions, r=rank, typically r=8):
//! - `a_matrix`: r × d  (down-projection)
//! - `b_matrix`: d × r  (up-projection)
//!
//! **Adaptation formula:**
//! `v_adapted = v_base + scale * B · (A · v_base)`
//!
//! B is zero-initialized → adapter is identity at start.
//! A is Kaiming-initialized → bounded initial projection.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Persisted LoRA adapter weights for one `(user_id, agent_id)` pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraWeights {
    /// User who owns this adapter.
    pub user_id: Uuid,
    /// Agent this adapter belongs to.  `None` = user-level adapter (no agent).
    pub agent_id: Option<String>,
    /// Down-projection matrix A, shape r×d, stored as a flat Vec (row-major).
    /// Element `[i][j]` = `a_matrix[i * dims + j]`.
    pub a_flat: Vec<f32>,
    /// Up-projection matrix B, shape d×r, stored as a flat Vec (row-major).
    /// Element `[i][j]` = `b_flat[i * rank + j]`.
    pub b_flat: Vec<f32>,
    /// Fixed scale factor: `alpha / rank` (default 0.125 for rank=8, alpha=1.0).
    pub scale: f32,
    /// Embedding dimension `d` (must match the base embedder's output).
    pub dims: usize,
    /// LoRA rank `r`.
    pub rank: usize,
    /// Number of implicit-feedback update steps applied.
    pub update_count: u64,
    /// Unix timestamp (seconds) of the most recent update.
    pub last_updated: i64,
}

impl LoraWeights {
    /// Default rank used when creating a fresh adapter.
    pub const DEFAULT_RANK: usize = 8;

    /// Default scale: `alpha / rank = 1.0 / 8.0`.
    pub const DEFAULT_SCALE: f32 = 1.0 / Self::DEFAULT_RANK as f32;

    /// Create a new adapter with the identity residual (B=0, A=Kaiming).
    ///
    /// The adapter starts as a pure pass-through: `v_adapted = v_base + 0`.
    /// Updates gradually shift A and B to encode the agent's relevance priors.
    pub fn initialize(user_id: Uuid, agent_id: Option<String>, dims: usize) -> Self {
        let rank = Self::DEFAULT_RANK;
        let scale = Self::DEFAULT_SCALE;

        // B = 0 (standard LoRA init — zero residual at start)
        let b_flat = vec![0.0f32; dims * rank];

        // A = Kaiming uniform: range [-k, k] where k = sqrt(1 / dims)
        // Deterministic seed incorporating (user_id, agent_id) so that each
        // (user, agent) pair gets a distinct projection matrix, improving the
        // diversity of adapted embedding spaces across pairs.
        let k = (1.0_f32 / dims as f32).sqrt();

        // Build a u64 seed from user_id bytes and agent_id string bytes.
        // We XOR a folded version of the user UUID with a hash of agent_id,
        // then use a linear-congruential step per matrix element for speed.
        let uid_bytes = user_id.as_bytes();
        let uid_seed: u64 = uid_bytes[..8]
            .iter()
            .enumerate()
            .fold(0u64, |acc, (i, &b)| acc ^ ((b as u64) << (i * 8)))
            ^ uid_bytes[8..]
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, &b)| acc ^ ((b as u64) << (i * 8)));

        let agent_seed: u64 = agent_id
            .as_deref()
            .unwrap_or("__global__")
            .bytes()
            .enumerate()
            .fold(0u64, |acc, (i, b)| {
                acc.wrapping_add(
                    (b as u64).wrapping_mul(6364136223846793005u64.wrapping_pow(i as u32 + 1)),
                )
            });

        let base_seed: u64 = uid_seed ^ agent_seed ^ 0xdeadbeef_cafebabe;

        let a_flat: Vec<f32> = (0..rank * dims)
            .map(|i| {
                // LCG step seeded per-element from the (user, agent, index) triple.
                // Multiplier and increment from Knuth TAOCP Vol 2 (64-bit LCG).
                let s = base_seed
                    .wrapping_add(i as u64)
                    .wrapping_mul(6364136223846793005u64)
                    .wrapping_add(1442695040888963407u64);
                // Take upper 24 bits for float conversion (lower bits have shorter period)
                let x = ((s >> 40) as f32) / (1u64 << 24) as f32; // [0, 1)
                (x * 2.0 - 1.0) * k // [-k, k]
            })
            .collect();

        let now = chrono::Utc::now().timestamp();

        Self {
            user_id,
            agent_id,
            a_flat,
            b_flat,
            scale,
            dims,
            rank,
            update_count: 0,
            last_updated: now,
        }
    }

    /// Return a human-readable key for logging.
    pub fn key_string(&self) -> String {
        match &self.agent_id {
            Some(id) => format!("{}:{}", self.user_id, id),
            None => format!("{}:__global__", self.user_id),
        }
    }
}

/// Request to reset (delete) an agent's LoRA adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetLoraRequest {
    /// If true, reset the user-level (agentless) adapter too.
    pub reset_global: bool,
}

/// Explicit relevance feedback for homeoadaptive adapter updates.
///
/// Submitted by an application after a retrieval+response cycle.  Each rating
/// nudges the `(user_id, agent_id)` adapter toward or away from the query
/// embedding, providing a stronger training signal than implicit access alone.
///
/// **Rating semantics:**
/// - `1.0`  = highly relevant — adapter moves toward this item
/// - `-1.0` = irrelevant     — adapter moves away from this item
/// - `0.0`  = neutral        — no update applied for this item
///
/// Only items with `|rating| > 0.0` produce a weight update; neutral ratings
/// are silently skipped to avoid noisy gradient accumulation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoraFeedbackRequest {
    /// User identifier (email or UUID string).
    pub user: String,
    /// The query text that was used for retrieval.
    pub query_text: String,
    /// Per-item relevance ratings.  Keys are edge/entity/episode IDs (UUID strings).
    /// Must contain at least one non-zero rating.
    pub ratings: std::collections::HashMap<String, f32>,
}

/// Response from the explicit feedback endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoraFeedbackResponse {
    /// Number of items for which a weight update was applied.
    pub items_updated: usize,
    /// Total implicit-feedback updates applied to this adapter so far
    /// (including this batch).
    pub total_update_count: u64,
    /// Frobenius norm of B after this update batch.
    pub b_frobenius_norm: f32,
}

/// Response from the LoRA stats endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoraStatsResponse {
    pub user_id: Uuid,
    pub agent_id: Option<String>,
    pub update_count: u64,
    pub last_updated: i64,
    pub dims: usize,
    pub rank: usize,
    pub scale: f32,
    /// Frobenius norm of B (proxy for how much the adapter has diverged from identity).
    pub b_frobenius_norm: f32,
}

impl From<&LoraWeights> for LoraStatsResponse {
    fn from(w: &LoraWeights) -> Self {
        let b_norm = w.b_flat.iter().map(|x| x * x).sum::<f32>().sqrt();
        Self {
            user_id: w.user_id,
            agent_id: w.agent_id.clone(),
            update_count: w.update_count,
            last_updated: w.last_updated,
            dims: w.dims,
            rank: w.rank,
            scale: w.scale,
            b_frobenius_norm: b_norm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_zero_b() {
        let w = LoraWeights::initialize(Uuid::from_u128(1), Some("agent1".into()), 384);
        assert_eq!(w.b_flat.len(), 384 * 8);
        assert!(
            w.b_flat.iter().all(|&x| x == 0.0),
            "B must be zero-initialized"
        );
    }

    #[test]
    fn test_initialize_nonzero_a() {
        let w = LoraWeights::initialize(Uuid::from_u128(1), Some("agent1".into()), 384);
        assert_eq!(w.a_flat.len(), 8 * 384);
        let nonzero = w.a_flat.iter().filter(|&&x| x != 0.0).count();
        assert!(nonzero > 0, "A should have non-zero entries");
    }

    #[test]
    fn test_initialize_dims_and_rank() {
        let w = LoraWeights::initialize(Uuid::from_u128(2), None, 384);
        assert_eq!(w.dims, 384);
        assert_eq!(w.rank, 8);
        assert!((w.scale - 0.125).abs() < 1e-6);
    }

    #[test]
    fn test_key_string_with_agent() {
        let uid = Uuid::from_u128(1);
        let w = LoraWeights::initialize(uid, Some("my-agent".into()), 384);
        assert!(w.key_string().ends_with(":my-agent"));
    }

    #[test]
    fn test_key_string_global() {
        let uid = Uuid::from_u128(1);
        let w = LoraWeights::initialize(uid, None, 384);
        assert!(w.key_string().ends_with(":__global__"));
    }

    #[test]
    fn test_stats_response_zero_norm_for_fresh_adapter() {
        let w = LoraWeights::initialize(Uuid::from_u128(1), Some("agent".into()), 384);
        let stats = LoraStatsResponse::from(&w);
        assert!(
            stats.b_frobenius_norm < 1e-6,
            "Fresh adapter B norm must be 0"
        );
    }

    #[test]
    fn test_serialization_roundtrip() {
        let w = LoraWeights::initialize(Uuid::from_u128(42), Some("bot".into()), 384);
        let json = serde_json::to_string(&w).unwrap();
        let w2: LoraWeights = serde_json::from_str(&json).unwrap();
        assert_eq!(w.dims, w2.dims);
        assert_eq!(w.rank, w2.rank);
        assert_eq!(w.b_flat.len(), w2.b_flat.len());
        assert_eq!(w.a_flat.len(), w2.a_flat.len());
        for i in 0..w.a_flat.len() {
            assert!((w.a_flat[i] - w2.a_flat[i]).abs() < 1e-7);
        }
    }
}
