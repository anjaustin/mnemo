# Spec 06 — TinyLoRA: Agent-Specific Memories of Shared Data

**Status:** In Progress  
**Crate:** `crates/mnemo-lora/`  
**Feature flag:** `MNEMO_LORA_ENABLED=true`

---

## Problem Statement

Mnemo currently scopes memory retrieval per-user and per-agent using hard filters
applied post-search. Two agents sharing a user's memory space see the same cosine
distances from the same base embeddings — retrieval relevance is identical regardless
of which agent is asking.

For multi-agent deployments (supervisor + worker + analyst sharing a user's facts),
each agent has systematically different relevance priors. The supervisor cares most
about deadlines and decisions; the analyst cares most about data relationships and
metrics; the worker cares about task instructions. The base embedding model has no
knowledge of these differences.

TinyLoRA adds per-`(user_id, agent_id)` learned linear transformations on top of the
shared embedding space. At query and ingest time, a small rank-decomposed matrix
`W = I + scale * B @ A` rotates the base embedding toward that agent's observed
relevance history. The shared Qdrant index is never modified — only the vectors
presented to it change.

---

## Design

### Adapter Math

```
d = 384  (AllMiniLML6V2 output dimension)
r = 8    (LoRA rank — 4% of d)

A ∈ ℝ^{r×d}   — down-projection (random Kaiming init)
B ∈ ℝ^{d×r}   — up-projection (zero init)
scale = α/r = 1.0/8.0 = 0.125   (standard LoRA default)

v_adapted = v_base + scale * B · (A · v_base)
```

**Initialization:**
- B = 0 → zero residual at start (adapter is identity at step 0, same as LoRA paper)
- A = random with Kaiming scale `sqrt(1/d)` → bounded initial down-projection

**Update rule (implicit feedback, no backprop):**
```
When edge/episode ID `e` is accessed (record_edge_access called):
  δ = v_query - v_e_stored   (direction from stored to query)
  ΔA += lr * outer(A·v_e, δ)  [simplified: δ direction in low-rank space]
  ΔB += lr * outer(δ, A·v_e)
  update_count += 1
```
This is a simplified online Hebbian update — not full gradient descent. It nudges
the adapter to reduce the angular distance between the query vector and accessed
facts in the agent's adapted space.

**Normalization:** After each update, L2-normalize each column of B to prevent
unbounded growth. Scale is fixed, not learned.

### Storage Schema

Redis key: `{prefix}lora:{user_id}:{agent_id_or_global}`

where `agent_id_or_global` is the agent identifier string, or `__global__` for a
user-level (no agent) adapter.

Value: JSON blob

```json
{
  "user_id": "uuid",
  "agent_id": "string | null",
  "a_matrix": [[f32; d]; r],   // r×d, row-major
  "b_matrix": [[f32; r]; d],   // d×r, row-major
  "scale": 0.125,
  "update_count": 42,
  "last_updated": 1700000000
}
```

No TTL — adapters persist until explicitly deleted via API or user deletion.

### Crate Structure

```
crates/mnemo-lora/
  Cargo.toml
  src/
    lib.rs        — LoraAdapter, LoraAdaptedEmbedder<E>, public API
    math.rs       — matrix ops (pure Rust, no BLAS)
    store.rs      — LoraStore trait + in-memory stub
```

The Redis implementation of `LoraStore` lives in `crates/mnemo-storage/` alongside
the other Redis-backed store implementations, following the existing pattern.

### Trait Extension

`EmbeddingProvider` in `mnemo-core` gains two optional methods with default
implementations that delegate to the un-adapted variants:

```rust
async fn embed_for_agent(
    &self,
    text: &str,
    user_id: Uuid,
    agent_id: Option<&str>,
) -> LlmResult<Vec<f32>> {
    self.embed(text).await   // default: no adaptation
}

async fn embed_batch_for_agent(
    &self,
    texts: &[String],
    user_id: Uuid,
    agent_id: Option<&str>,
) -> LlmResult<Vec<Vec<f32>>> {
    self.embed_batch(texts).await   // default: no adaptation
}
```

`LoraAdaptedEmbedder<E: EmbeddingProvider>` overrides these to apply the adapter.

### Integration Points

| Pipeline stage | Location | Change |
|---|---|---|
| Ingest: entity embed | `mnemo-ingest/src/lib.rs:762` | `embed` → `embed_for_agent` |
| Ingest: edge embed | `mnemo-ingest/src/lib.rs:884` | `embed` → `embed_for_agent` |
| Ingest: episode embed | `mnemo-ingest/src/lib.rs:898` | `embed` → `embed_for_agent` |
| Retrieval: query embed | `mnemo-retrieval/src/lib.rs:172` | `embed` → `embed_for_agent` |
| Sleep-time update | `mnemo-retrieval` proactive path | adapter update on `record_edge_access` |

### Feature Flag

`MNEMO_LORA_ENABLED=true/false` (default: `false`).

When disabled, `embed_for_agent` delegates to `embed` (zero overhead, zero behavior
change). When enabled, `LoraAdaptedEmbedder` wraps the base embedder and loads/caches
adapters from Redis per `(user_id, agent_id)`.

---

## Deliverables

| ID | Description | File(s) |
|---|---|---|
| D1 | `LoraStore` trait in mnemo-core + Redis impl in mnemo-storage | `mnemo-core/src/traits/storage.rs`, `mnemo-storage/src/lora_store.rs` |
| D2 | `LoraAdapter`, `LoraAdaptedEmbedder<E>`, math | `crates/mnemo-lora/src/` |
| D3 | `EmbeddingProvider::embed_for_agent` default methods | `mnemo-core/src/traits/llm.rs` |
| D4 | `LoraAdaptedEmbedder<E>` wrapper (lives in mnemo-lora) | `crates/mnemo-lora/src/lib.rs` |
| D5 | Wire `embed_for_agent` into `IngestWorker::process_episode` | `mnemo-ingest/src/lib.rs` |
| D6 | Wire `embed_for_agent` into `RetrievalEngine::get_context` | `mnemo-retrieval/src/lib.rs` |
| D7 | Adapter update on `record_edge_access` (implicit feedback) | `mnemo-retrieval/src/lib.rs` |
| D8 | `GET /api/v1/agents/{id}/lora/stats`, `DELETE /api/v1/agents/{id}/lora` | `mnemo-server/src/routes.rs` |
| D9 | Eval case pack for LoRA personalization | `eval/cases/lora_personalization.json` |
| D10 | `MNEMO_LORA_ENABLED` config + AppState wiring | `mnemo-server/src/config.rs`, `state.rs` |

---

## Non-Goals

- Backprop through the base embedding model (the base model is frozen)
- Per-session adapters (user+agent granularity is sufficient)
- Adapter merging / ensemble (future work)
- BLAS/LAPACK dependency (pure Rust, <1ms overhead for d=384, r=8)
