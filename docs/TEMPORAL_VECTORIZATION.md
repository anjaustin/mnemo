# Temporal Vectorization Sketch

Date: 2026-03-02
Status: implemented-v2

## Goal

Improve memory retrieval by modeling both:

- semantic relevance (what the memory is about)
- temporal relevance (when the memory is true/useful)

This should make recall more accurate for "current" vs "historical" questions and reduce stale-memory errors.

## Delivered in v2

- Temporal controls are live on context APIs (`time_intent`, `as_of`, `temporal_weight`).
- Query-time intent auto-resolution includes recency/historical/date hints.
- Temporal reranking is applied to facts, episodes, and entities before context assembly.
- Response payload now includes `temporal_diagnostics` with resolved intent, applied weight, and scored counts.
- Integration falsification covers ranking shifts between `current` and `historical` intents.

## Core idea

Use a dual-channel retrieval score:

- semantic score from vector search (existing)
- temporal score from time-aware features (new)

Final rank score:

```text
final_score = alpha * semantic_score + beta * temporal_score + gamma * graph_score
```

Where:

- `semantic_score` comes from Qdrant similarity
- `temporal_score` comes from recency + validity window matching + query time intent
- `graph_score` is optional boost from relationship/topology confidence

## v1 plan (low risk, incremental)

### 1) Keep current semantic vectors unchanged

No change to embedding generation or Qdrant schema required for semantic channel.

### 2) Add temporal features to payload and in-memory reranker

Store and/or compute these fields for entities/edges/episodes:

- `created_at`
- `valid_at`
- `invalid_at`
- `age_days` (derived)
- `is_current` (`invalid_at` is null)
- `confidence`

### 3) Add query-time temporal intent detection

Heuristics first (later model-based):

- `current`: "now", "currently", "these days", "latest"
- `historical`: "as of", "back in", explicit date ranges
- `recent`: "recently", "last week", "this month"

Map intent to weights:

- current questions: boost current facts + recency
- historical questions: prioritize validity overlap with requested time window
- timeless questions: lower temporal weight

### 4) Add temporal scoring function

Example v1 temporal score:

```text
temporal_score =
  w_current * current_validity
  + w_window * window_overlap
  + w_recency * exp(-age_days / tau)
```

Suggested defaults:

- `tau = 30` days for conversational preferences
- dynamic `w_*` by query intent

### 5) Fuse + rerank top candidates

Pipeline:

1. Retrieve top-N semantic hits from Qdrant/full-text.
2. Compute temporal score for each candidate.
3. Combine into `final_score`.
4. Build context from reranked set.

This can be added in `mnemo-retrieval` without API breakage.

## v2 plan (more novel)

Introduce explicit temporal embeddings and multi-vector search.

Options:

1. **Feature-vector concat**
   - build a small time feature vector (e.g., cyclical day/week/month + age bucket)
   - concatenate with semantic vector before indexing

2. **Dual-vector index**
   - keep separate semantic and temporal vectors
   - query each channel and fuse with weighted rank aggregation

3. **Time2Vec-style encoding**
   - learn or define periodic + linear components for time representation

v2 requires careful offline evals before rollout.

## API implications

Add optional fields to context request (non-breaking):

- `time_intent`: `current | recent | historical | auto`
- `as_of`: timestamp for point-in-time recall
- `time_window`: `{start, end}`
- `temporal_weight`: float override

Return diagnostic metadata:

- `temporal_mode`
- `scoring_weights`
- per-source contribution stats (for debugging quality)

## Evaluation plan

Build a temporal recall benchmark set with cases:

1. preference changes over time
2. contradictory facts with explicit supersession
3. point-in-time questions
4. current-state questions after old history accumulation

Track metrics:

- temporal precision@k
- stale-fact rate
- contradiction error rate
- answer latency impact

Compare:

- baseline semantic-only
- v1 temporal reranker
- v2 temporal embedding variants

## Risks and mitigations

1. Recency bias overpowers relevance
   - Mitigation: cap recency contribution and keep semantic floor.

2. Intent classifier mistakes
   - Mitigation: expose `time_intent` override in API.

3. Score instability across domains
   - Mitigation: profile-based defaults (support/chat/sales) and config tuning.

4. Added latency
   - Mitigation: rerank only top-N candidates and cache temporal feature derivations.

## Rollout strategy

1. Feature-flag temporal reranking off by default.
2. Shadow-score in logs to compare baseline vs temporal rank.
3. Enable for a subset of users/queries.
4. Promote to default when stale-fact rate and contradiction errors improve without latency regression.
