# Metadata Index Layer (Design Spec)

Date: 2026-03-02
Status: implemented-v1

## Goal

Add a dedicated metadata indexing layer for sessions and episodes to improve retrieval precision, latency, and control-plane visibility.

This layer complements semantic vectors and temporal scoring; it does not replace them.

## Why this matters

Vector search is strong for semantic similarity but weak for strict constraints and operational queries.

A metadata index enables:

- fast prefiltering (user/session/role/source/time)
- deterministic scoping before semantic retrieval
- lower retrieval noise and reduced search cost
- better explainability and debuggability

## Scope

### Indexed objects

1. Session metadata documents
2. Episode metadata documents

### Out of scope (v1)

- replacing current vector index
- advanced learned planners
- cross-tenant analytics

## Data model

## Session index document

- `session_id` (primary)
- `user_id`
- `name`
- `created_at`
- `updated_at`
- `head_episode_id` (if Thread HEAD proposal lands)
- `head_updated_at` (if Thread HEAD proposal lands)
- `episode_count`
- `last_role`
- `tags` (optional)
- `metadata` (flattened selected fields)

## Episode index document

- `episode_id` (primary)
- `user_id`
- `session_id`
- `type` (`message|json|text`)
- `role` (nullable)
- `created_at`
- `processing_status`
- `has_entities` (bool)
- `has_edges` (bool)
- `source` (optional: sdk/webhook/import)
- `tags` (optional)
- `content_preview` (short, optional)
- `metadata` (flattened selected fields)

## Query capabilities

v1 filter operators:

- equality (`user_id`, `session_id`, `role`, `status`)
- set inclusion (`tags any/all`)
- range (`created_at` start/end)
- prefix (session name, optional)
- ordering (`created_at desc`, `head_updated_at desc`)

## Retrieval pipeline integration

Planned retrieval stages:

1. Metadata prefilter stage
   - resolve strict constraints to candidate episode/session IDs
2. Semantic+temporal retrieval stage
   - run vector/fulltext retrieval constrained to candidate set
3. Fusion stage
   - combine semantic, temporal, graph, and metadata priors
4. Context assembly stage
   - construct output context with source diagnostics

### v1 delivered

- Memory context API now accepts metadata filters (`roles`, `tags_any`, `tags_all`, `created_after`, `created_before`, `processing_status`).
- A metadata prefilter planner scans candidate episodes, applies filters, and emits diagnostics:
  - `candidate_count_before_filters`
  - `candidate_count_after_filters`
  - `candidate_reduction_ratio`
  - `planner_latency_ms`
  - `applied_filters`
- Planner controls are configurable (`metadata_prefilter_enabled`, `metadata_scan_limit`, `metadata_relax_if_empty`).
- Optional relaxed fallback can recover from over-pruning when enabled.
- Filtered candidate sessions influence retrieval session scoping when explicit session is not provided.

### Example planner behavior

Request:

```json
{
  "query": "what did i decide in this sprint retro?",
  "session": "retro-2026-03",
  "mode": "head"
}
```

Planner:

1. metadata filter: `user_id=X AND session_name=retro-2026-03`
2. candidate episode IDs from that session
3. constrained semantic+temporal scoring over candidate set

## Storage options

Primary implementation options:

1. RediSearch-backed metadata index (recommended v1)
   - aligns with existing Redis footprint
   - low operational overhead

2. Dedicated index backend (future)
   - only if scale/features outgrow RediSearch behavior

## API extensions (non-breaking)

Add optional request filters to memory/context endpoints:

- `filters.roles`
- `filters.tags_any`
- `filters.tags_all`
- `filters.created_after`
- `filters.created_before`
- `filters.processing_status`

Add response diagnostics:

- `candidate_count_before_filters`
- `candidate_count_after_filters`
- `applied_filters`

## Write path and consistency

Index update triggers:

- on session create/update
- on episode create
- on ingestion completion (status/features updates)

Consistency mode:

- eventually consistent for retrieval (acceptable)
- strongly consistent for identity lookups by primary ID

## Performance targets

- metadata prefilter p95 < 10ms (local)
- end-to-end context retrieval p95 unaffected or improved
- candidate set reduction > 50% on filtered queries

## Observability

Track:

- filter hit rate
- average candidate reduction ratio
- latency contribution per stage
- stale index lag (event time vs index update time)

## Risks and mitigations

1. Over-filtering drops relevant memories
   - Mitigation: fallback to relaxed filter mode when candidate set is too small.

2. Index schema drift
   - Mitigation: versioned index schema and migration checks at startup.

3. Write amplification
   - Mitigation: batched index updates and selective field indexing.

4. Query complexity growth
   - Mitigation: cap v1 operators and add planner safeguards.

## Rollout plan

1. Define indexed field schema + startup index validation.
2. Implement episode/session metadata upserts.
3. Add retrieval prefilter planner behind feature flag.
4. Emit diagnostics and measure candidate reduction + latency.
5. Enable by default for constrained-context requests.
