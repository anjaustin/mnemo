# Spec 07 — AUMRL: Agentic-User-Moulded Representation Learner

**Status:** Design  
**Depends on:** Spec 06 (TinyLoRA infrastructure)  
**Feature flag:** `MNEMO_AUMRL_ENABLED=true` (independent of `MNEMO_LORA_ENABLED`)

---

## Problem Statement

Spec 06 TinyLoRA adapts embeddings from the **user's perspective** — the adapter for
`(user_id, agent_id)` captures what _this user_ finds relevant when interacting with
_this agent_.

AUMRL inverts the ownership: the adapter for `(agent_id, user_id)` captures what _this
agent_ has learned about _this user's_ communication style, domain depth, vocabulary,
and retrieval preferences. The agent accumulates a library of user profiles and applies
them at query time — adapting its embedding space to each user it serves.

**Key distinction:**

| | TinyLoRA (Spec 06) | AUMRL (Spec 07) |
|---|---|---|
| Ownership | User owns adapter, scoped to agent | Agent owns adapter, scoped to user |
| Redis key | `lora:{user_id}:{agent_id}` | `aumrl:{agent_id}:{user_id}` |
| Training signal | Implicit (access patterns) | Explicit (user feedback ratings) |
| Cross-user learning | Not possible (isolated per user) | Enabled (agent sees all its user profiles) |
| Personalization target | What this user finds relevant | How this agent should represent queries for this user |

---

## Design

### Adapter Math

Identical to Spec 06:

```
v_adapted = v_base + scale * B · (A · v_base)
A ∈ ℝ^{r×d}   (fixed random projection, seeded from agent_id + user_id hash)
B ∈ ℝ^{d×r}   (zero init; updated from explicit feedback)
scale = 0.125, rank = 8
```

### Training Signal: Explicit Feedback

AUMRL uses explicit user feedback rather than implicit access patterns:

```
POST /api/v1/agents/:agent_id/feedback
{
  "user": "alice",
  "query_text": "...",
  "retrieved_fact_ids": ["edge-uuid-1", "edge-uuid-2"],
  "ratings": {"edge-uuid-1": 1.0, "edge-uuid-2": -0.5}
}
```

Rating semantics:
- `1.0` = highly relevant (positive nudge toward query)
- `-1.0` = irrelevant (nudge away)
- `0.0` = neutral

Update rule per rated item:
```
lr_scaled = lr * |rating|
sign = sign(rating)
ΔB += sign * lr_scaled * outer(v_query - v_item_adapted, A · v_item)
```

Implicit access patterns (from retrieval) remain as a secondary signal at half weight,
so AUMRL adapters are not dependent on users actively providing ratings.

### Storage Schema

```
Redis key: {prefix}aumrl:{agent_id}:{user_id}   → JSON AumrlWeights
Redis key: {prefix}aumrl_idx:{agent_id}          → Set of known user slots
```

`AumrlWeights` is structurally identical to `LoraWeights` with an additional `ratings_count` field.

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/agents/:agent_id/feedback` | Submit explicit relevance ratings |
| `GET` | `/api/v1/agents/:agent_id/aumrl/stats` | List per-user adapter stats for this agent |
| `GET` | `/api/v1/agents/:agent_id/aumrl/stats/:user` | Stats for one user's adapter |
| `DELETE` | `/api/v1/agents/:agent_id/aumrl/:user` | Reset one user's adapter |
| `DELETE` | `/api/v1/agents/:agent_id/aumrl` | Reset all user adapters for this agent |

### Homeoadaptive Behavior

The term "homeoadaptive" reflects that the embedding space self-regulates toward each
user's equilibrium without explicit configuration. After sufficient interactions with
a user, the agent's representation of that user's queries stabilizes (B approaches a
fixed point given the user's consistent retrieval preferences), and further updates
produce diminishing drift (Frobenius clamp enforces boundedness).

### Cross-User Learning (Future)

Because the agent owns adapters for all users, it can examine the distribution of B
matrices across its user base. This enables:
- **User clustering**: group users by embedding preference similarity
- **Cold-start warm-up**: initialize a new user's adapter from the centroid of similar users
- **Drift detection**: flag user profiles that diverge significantly from the agent's median

This is not implemented in Spec 07 but is architecturally enabled by the inverted ownership.

---

## Deliverables

| ID | Description | File(s) |
|---|---|---|
| A1 | `AumrlStore` trait extending `LoraStore` with agent-scoped queries | `mnemo-core/src/traits/storage.rs` |
| A2 | Redis impl of `AumrlStore` | `mnemo-storage/src/redis_store.rs` |
| A3 | `AumrlAdaptedEmbedder<E, S>` in mnemo-lora | `crates/mnemo-lora/src/aumrl.rs` |
| A4 | `POST /api/v1/agents/:agent_id/feedback` endpoint | `mnemo-server/src/routes.rs` |
| A5 | Stats + reset endpoints | `mnemo-server/src/routes.rs` |
| A6 | Wire AUMRL embedder into `LoraEmbedderHandle::Aumrl` variant | `mnemo-server/src/lora_handle.rs` |
| A7 | Eval case pack | `eval/cases/aumrl_user_profiles.json` |

---

## Non-Goals

- Federated cross-agent learning (agents sharing user profiles with each other)
- Model distillation from user profiles (learning a new base model)
- Real-time streaming feedback (batch updates on request, not per-token)

---

## Implementation Notes

- `AumrlAdaptedEmbedder` can be a thin wrapper over the same `LoraAdapter` struct — the
  math is identical, only the key namespace and ownership semantics differ.
- `LoraEmbedderHandle` in mnemo-server will gain an `Aumrl` variant when A6 is implemented.
- AUMRL and TinyLoRA can coexist: both can be enabled simultaneously, applying different
  adapter layers for different purposes (user-owned vs. agent-owned).
- Explicit feedback endpoint must validate that `query_text` or `retrieved_fact_ids` are
  non-empty; ratings without anchor text have no vector to update toward.
