# Spec 07 — Homeoadaptive Extensions: Explicit Feedback + Agent-View Stats

**Status:** Complete  
**Depends on:** Spec 06 (TinyLoRA infrastructure)

---

## Summary

Spec 07 extends TinyLoRA's homeoadaptive embedding personalization with two capabilities
built entirely on the existing `(user_id, agent_id)` adapter infrastructure:

1. **Explicit feedback loop** — `POST /api/v1/agents/:agent_id/feedback` accepts signed
   relevance ratings for retrieved items and applies them as directed LoRA updates,
   complementing Spec 06's implicit access-pattern signal.

2. **Agent-view stats** — `list_lora_weights_for_agent` lets an agent enumerate all
   user adapters it has accumulated, enabling observability and future cross-user analysis.

---

## Background: The AUMRL Evaluation

An earlier proposal (AUMRL — Agentic-User-Moulded Representation Learner) proposed
inverting the adapter key from `(user_id, agent_id)` to `(agent_id, user_id)` and
calling this a distinct system. On evaluation, the distinction was cosmetic: the math,
the struct, the Redis schema, and the update rule are identical regardless of key order.
The only genuinely new ideas were:

- Explicit feedback as a training signal (implemented here as D1)
- An agent-scoped list query (implemented here as D2)

The key-inversion framing was retired. AUMRL is not a separate system; it is a
description of what TinyLoRA already does viewed from the agent's perspective.

---

## Coined Term: Homeoadaptive

**Homeoadaptive** *(adj.)* — of an embedding system: self-regulating toward a stable
operating point for each `(user, agent)` pair through continuous implicit and explicit
feedback, such that successive retrievals converge on each agent's equilibrium
representation of relevance without external configuration.

Analogous to homeostasis: the system corrects toward a *learned* set-point, not a
fixed one. The B matrix's three-phase convergence (early differentiation → stabilization
→ slow long-term drift) is what makes TinyLoRA homeoadaptive rather than merely adaptive.

Coined in Mnemo v0.7.0 (Spec 06 + 07).

---

## Deliverables

### D1 — Explicit Feedback Endpoint

**`POST /api/v1/agents/:agent_id/feedback`**

Accepts `LoraFeedbackRequest`:
```json
{
  "user": "alice@example.com",
  "query_text": "What are our Q3 revenue projections?",
  "ratings": {
    "edge-uuid-1": 1.0,
    "edge-uuid-2": -0.5
  }
}
```

For each non-zero rating:
1. Look up the edge by UUID from the state store (skip if not found or wrong user)
2. Re-embed the edge `fact` text using the base embedder
3. Call `update_lora_with_rating(v_query, v_item, rating, user_id, agent_id)`

The signed update rule in `LoraAdapter::update_with_rating`:
```
delta = sign(rating) * (v_query - v_item_adapted)
ΔB   += |rating| * lr * outer(delta, A · v_item)
B     = clamp_frobenius(B, 10.0)
```

Positive rating → adapter moves toward the item.
Negative rating → adapter moves away from the item.
Rating `0.0` → no-op.

Response: `LoraFeedbackResponse` with `items_updated`, `total_update_count`,
`b_frobenius_norm`.

**When `MNEMO_LORA_ENABLED=false`**: accepts the request, returns `items_updated: 0`.

### D2 — Agent-View List Query

**`list_lora_weights_for_agent(agent_id: &str) -> Vec<LoraWeights>`**

Added to the `LoraStore` trait and implemented in `RedisStateStore`.

Key schema addition:
```
{prefix}lora_agent_idx:{agent_id}  →  Set<user_id string>
```

Maintained in parallel with the existing `lora_idx:{user_id}` user index:
- `save_lora_weights` → also `SADD lora_agent_idx:{agent_id} {user_id}` (for concrete agents)
- `delete_lora_weights` → also `SREM lora_agent_idx:{agent_id} {user_id}`
- `list_lora_weights_for_agent` → `SMEMBERS lora_agent_idx:{agent_id}` → fetch each weight

Stale index entries (adapter deleted but index not cleaned up) are pruned lazily during
list operations.

### D3 — `update_lora_with_rating` trait method + implementations

Added to `EmbeddingProvider` (default no-op), `LoraAdaptedEmbedder<E,S>`, and
`LoraEmbedderHandle`. The method signature:

```rust
async fn update_lora_with_rating(
    &self,
    v_query: &[f32],
    v_item: &[f32],
    rating: f32,         // [-1.0, 1.0]
    user_id: Uuid,
    agent_id: Option<&str>,
)
```

`LoraAdapter::update_from_access` is refactored to delegate to
`update_with_rating(v_query, v_item, 1.0)` — a positive-1.0 explicit rating — so
implicit and explicit feedback share a single update kernel.

### D4 — `RetrievalEngine::embedder()` accessor

Added `pub fn embedder(&self) -> &Arc<E>` to `RetrievalEngine`, allowing the feedback
route handler to access the embedder for rating-directed updates without adding a new
engine method.

---

## Tests Added

| Test | Crate | What it verifies |
|------|-------|-----------------|
| `test_update_with_zero_rating_is_noop` | mnemo-lora | Zero rating does not modify B or update_count |
| `test_negative_rating_inverts_update` | mnemo-lora | Single-step ±1.0 ratings produce exactly opposite ΔB (before clamp) |

---

## Non-Goals

- A separate key namespace for `(agent_id, user_id)` — redundant given existing infrastructure
- Cross-user clustering from B matrices — future work, architecturally enabled by D2
- Real-time streaming feedback — batch-on-request is sufficient
- Feedback on entity or episode IDs (only edges are supported; they carry the `fact` text needed for re-embedding)
