# API Reference

Base URL: `http://localhost:8080`

Mnemo serves both **REST** (JSON over HTTP/1.1) and **gRPC** (protobuf over HTTP/2) on the same port. Content-type routing dispatches `application/grpc` requests to the gRPC stack.

All REST request and response bodies are JSON. IDs are UUIDv7 strings.

All REST responses include `x-mnemo-request-id`. Clients may also provide `x-mnemo-request-id` and Mnemo will propagate it.

---

## gRPC API

Proto definition: `proto/mnemo/v1/memory.proto`

gRPC services are served on the same port as REST. Use any gRPC client (e.g. `grpcurl`, tonic, grpc-go) pointed at the server address.

### Services

| Service | RPCs |
|---|---|
| `mnemo.v1.MemoryService` | `GetContext`, `CreateEpisode`, `ListEpisodes`, `DeleteEpisode` |
| `mnemo.v1.EntityService` | `ListEntities`, `GetEntity` |
| `mnemo.v1.EdgeService` | `QueryEdges`, `GetEdge` |
| `grpc.health.v1.Health` | `Check` (standard health check) |
| `grpc.reflection.v1.ServerReflection` | Server reflection for schema discovery |

### Authentication

When `MNEMO_AUTH_ENABLED=true`, gRPC RPCs require the same API key as REST endpoints. Pass the key via gRPC metadata:

- `authorization: Bearer <key>` (preferred)
- `x-api-key: <key>` (alternative)

Health check and reflection services do **not** require authentication.

Unauthenticated or invalid requests return `UNAUTHENTICATED` (gRPC code 16).

### Input validation

| Field | Constraint |
|---|---|
| `episode_type` | Must be `"message"` |
| `role` | Must be `"user"`, `"assistant"`, or `"system"` (or omitted) |
| `max_tokens` | Must be a positive integer (> 0) |
| `as_of` | Must be a valid RFC 3339 timestamp |
| `min_relevance` | Must be between 0.0 and 1.0 |
| `entity_type` (ListEntities) | Filters results by entity type (e.g. `"person"`, `"product"`) |

### Message size limits

Incoming gRPC messages are limited to **4 MiB**; outgoing responses to **16 MiB**.

### Quick start with grpcurl

```bash
# Health check
grpcurl -plaintext localhost:8080 grpc.health.v1.Health/Check

# List services (via reflection)
grpcurl -plaintext localhost:8080 list

# Create an episode (with auth)
grpcurl -plaintext -H 'authorization: Bearer YOUR_API_KEY' -d '{
  "user_id": "01234567-89ab-cdef-0123-456789abcdef",
  "session_id": "01234567-89ab-cdef-0123-456789abcde0",
  "content": "I just bought Nike running shoes",
  "episode_type": "message",
  "role": "user"
}' localhost:8080 mnemo.v1.MemoryService/CreateEpisode

# Get context
grpcurl -plaintext -H 'authorization: Bearer YOUR_API_KEY' -d '{
  "user_id": "01234567-89ab-cdef-0123-456789abcdef",
  "messages": [{"role": "user", "content": "What shoes does Kendra like?"}],
  "max_tokens": 500
}' localhost:8080 mnemo.v1.MemoryService/GetContext

# List entities (with type filter)
grpcurl -plaintext -H 'authorization: Bearer YOUR_API_KEY' -d '{
  "user_id": "01234567-89ab-cdef-0123-456789abcdef",
  "limit": 10,
  "entity_type": "person"
}' localhost:8080 mnemo.v1.EntityService/ListEntities

# Query edges
grpcurl -plaintext -H 'authorization: Bearer YOUR_API_KEY' -d '{
  "user_id": "01234567-89ab-cdef-0123-456789abcdef",
  "current_only": true,
  "limit": 20
}' localhost:8080 mnemo.v1.EdgeService/QueryEdges
```

### Error mapping

| gRPC Status | REST Equivalent | When |
|---|---|---|
| `NOT_FOUND` | 404 | Resource does not exist |
| `INVALID_ARGUMENT` | 400 | Malformed UUID, missing required field |
| `ALREADY_EXISTS` | 409 | Duplicate resource |
| `PERMISSION_DENIED` | 403 | Insufficient permissions |
| `UNAUTHENTICATED` | 401 | Missing/invalid auth |
| `RESOURCE_EXHAUSTED` | 429 | Rate limited |
| `INTERNAL` | 500 | Server error |

---

## Health

### `GET /health`

Returns server status and version. Also available at `/healthz` for Kubernetes-style liveness probes.

```json
// Response 200
{
  "status": "ok",
  "version": "0.5.0"
}
```

### `GET /metrics`

Returns Prometheus-compatible plaintext metrics (`text/plain; version=0.0.4`).

Included counters/gauges cover:
- HTTP request and response class totals
- Webhook delivery success/failure/dead-letter totals
- Webhook replay and manual retry totals
- Current retained webhook event backlog gauges

### `GET /api/v1/ops/summary`

Operator summary endpoint for dashboard cards.

Query params:
- `window_seconds` (optional, default `300`, max `86400`)

Returns HTTP/webhook/policy counters plus active backlog gauges and recent audit activity counts.

### `GET /api/v1/ops/compression`

Temporal tensor compression statistics. Reports per-tier point counts,
estimated storage, savings percentage, and sweep history.

**Compression tiers**:

| Tier | Age | Precision | Savings |
|------|-----|-----------|---------|
| `full` | 0-7 days | f32 | 0% |
| `half` | 7-30 days | f16 | ~50% |
| `int8` | 30-90 days | int8 | ~75% |
| `binary` | 90+ days | binary | ~97% |

**Configuration** (environment variables):

| Variable | Default | Description |
|---|---|---|
| `MNEMO_EMBEDDING_COMPRESSION_ENABLED` | `false` | Enable background compression sweep |
| `MNEMO_COMPRESSION_TIER1_DAYS` | `7` | Days until f16 quantization |
| `MNEMO_COMPRESSION_TIER2_DAYS` | `30` | Days until int8 quantization |
| `MNEMO_COMPRESSION_TIER3_DAYS` | `90` | Days until binary quantization |
| `MNEMO_COMPRESSION_SWEEP_INTERVAL_SECS` | `3600` | Seconds between sweep runs |

**Response shape** (abridged):
```json
{
  "enabled": true,
  "dimensions": 384,
  "total_points": 15000,
  "tiers": {
    "full":   { "count": 5000, "precision": "f32",    "estimated_bytes": 7680000 },
    "half":   { "count": 4000, "precision": "f16",    "estimated_bytes": 3072000 },
    "int8":   { "count": 3000, "precision": "int8",   "estimated_bytes": 1152000 },
    "binary": { "count": 3000, "precision": "binary", "estimated_bytes": 144000 }
  },
  "storage": {
    "estimated_bytes": 12048000,
    "uncompressed_bytes": 23040000,
    "savings_percent": 47.71
  },
  "sweep": {
    "interval_secs": 3600,
    "last_sweep_at": "2025-03-10T12:00:00Z",
    "last_sweep_compressed": 150,
    "total_sweeps": 24
  }
}
```

### `GET /api/v1/ops/hyperbolic`

Hyperbolic HNSW status. Reports whether Poincare ball re-ranking is enabled
for entity search results, along with curvature and blend parameters.

**Configuration** (environment variables):

| Variable | Default | Description |
|---|---|---|
| `MNEMO_HYPERBOLIC_GRAPH_ENABLED` | `false` | Enable Poincare ball re-ranking for entity search |
| `MNEMO_HYPERBOLIC_CURVATURE` | `1.0` | Curvature of the Poincare ball (higher = more hierarchy compression) |
| `MNEMO_HYPERBOLIC_ALPHA` | `0.3` | Blend factor: 0.0 = pure Cosine, 1.0 = pure hyperbolic |

**Response shape**:
```json
{
  "enabled": false,
  "curvature": 1.0,
  "alpha": 0.3,
  "description": "Hyperbolic re-ranking disabled: entity search uses standard Cosine similarity only"
}
```

### `GET /api/v1/ops/pipeline`

DAG pipeline status. Returns per-step metrics, DAG structure, dead-letter queue
summary, and pipeline configuration.

**Configuration** (environment variables):

| Variable | Default | Description |
|---|---|---|
| `MNEMO_PIPELINE_RETRY_MAX` | `3` | Maximum retries per step before dead-lettering |
| `MNEMO_PIPELINE_DEAD_LETTER_ENABLED` | `true` | Enable the dead-letter queue for permanently failed items |
| `MNEMO_PIPELINE_DEAD_LETTER_MAX_SIZE` | `1000` | Maximum items in the dead-letter queue (evicts oldest) |

**Response shape**:
```json
{
  "steps": [
    {
      "step": "ingest",
      "executions": 150,
      "successes": 148,
      "failures": 2,
      "retries": 3,
      "error_rate": 0.0133,
      "avg_duration_us": 1200
    }
  ],
  "dag": [
    {
      "step": "ingest",
      "description": "Claim episode from pending queue",
      "dependencies": [],
      "critical": true
    },
    {
      "step": "extract",
      "description": "LLM entity + relationship extraction",
      "dependencies": ["ingest"],
      "critical": true
    }
  ],
  "dead_letter": {
    "count": 0,
    "max_size": 1000,
    "recent_items": []
  },
  "config": {
    "max_retries": 3,
    "dead_letter_enabled": true,
    "dead_letter_max_size": 1000,
    "retry_base_delay_ms": 500
  }
}
```

The `steps` array contains metrics for all 7 pipeline steps: `ingest`, `extract`,
`embed`, `graph_update`, `webhook_notify`, `digest_invalidate`, `session_summarize`.

The `dag` array describes the directed acyclic graph structure: each step lists its
dependencies and whether it is critical (failure triggers retry) or optional.

### `GET /api/v1/ops/sync`

Delta consensus sync status. Reports node identity, vector clock state, known
peers, and delta exchange counters.

**Configuration** (environment variables):

| Variable | Default | Description |
|---|---|---|
| `MNEMO_SYNC_ENABLED` | `false` | Enable delta consensus sync |
| `MNEMO_SYNC_NODE_ID` | `standalone` | Unique identifier for this node in the cluster |

**Response shape** (disabled):
```json
{
  "node_id": "standalone",
  "vector_clock": { "entries": {} },
  "known_peers": [],
  "deltas_produced": 0,
  "deltas_received": 0,
  "conflicts_resolved": 0,
  "last_sync": {},
  "enabled": false
}
```

**Response shape** (enabled):
```json
{
  "node_id": "us-east-1",
  "vector_clock": { "entries": { "us-east-1": 42, "eu-west-1": 38 } },
  "known_peers": ["eu-west-1", "ap-south-1"],
  "deltas_produced": 1523,
  "deltas_received": 1487,
  "conflicts_resolved": 12,
  "last_sync": {
    "eu-west-1": "2026-03-11T10:30:00Z",
    "ap-south-1": "2026-03-11T10:29:45Z"
  },
  "enabled": true
}
```

CRDT types available for field-level sync:
- **GCounter** — grow-only counters (mention_count, episode_count)
- **LWWRegister** — last-writer-wins registers (name, summary, status)
- **ORSet** — observed-remove sets with add-wins semantics (aliases, tags)
- **LWWMap** — last-writer-wins maps (metadata fields)

Causal ordering is maintained via **Hybrid Logical Clocks (HLC)** and
**Vector Clocks** to detect concurrent writes across nodes.

### `GET /api/v1/traces/:request_id`

Cross-pipeline trace lookup by request correlation ID.

Query params:
- `from` (optional, default now-30d, RFC3339 timestamp)
- `to` (optional, default now, RFC3339 timestamp)
- `limit` (optional, default `100`, max `500`, per-source cap)
- `include_episodes` (optional, default `true`)
- `include_webhook_events` (optional, default `true`)
- `include_webhook_audit` (optional, default `true`)
- `include_governance_audit` (optional, default `true`)
- `user` (optional, user UUID or external_id filter for episode scan)

If `to <= from`, the API returns `400` validation error.

Returns matched artifacts across:
- episode metadata writes
- webhook event rows
- webhook audit rows
- governance audit rows

### `GET /api/v1/audit/export`

SOC 2 / compliance audit log export. Returns a unified, time-bounded list of governance and webhook audit events suitable for shipping to a SIEM, exporting for auditors, or feeding into compliance tooling.

Query parameters:

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `from` | RFC3339 datetime | 30 days ago | Start of time window (exclusive) |
| `to` | RFC3339 datetime | now | End of time window (inclusive) |
| `limit` | integer | `1000` | Max records returned (max `10000`) |
| `include_governance` | bool | `true` | Include governance policy audit events |
| `include_webhook` | bool | `true` | Include webhook delivery audit events |
| `user` | string | (none) | Filter by user UUID or external_id |

`to` must be after `from`; otherwise returns `400`.

```json
// Response 200
{
  "ok": true,
  "from": "2026-02-04T00:00:00Z",
  "to": "2026-03-06T00:00:00Z",
  "total": 2,
  "records": [
    {
      "audit_type": "governance",
      "id": "019513a4-7e2b-7000-8000-000000000001",
      "user_id": "019513a4-7e2b-7000-8000-000000000002",
      "action": "policy_update",
      "at": "2026-02-15T10:30:00Z",
      "request_id": "req-abc-123",
      "details": {"retention_days_message": 90}
    },
    {
      "audit_type": "webhook",
      "id": "019513a4-7e2b-7000-8000-000000000003",
      "user_id": "019513a4-7e2b-7000-8000-000000000002",
      "action": "delivered",
      "at": "2026-02-16T08:00:00Z",
      "webhook_id": "019513a4-7e2b-7000-8000-000000000004",
      "details": {"event_type": "memory.written", "attempts": 1}
    }
  ]
}
```

Records are returned in ascending `at` order. Each record has an `audit_type` field (`"governance"` or `"webhook"`); webhook records additionally carry a `webhook_id` field.

---

## Memory API (High-Level)

These endpoints are the fastest way to integrate Mnemo without manually managing user/session/episode IDs.

### `POST /api/v1/memory`

Remember a piece of text for a user. Mnemo resolves or creates the user, resolves or creates the session, and stores the episode.

```json
// Request
{
  "user": "kendra",
  "text": "I love hiking in Colorado and my dog is named Bear",
  "session": "default",
  "role": "user"
}
```

`session` and `role` are optional. Defaults are `"default"` and `"user"`.

```json
// Response 201
{
  "ok": true,
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "session_id": "019513a4-8c1f-7000-8000-000000000002",
  "episode_id": "019513a4-9d3a-7000-8000-000000000003"
}
```

### `POST /api/v1/memory/extract`

Synchronously extract entities and relationships from text **without persisting anything**. Returns what the LLM would produce if the text were submitted via `POST /api/v1/memory`. Useful for previewing extraction quality, building test harnesses, and debugging LLM configuration.

If no LLM is configured the endpoint returns an empty extraction with a `note: "no_llm"` field rather than an error, so callers can detect the no-LLM state explicitly.

```json
// Request
{
  "text": "Kendra just switched from Adidas to Nike running shoes",
  "user": "kendra"   // optional — if supplied, existing entities are used as dedup hints
}
```

`user` is optional. When supplied, the user's existing entity graph is passed to the LLM as deduplication hints so entity names are consistent with stored data.

```json
// Response 200
{
  "ok": true,
  "entities": [
    {"name": "Kendra", "entity_type": "person", "summary": "A runner"},
    {"name": "Nike", "entity_type": "organization", "summary": "Athletic shoe brand"},
    {"name": "Adidas", "entity_type": "organization", "summary": "Athletic shoe brand"}
  ],
  "relationships": [
    {
      "source_name": "Kendra",
      "target_name": "Nike",
      "label": "switched_to",
      "fact": "Kendra switched from Adidas to Nike running shoes",
      "confidence": 0.95
    }
  ],
  "entity_count": 3,
  "relationship_count": 1,
  "provider": "ollama/hf.co/LiquidAI/LFM2-24B-A2B-GGUF"
}
```

When no LLM is configured, `entity_count` and `relationship_count` are 0 and `note` is `"no_llm: LLM is not configured; set MNEMO_LLM_API_KEY to enable extraction"`.

### `POST /api/v1/memory/:user/context`

Retrieve context for a user by identifier (`:user` can be UUID, external_id, or name).

```json
// Request
{
  "query": "What are my hobbies?",
  "session": "default",
  "max_tokens": 500,
  "min_relevance": 0.3,
  "mode": "hybrid",
  "contract": "default",
  "retrieval_policy": "balanced",
  "time_intent": "current",
  "as_of": "2025-01-01T00:00:00Z",
  "temporal_weight": 0.5,
  "filters": {
    "roles": ["user"],
    "tags_any": ["priority"],
    "created_after": "2026-03-01T00:00:00Z"
  }
}
```

`session`, `max_tokens`, `min_relevance`, `mode`, `contract`, `retrieval_policy`, `time_intent`, `as_of`, and `temporal_weight` are optional.

- `mode`: `head | hybrid | historical` — **when omitted, the semantic router auto-classifies the query** (e.g., "what did we just discuss?" routes to `head`, "remember when..." routes to `historical`)
- `contract`: `default | support_safe | current_strict | historical_strict`
- `retrieval_policy`: `balanced | precision | recall | stability`
- `time_intent`: `auto | current | recent | historical`
- `as_of`: point-in-time target for historical recall
- `temporal_weight`: override temporal influence (0.0–1.0)
- `filters`: optional metadata prefilter (`roles`, `tags_any`, `tags_all`, `created_after`, `created_before`, `processing_status`)

**Semantic Routing**: When `mode` is omitted, the server's semantic router auto-classifies the query using keyword pattern matching. The routing decision (selected strategy, confidence, source, alternatives) is included in the response as `routing_decision`. The router recognises five strategies: `head`, `hybrid`, `historical`, `graph_focused`, and `episode_recall`. Graph-focused and episode-recall map to the hybrid pipeline with appropriate weighting.

If semantic retrieval is unavailable or not yet warmed up, Mnemo falls back to recent episode recall so the returned context is still usable immediately after `remember`.

```json
// Response 200
{
  "context": "### Relevant Entities...",
  "token_count": 183,
  "entities": [],
  "facts": [],
  "episodes": [],
  "latency_ms": 47,
  "sources": ["semantic_search", "full_text_search"],
  "temporal_diagnostics": {
    "resolved_intent": "current",
    "temporal_weight": 0.5,
    "as_of": "2025-01-01T00:00:00Z",
    "entities_scored": 3,
    "facts_scored": 5,
    "episodes_scored": 2
  },
  "metadata_filter_diagnostics": {
    "prefilter_enabled": true,
    "candidate_count_before_filters": 18,
    "candidate_count_after_filters": 4,
    "candidate_reduction_ratio": 0.78,
    "planner_latency_ms": 3,
    "relaxed_fallback_applied": false,
    "applied_filters": {
      "roles": ["user"],
      "tags_any": ["priority"]
    }
  },
  "mode": "hybrid",
  "contract_applied": "default",
  "retrieval_policy_applied": "balanced",
  "retrieval_policy_diagnostics": {
    "effective_max_tokens": 500,
    "effective_min_relevance": 0.3,
    "effective_temporal_intent": "auto",
    "effective_temporal_weight": null
  },
  "head": {
    "session_id": "019513a4-8c1f-7000-8000-000000000002",
    "episode_id": "019513a4-9d3a-7000-8000-000000000003",
    "updated_at": "2026-03-02T12:34:56Z",
    "version": 7
  }
}
```

### `POST /api/v1/memory/:user/changes_since`

Return what changed in memory between two points in time.

```json
// Request
{
  "from": "2025-02-01T00:00:00Z",
  "to": "2025-04-01T00:00:00Z",
  "session": "default"
}
```

- `session` is optional and can be a session name or UUID.
- Includes added/superseded facts, confidence deltas, head movement, and added episodes.
- `added_episodes` may include `request_id` when source writes had correlation IDs.

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "from": "2025-02-01T00:00:00Z",
  "to": "2025-04-01T00:00:00Z",
  "session": "default",
  "added_facts": [],
  "superseded_facts": [],
  "confidence_deltas": [],
  "head_changes": [
    {
      "session_id": "019513a4-8c1f-7000-8000-000000000002",
      "session_name": "default",
      "head_episode_id": "019513a4-9d3a-7000-8000-000000000003",
      "head_version": 7,
      "at": "2025-03-01T12:00:00Z"
    }
  ],
  "added_episodes": [
    {
      "episode_id": "019513a4-9d3a-7000-8000-000000000003",
      "session_id": "019513a4-8c1f-7000-8000-000000000002",
      "session_name": "default",
      "role": "user",
      "created_at": "2025-03-01T12:00:00Z",
      "preview": "I switched from Adidas to Nike..."
    }
  ],
  "summary": "0 added facts, 0 superseded facts, 0 confidence deltas, 1 head changes, 1 added episodes"
}
```

### `POST /api/v1/memory/:user/conflict_radar`

Build a contradiction/instability view over a user memory graph.

```json
// Request
{
  "as_of": "2026-03-03T00:00:00Z",
  "include_resolved": false,
  "max_items": 50
}
```

All fields are optional.

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "as_of": "2026-03-03T00:00:00Z",
  "conflicts": [
    {
      "source_entity": "Kendra",
      "label": "prefers",
      "severity": 0.85,
      "active_edge_count": 2,
      "recent_supersessions": 0,
      "needs_resolution": true,
      "reason": "multiple simultaneously active facts",
      "edges": [
        {
          "edge_id": "019513a4-9d3a-7000-8000-000000000111",
          "target_entity": "Adidas",
          "fact": "Kendra prefers Adidas",
          "confidence": 0.8,
          "valid_at": "2026-03-01T00:00:00Z",
          "invalid_at": null,
          "is_active": true
        }
      ]
    }
  ],
  "summary": {
    "clusters": 1,
    "needs_resolution": 1,
    "high_severity": 1
  }
}
```

### `GET /api/v1/users/:user/coherence`

Compute a coherence report for a user's knowledge graph. Measures internal
consistency across four dimensions: entity coherence, fact coherence,
temporal coherence, and structural coherence. No request body required.

`:user` can be a UUID, `external_id`, or user name.

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "score": 0.82,
  "entity_coherence": 0.85,
  "fact_coherence": 0.70,
  "temporal_coherence": 0.90,
  "structural_coherence": 0.83,
  "recommendations": [
    "Resolve 2 conflicting fact groups to improve consistency"
  ],
  "diagnostics": {
    "total_entities": 42,
    "total_edges": 128,
    "active_edges": 104,
    "invalidated_edges": 24,
    "conflicting_groups": 2,
    "communities_detected": 5,
    "isolated_entities": 3,
    "recent_supersessions": 6,
    "recent_corroborations": 12
  }
}
```

**Sub-score weights** (sum to 1.0):

| Dimension | Weight | Measures |
|-----------|--------|----------|
| Entity coherence | 0.20 | Type compatibility + evidence strength of connected entities |
| Fact coherence | 0.35 | Ratio of clean vs. conflicting fact groups |
| Temporal coherence | 0.20 | Corroboration-to-supersession ratio (last 30 days) |
| Structural coherence | 0.25 | Graph connectivity; penalizes fragmentation and isolation |

### `GET /api/v1/memory/:user/stale`

Returns facts whose effective confidence has decayed below the revalidation
threshold, ranked by Fisher importance (most important stale facts first).
Stale facts are valid edges that haven't been reinforced recently enough to
maintain confidence above the threshold.

Query parameters:

| Parameter | Default | Description |
|---|---|---|
| `threshold` | `0.3` | Effective confidence below which a fact is stale |
| `limit` | `50` | Maximum number of stale facts to return |
| `half_life_days` | `90` | Decay half-life in days |

Response:

```json
{
  "user_id": "019513a4-...",
  "threshold": 0.3,
  "half_life_days": 90,
  "stale_count": 3,
  "stale_facts": [
    {
      "edge": {
        "id": "...",
        "label": "loves",
        "fact": "Kendra loves Adidas shoes",
        "confidence": 0.85,
        "corroboration_count": 2,
        "valid_at": "2025-06-15T..."
      },
      "effective_confidence": 0.22,
      "fisher_importance": 0.74,
      "age_days": 270,
      "suggested_question": "Is it still true that Kendra loves Adidas shoes?"
    }
  ]
}
```

Effective confidence formula:
`confidence * corroboration_boost * decay_factor * importance_protection`

- `corroboration_boost`: `min(1.0 + 0.1 * (corroboration_count - 1), 2.0)` — each corroboration adds 10%, capped at 2x.
- `decay_factor`: `2^(-age_days / half_life)` — exponential decay.
- `importance_protection`: `1 + clamp(fisher_importance, 0, 1) * 2.0` — structurally important edges resist decay (EWC++ principle).

### `POST /api/v1/memory/:user/revalidate`

Revalidate a stale fact by updating its confidence and resetting the decay clock.
Call this when a user confirms that a previously-stale fact is still true.

```json
{
  "edge_id": "019513a4-...",
  "new_confidence": 0.9,
  "evidence_episode_id": "019513a5-..."
}
```

| Field | Required | Description |
|---|---|---|
| `edge_id` | yes | The edge ID to revalidate |
| `new_confidence` | yes | New confidence value (0.0-1.0) |
| `evidence_episode_id` | no | Episode providing evidence; if present, increments corroboration count |

Response:

```json
{
  "edge": { "...updated edge..." },
  "previous_confidence": 0.85,
  "new_effective_confidence": 0.9
}
```

The edge's `valid_at` is reset to now, restarting the decay clock. If
`evidence_episode_id` is provided, `corroboration_count` is incremented.

Errors:
- `400` if `new_confidence` is outside `[0.0, 1.0]`.
- `400` if the edge is already invalidated (superseded facts cannot be revalidated).
- `400` if the edge does not belong to the specified user.
- `404` if the edge or user does not exist.

### `GET /api/v1/memory/:user/clarifications`

List self-healing clarification requests for a user. By default, returns only
pending (unresolved) clarifications sorted by severity. These are questions
the system has generated from detected memory conflicts.

Query parameters:

| Parameter | Default | Description |
|---|---|---|
| `all` | `false` | If `true`, include resolved/expired/dismissed clarifications |
| `limit` | `50` | Maximum number of clarifications to return (max 200) |

Response:

```json
{
  "user_id": "019513a4-...",
  "count": 2,
  "pending_only": true,
  "clarifications": [
    {
      "id": "019513a5-...",
      "user_id": "019513a4-...",
      "question": "We have conflicting information about Kendra's loves. Is it \"Kendra loves Adidas shoes\" or \"Kendra loves Nike shoes\"?",
      "status": "pending",
      "conflict_edge_ids": ["...", "..."],
      "source_entity": "Kendra",
      "label": "loves",
      "conflicting_facts": ["Kendra loves Adidas shoes", "Kendra loves Nike shoes"],
      "severity": 0.85,
      "created_at": "2026-03-11T...",
      "expires_at": "2026-03-18T..."
    }
  ]
}
```

### `POST /api/v1/memory/:user/clarifications`

Generate clarification requests from detected memory conflicts. Runs the
conflict radar, identifies conflicts above the severity threshold, and creates
targeted clarification questions.

```json
{
  "min_severity": 0.6,
  "max_clarifications": 5
}
```

| Field | Default | Description |
|---|---|---|
| `min_severity` | `0.6` | Minimum conflict severity to generate clarifications for |
| `max_clarifications` | `5` | Maximum number of clarifications to generate |

Returns `201 Created` with the generated clarifications.

### `POST /api/v1/memory/:user/clarifications/:id/resolve`

Manually resolve a pending clarification with an answer. If `winning_edge_id`
is provided, the losing edges are automatically invalidated (superseded).

```json
{
  "answer": "Nike is correct, she switched brands last month",
  "winning_edge_id": "019513a5-..."
}
```

| Field | Required | Description |
|---|---|---|
| `answer` | yes | The answer to the clarification question |
| `winning_edge_id` | no | Which conflicting edge to keep; losers are invalidated |

Returns the updated clarification with `status: "resolved"`.

### `POST /api/v1/memory/:user/clarifications/:id/dismiss`

Dismiss a clarification without resolving it. No request body required.
Returns the updated clarification with `status: "dismissed"`.

### `GET /api/v1/memory/:user/narrative`

Get the current narrative summary for a user. The narrative is an evolving
"story of the user" that distills sessions into a readable summary the agent
can use as high-level context.

Returns `404` if no narrative has been generated yet.

```json
// Response
{
  "user_id": "uuid",
  "version": 3,
  "narrative_text": "This user started as a fitness beginner, transitioned to marathon training, and recently shifted focus to recovery.",
  "chapters": [
    {
      "period": "January 2026",
      "summary": "User began fitness journey with basic questions.",
      "key_changes": ["Added running preference", "Set initial 5K goal"],
      "session_ids": ["uuid1", "uuid2"]
    }
  ],
  "session_count": 12,
  "created_at": "2026-01-15T10:00:00Z",
  "updated_at": "2026-03-10T14:30:00Z"
}
```

### `DELETE /api/v1/memory/:user/narrative`

Delete a user's narrative summary. Returns `204 No Content`.

### `POST /api/v1/memory/:user/narrative/refresh`

Generate or update the narrative summary for a user. Collects session summaries,
builds a prompt, calls the LLM, and persists the result. Requires an LLM provider
to be configured.

```json
// Request
{
  "full_rebuild": false,
  "max_chapters": 10
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `full_rebuild` | `false` | If `true`, regenerate from scratch ignoring previous narrative |
| `max_chapters` | `20` | Maximum number of chapters to include |

Returns `201 Created` with the generated/updated narrative.

**Note**: The `POST /api/v1/memory/:user/context` endpoint accepts an optional
`include_narrative: true` field. When set, the narrative summary is included in
the response as a `narrative` field alongside the context block.

**Webhook event**: `narrative_refreshed` is emitted when a narrative is generated.

### `GET /api/v1/memory/:user/goals`

List goal profiles for a user. Returns both user-specific and global (system-wide)
profiles. Goal profiles define how retrieval weights should be adjusted when
the agent is pursuing a specific objective.

| Query Param | Default | Description |
|-------------|---------|-------------|
| `limit` | `50` | Maximum number of profiles to return (max 200) |

### `POST /api/v1/memory/:user/goals`

Create a new goal profile for a user. Goal names must be unique per user.

```json
// Request
{
  "name": "resolve_ticket",
  "description": "Focus on support issues and recent complaints",
  "entity_category_boosts": {"complaint": 2.0, "preference": 0.5},
  "edge_label_boosts": {"filed_by": 1.5, "resolved_by": 1.8},
  "temporal_bias": 0.7,
  "recency_window_days": 30,
  "boost_keywords": ["urgent", "escalated"],
  "suppress_keywords": ["spam", "test"]
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `name` | required | Unique goal name |
| `description` | `""` | Human-readable description |
| `entity_category_boosts` | `{}` | Category-to-multiplier map (>1.0 = boost, <1.0 = suppress) |
| `edge_label_boosts` | `{}` | Edge label-to-multiplier map |
| `temporal_bias` | `0.0` | -1.0 (historical) to 1.0 (recent). Clamped. |
| `recency_window_days` | `0` | Facts older than this may get suppressed |
| `boost_keywords` | `[]` | Keywords that boost matching facts (case-insensitive) |
| `suppress_keywords` | `[]` | Keywords that suppress matching facts |

Returns `201 Created` with the goal profile.

### `GET /api/v1/memory/:user/goals/:id`

Get a specific goal profile by ID.

### `PUT /api/v1/memory/:user/goals/:id`

Update an existing goal profile. All fields are optional — only provided
fields are updated. The `temporal_bias` is clamped to [-1.0, 1.0].

### `DELETE /api/v1/memory/:user/goals/:id`

Delete a goal profile. Returns `204 No Content`.

**Context integration**: The `POST /api/v1/memory/:user/context` endpoint accepts
an optional `goal` field (string). When set, Mnemo looks up the matching goal
profile and re-scores context facts using the profile's relevance adjustments.
The response includes a `goal_applied` field indicating which goal was used.

### `POST /api/v1/memory/:user/counterfactual`

Simulate retrieval context under hypothetical assumptions. Returns a full context
block as if certain facts were different — without modifying actual memory state.
This is a read-only operation; no state is changed.

```json
// Request
{
  "query": "What brand does the user prefer?",
  "hypotheticals": [
    {
      "entity": "user",
      "attribute": "brand_preference",
      "value": "Adidas",
      "confidence": 0.95
    }
  ],
  "max_tokens": 2000,
  "min_relevance": 0.0
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `query` | required | The query to retrieve context for |
| `hypotheticals` | required | Array of fact overrides (at least one) |
| `session` | `null` | Optional session scope |
| `max_tokens` | `2000` | Maximum token budget |
| `min_relevance` | `0.0` | Minimum relevance threshold |

Each hypothetical has:
- `entity` — entity name to match (case-insensitive)
- `attribute` — relationship label to override
- `value` — the hypothetical value
- `confidence` — confidence of the hypothetical (default: 0.9)

```json
// Response
{
  "context": "- user brand_preference Adidas\n- user color_preference blue",
  "token_count": 15,
  "facts": [...],
  "counterfactual_diff": {
    "overridden_facts": [
      {
        "original": {"id": "...", "fact": "user prefers Nike", ...},
        "replaced_by_index": 0,
        "hypothetical": {"entity": "user", "attribute": "brand_preference", "value": "Adidas", "confidence": 0.95}
      }
    ],
    "injected_count": 1,
    "novel_hypotheticals": []
  },
  "latency_ms": 42
}
```

The `counterfactual_diff` shows:
- `overridden_facts` — real facts replaced by hypotheticals
- `injected_count` — total hypotheticals applied
- `novel_hypotheticals` — hypotheticals that didn't match existing facts (added as new)

### `POST /api/v1/memory/:user/causal_recall`

Explain why memory was retrieved by returning fact-to-episode lineage chains.

```json
// Request
{
  "query": "What does Kendra prefer?",
  "session": "default",
  "mode": "hybrid",
  "time_intent": "current",
  "as_of": "2025-01-01T00:00:00Z",
  "max_tokens": 700
}
```

`query` is required. Other fields are optional.

```json
// Response 200
{
  "query": "What does Kendra prefer?",
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "mode": "hybrid",
  "retrieval_sources": ["semantic_search", "graph_traversal"],
  "chains": [
    {
      "id": "fact:019513a4-9d3a-7000-8000-000000000111",
      "confidence": 0.94,
      "reason": "Matched fact 'prefers' with 1 supporting episode(s)",
      "fact": {
        "fact_id": "019513a4-9d3a-7000-8000-000000000111",
        "source_entity": "Kendra",
        "target_entity": "Nike",
        "label": "prefers",
        "text": "Kendra prefers Nike running shoes",
        "valid_at": "2025-01-01T00:00:00Z",
        "invalid_at": null,
        "relevance": 0.82
      },
      "source_episodes": [
        {
          "episode_id": "019513a4-9d3a-7000-8000-000000000222",
          "session_id": "019513a4-8c1f-7000-8000-000000000002",
          "role": "user",
          "created_at": "2025-01-01T00:00:00Z",
          "relevance": 0.74,
          "preview": "Kendra prefers Nike running shoes."
        }
      ]
    }
  ],
  "summary": "1 causal chains built from 1 facts and 1 episodes"
}
```

### `POST /api/v1/memory/:user/time_travel/trace`

Trace how memory-backed answers evolve across a time window.

`timeline` events may include `request_id` for joins across API calls, ingest processing, and webhook delivery.

```json
// Request
{
  "query": "What does Kendra prefer?",
  "from": "2025-02-01T00:00:00Z",
  "to": "2025-04-01T00:00:00Z",
  "session": "default",
  "max_tokens": 500,
  "min_relevance": 0.3,
  "contract": "historical_strict",
  "retrieval_policy": "balanced"
}
```

`session`, `max_tokens`, `min_relevance`, `contract`, and `retrieval_policy` are optional.

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "query": "What does Kendra prefer?",
  "from": "2025-02-01T00:00:00Z",
  "to": "2025-04-01T00:00:00Z",
  "session": "default",
  "contract_applied": "historical_strict",
  "retrieval_policy_applied": "balanced",
  "retrieval_policy_diagnostics": {
    "effective_max_tokens": 500,
    "effective_min_relevance": 0.3,
    "effective_temporal_intent": "auto",
    "effective_temporal_weight": null
  },
  "snapshot_from": {
    "as_of": "2025-02-01T00:00:00Z",
    "token_count": 120,
    "fact_count": 1,
    "episode_count": 1,
    "top_facts": [],
    "top_episodes": []
  },
  "snapshot_to": {
    "as_of": "2025-04-01T00:00:00Z",
    "token_count": 230,
    "fact_count": 1,
    "episode_count": 2,
    "top_facts": [],
    "top_episodes": []
  },
  "gained_facts": [
    {
      "id": "019513a4-9d3a-7000-8000-000000000777",
      "source_entity": "Kendra",
      "target_entity": "Nike",
      "label": "prefers",
      "fact": "Kendra prefers Nike",
      "valid_at": "2025-03-10T00:00:00Z",
      "invalid_at": null,
      "relevance": 0.88
    }
  ],
  "lost_facts": [
    {
      "id": "019513a4-9d3a-7000-8000-000000000666",
      "source_entity": "Kendra",
      "target_entity": "Adidas",
      "label": "prefers",
      "fact": "Kendra prefers Adidas",
      "valid_at": "2025-01-10T00:00:00Z",
      "invalid_at": "2025-02-20T00:00:00Z",
      "relevance": 0.81
    }
  ],
  "gained_episodes": [],
  "lost_episodes": [],
  "timeline": [
    {
      "at": "2025-02-20T00:00:00Z",
      "event_type": "fact_superseded",
      "description": "Superseded: Kendra prefers Adidas"
    },
    {
      "at": "2025-03-10T00:00:00Z",
      "event_type": "fact_added",
      "description": "Kendra prefers Nike"
    }
  ],
  "summary": "4 timeline events; 1 gained facts, 1 lost facts; 1 gained episodes, 0 lost episodes"
}
```

Response fields include `retrieval_policy_diagnostics` (effective resolved policy values), `gained_episodes` / `lost_episodes` (episode-level diffs mirroring `gained_facts` / `lost_facts`), and enriched snapshot objects with `token_count`, `top_facts`, and `top_episodes`.

### `POST /api/v1/memory/:user/time_travel/summary`

Lightweight compare endpoint for fast RCA render paths.

```json
// Request
{
  "query": "What changed about Kendra preferences?",
  "from": "2025-02-01T00:00:00Z",
  "to": "2025-04-01T00:00:00Z",
  "session": "default",
  "contract": "historical_strict",
  "retrieval_policy": "balanced"
}
```

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "from": "2025-02-01T00:00:00Z",
  "to": "2025-04-01T00:00:00Z",
  "contract_applied": "historical_strict",
  "retrieval_policy_applied": "balanced",
  "fact_count_from": 1,
  "fact_count_to": 2,
  "episode_count_from": 1,
  "episode_count_to": 3,
  "gained_fact_count": 1,
  "lost_fact_count": 0,
  "gained_episode_count": 2,
  "lost_episode_count": 0,
  "summary": "1 gained facts, 0 lost facts; 2 gained episodes, 0 lost episodes"
}
```

### `GET /api/v1/memory/webhooks`

List all registered webhook subscriptions, sorted newest-first.

```json
// Response 200
{
  "data": [
    {
      "id": "019cba8c-4929-7cd0-...",
      "user_id": "019cba8c-44b6-7653-...",
      "user_identifier": "kendra",
      "target_url": "https://example.com/hooks/memory",
      "events": ["head_advanced", "conflict_detected"],
      "enabled": true,
      "created_at": "2026-03-04T20:31:21Z",
      "updated_at": "2026-03-04T20:31:21Z"
    }
  ],
  "count": 1
}
```

> **Note:** `signing_secret` is never included in list responses.

### `POST /api/v1/memory/webhooks`

Register a per-user webhook subscription for memory lifecycle events.

```json
// Request
{
  "user": "kendra",
  "target_url": "https://example.com/hooks/memory",
  "signing_secret": "whsec_abc123",
  "events": ["head_advanced", "conflict_detected"],
  "enabled": true
}
```

- `events` is optional; defaults to all event types: `fact_added`, `fact_superseded`, `head_advanced`, `conflict_detected`.
- `signing_secret` is optional; when present, Mnemo signs each delivery using HMAC-SHA256 and sends `x-mnemo-signature` as `t=<unix>,v1=<hex>` over `"<timestamp>.<raw_body>"`.
- Deliveries use retry with exponential backoff (default 3 attempts).
- See `docs/WEBHOOKS.md` for signature verification snippets and delivery semantics.

```json
// Response 201
{
  "ok": true,
  "webhook": {
    "id": "019513a4-9d3a-7000-8000-000000000444",
    "user_id": "019513a4-7e2b-7000-8000-000000000001",
    "user_identifier": "kendra",
    "target_url": "https://example.com/hooks/memory",
    "events": ["head_advanced", "conflict_detected"],
    "enabled": true,
    "created_at": "2026-03-03T12:00:00Z",
    "updated_at": "2026-03-03T12:00:00Z"
  }
}
```

### `GET /api/v1/memory/webhooks/:id/events`

List recent events captured for a webhook subscription.

Query params:
- `limit` (optional, default `100`, max `1000`)
- `event_type` (optional filter)

```json
// Response 200
{
  "webhook_id": "019513a4-9d3a-7000-8000-000000000444",
  "count": 1,
  "events": [
    {
      "id": "019513a4-9d3a-7000-8000-000000000555",
      "webhook_id": "019513a4-9d3a-7000-8000-000000000444",
      "event_type": "head_advanced",
      "user_id": "019513a4-7e2b-7000-8000-000000000001",
      "payload": {
        "session_id": "019513a4-8c1f-7000-8000-000000000002",
        "head_episode_id": "019513a4-9d3a-7000-8000-000000000003"
      },
      "created_at": "2026-03-03T12:01:00Z",
      "request_id": "req_01hxy...",
      "attempts": 1,
      "delivered": true,
      "dead_letter": false,
      "delivered_at": "2026-03-03T12:01:00Z"
    }
  ]
}
```

### `GET /api/v1/memory/webhooks/:id/events/dead-letter`

List only dead-lettered events (events that exhausted retries without a successful delivery).

### `GET /api/v1/memory/webhooks/:id/events/replay`

Cursor-style replay API for webhook event consumers.

Query params:
- `after_event_id` (optional, exclusive cursor)
- `limit` (optional, default `100`, max `1000`)
- `include_delivered` (optional, default `true`)
- `include_dead_letter` (optional, default `true`)

### `POST /api/v1/memory/webhooks/:id/events/:event_id/retry`

Manually queue a re-delivery attempt for a specific event.

```json
// Request
{
  "force": false
}
```

`force=true` allows re-delivery even if event is already marked delivered.

```json
// Response 200
{
  "webhook_id": "019513a4-9d3a-7000-8000-000000000444",
  "event_id": "019513a4-9d3a-7000-8000-000000000555",
  "queued": true,
  "reason": "delivery retry queued",
  "event": {
    "id": "019513a4-9d3a-7000-8000-000000000555",
    "event_type": "head_advanced",
    "attempts": 1,
    "delivered": false,
    "dead_letter": false
  }
}
```

`event` is an optional snapshot of the current webhook event row after retry bookkeeping updates.

### `GET /api/v1/memory/webhooks/:id/audit`

List webhook operational audit records (`webhook_registered`, `retry_queued`, `delivery_dead_letter`, etc).

### `GET /api/v1/memory/webhooks/:id/stats`

Get webhook delivery telemetry counters (pending, delivered, dead-letter, recent failures, circuit state).

| Query Param | Type | Default | Description |
|-------------|------|---------|-------------|
| `window_seconds` | int | `300` | Rolling window for failure rate calculation (1–86400) |

### `GET /api/v1/memory/webhooks/:id`

Fetch webhook configuration by ID.

### `PATCH /api/v1/memory/webhooks/:id`

Partially update a webhook subscription. Only the provided fields are changed.

```json
// Request (all fields optional)
{
  "target_url": "https://updated.example/hook",
  "events": ["fact_added", "fact_superseded"],
  "enabled": false,
  "signing_secret": "new-secret"
}
```

If `target_url` is changed, the same validation as registration applies:
- Must be a valid `http://` or `https://` URL
- When `MNEMO_REQUIRE_TLS=true`, must use `https://`
- Must pass the user's `webhook_domain_allowlist` policy (if set)

**Response** `200 OK`:
```json
{
  "ok": true,
  "webhook": {
    "id": "...",
    "user_id": "...",
    "target_url": "https://updated.example/hook",
    "events": ["fact_added", "fact_superseded"],
    "enabled": false,
    "created_at": "...",
    "updated_at": "..."
  }
}
```

### `DELETE /api/v1/memory/webhooks/:id`

Delete webhook configuration and retained in-memory event records.

### `GET /api/v1/policies/:user`

Fetch effective governance policy for a user identifier.

### `PUT /api/v1/policies/:user`

Upsert user governance policy.

```json
// Request
{
  "retention_days_message": 365,
  "retention_days_text": 180,
  "retention_days_json": 90,
  "webhook_domain_allowlist": ["hooks.acme.example"],
  "default_memory_contract": "default",
  "default_retrieval_policy": "balanced"
}
```

- `webhook_domain_allowlist` blocks webhook registrations outside allowed hosts/subdomains.
- `default_memory_contract` and `default_retrieval_policy` are applied when memory context/trace/summary requests omit those fields.
- retention fields (`retention_days_*`) are enforced on episode writes (`/api/v1/sessions/:session_id/episodes*`).

### `POST /api/v1/policies/:user/preview`

Estimate policy impact before applying it.

```json
// Request
{
  "retention_days_message": 30,
  "retention_days_text": 90,
  "retention_days_json": 180,
  "webhook_domain_allowlist": ["hooks.acme.example"],
  "default_memory_contract": "support_safe",
  "default_retrieval_policy": "precision"
}
```

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "current_policy": {
    "retention_days_message": 3650,
    "retention_days_text": 3650,
    "retention_days_json": 3650,
    "webhook_domain_allowlist": [],
    "default_memory_contract": "default",
    "default_retrieval_policy": "balanced"
  },
  "preview_policy": {
    "retention_days_message": 30,
    "retention_days_text": 90,
    "retention_days_json": 180,
    "webhook_domain_allowlist": ["hooks.acme.example"],
    "default_memory_contract": "support_safe",
    "default_retrieval_policy": "precision"
  },
  "estimated_affected_episodes_total": 42,
  "estimated_affected_message_episodes": 20,
  "estimated_affected_text_episodes": 15,
  "estimated_affected_json_episodes": 7,
  "confidence": "estimated"
}
```

### `GET /api/v1/policies/:user/audit`

List governance audit records (`policy_updated`, `policy_violation_webhook_domain`, `session_deleted`, `user_deleted`).

### `GET /api/v1/policies/:user/violations`

Query policy violation audit events inside a time window.

Query params:
- `from` (required, RFC3339 timestamp)
- `to` (required, RFC3339 timestamp)
- `limit` (optional, default `100`, max `1000`)

Only governance audit rows with actions prefixed by `policy_violation_` are returned.

```json
// Response 200
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "from": "2026-03-01T00:00:00Z",
  "to": "2026-03-04T00:00:00Z",
  "count": 1,
  "violations": [
    {
      "at": "2026-03-03T13:22:11Z",
      "action": "policy_violation_webhook_domain",
      "request_id": "req_01hxy...",
      "details": {
        "target_url": "https://bad.example/webhook",
        "allowlist": ["hooks.acme.example"]
      }
    }
  ]
}
```

---

## Import API

Async import jobs for migrating existing chat history into Mnemo.

### `POST /api/v1/import/chat-history`

Start an import job.

```json
// Request
{
  "user": "kendra",
  "source": "ndjson",
  "idempotency_key": "import-2026-03-03-001",
  "dry_run": false,
  "default_session": "Imported History",
  "payload": [
    {
      "session": "Imported History",
      "role": "user",
      "content": "I switched to Nike.",
      "created_at": "2025-02-01T10:00:00Z"
    }
  ]
}
```

- `source` currently supports: `ndjson`, `chatgpt_export`, `gemini_export`
- `dry_run=true` validates and counts importable rows without writing episodes.
- `idempotency_key` replays a prior job for the same user/key pair without creating duplicate imports.
- For format details and migration walkthroughs, see `docs/IMPORTING_CHAT_HISTORY.md`.

```json
// Response 202
{
  "ok": true,
  "job_id": "01954b4f-4f35-7000-8000-000000000001",
  "status": "queued"
}
```

If the same `user` + `idempotency_key` is submitted again, the server returns `200` with the original `job_id` and latest job status.

### `GET /api/v1/import/jobs/:job_id`

Get import job status and counters.

```json
// Response 200
{
  "id": "01954b4f-4f35-7000-8000-000000000001",
  "source": "ndjson",
  "user": "kendra",
  "dry_run": false,
  "status": "completed",
  "total_messages": 24,
  "imported_messages": 24,
  "failed_messages": 0,
  "sessions_touched": 2,
  "errors": [],
  "created_at": "2026-03-03T03:10:14Z",
  "started_at": "2026-03-03T03:10:14Z",
  "finished_at": "2026-03-03T03:10:15Z"
}
```

---

## Agent Identity Substrate (P0)

These endpoints provide a separated agent identity/experience layer.

### `GET /api/v1/agents/:agent_id/identity`

Returns current identity profile. Creates a default profile on first access.

### `PUT /api/v1/agents/:agent_id/identity`

Update the identity core.

```json
{
  "core": {
    "mission": "Resolve user issues accurately and safely.",
    "boundaries": {
      "never_claim_human_experience": true
    }
  }
}
```

### `GET /api/v1/agents/:agent_id/identity/versions?limit=20`

Lists recent identity snapshots (newest first).

### `GET /api/v1/agents/:agent_id/identity/audit?limit=50`

Lists identity audit events (`created`, `updated`, `rolled_back`).

Each event includes **witness chain** fields for tamper-evidence:

| Field | Description |
|---|---|
| `prev_hash` | SHA-256 hash of the preceding audit event (`null` for genesis) |
| `event_hash` | SHA-256 of `action\|from_version\|to_version\|prev_hash\|timestamp_ms` |

The chain is tamper-evident: any deletion, modification, or reordering of events
breaks the hash chain and is detectable via the verify endpoint.

### `GET /api/v1/agents/:agent_id/identity/audit/verify`

Walk the full witness chain and verify cryptographic integrity.

**Response:**

```json
{
  "valid": true,
  "chain_length": 5,
  "breaks": []
}
```

If tampering is detected, `valid` is `false` and `breaks` contains entries:

```json
{
  "valid": false,
  "chain_length": 5,
  "breaks": [
    {
      "index": 2,
      "event_id": "019513a4-...",
      "reason": "event_hash mismatch: stored=abc..., computed=def..."
    }
  ]
}
```

### `POST /api/v1/agents/:agent_id/identity/rollback`

Rollback identity core to a prior version while preserving an append-only version history.

```json
{
  "target_version": 2,
  "reason": "revert unsafe identity mutation"
}
```

### `POST /api/v1/agents/:agent_id/identity/verified`

Proof-carrying identity update. The proposer attaches a Merkle proof that every
top-level key in `core` is a member of the canonical identity allowlist. The
server verifies the proof (cheap) and applies the update only if verification
passes. The proof is stored alongside the response for auditability.

The canonical allowlist keys are: `boundaries`, `capabilities`, `mission`,
`persona`, `style`, `values`.

**Generating a proof**: Build an `AllowlistMerkleTree` from the canonical
allowlist, then call `tree.prove(key)` for each top-level key in your candidate
core. Collect these into an `IdentityUpdateProof` with the tree's root.

```json
{
  "core": {
    "mission": "help users accomplish tasks",
    "style": "concise and direct"
  },
  "proof": {
    "merkle_root": "a1b2c3...64hex...",
    "key_proofs": [
      {
        "key": "mission",
        "leaf_index": 2,
        "siblings": [
          { "hash": "d4e5f6...64hex...", "position": "right" },
          { "hash": "a7b8c9...64hex...", "position": "left" }
        ],
        "root": "a1b2c3...64hex..."
      },
      {
        "key": "style",
        "leaf_index": 4,
        "siblings": [
          { "hash": "f0e1d2...64hex...", "position": "right" },
          { "hash": "c3b4a5...64hex...", "position": "left" }
        ],
        "root": "a1b2c3...64hex..."
      }
    ]
  }
}
```

Response includes the updated identity profile and the verification result:

```json
{
  "identity": {
    "agent_id": "support-bot",
    "version": 6,
    "core": { "mission": "...", "style": "..." },
    "created_at": "...",
    "updated_at": "..."
  },
  "verification": {
    "verified": true,
    "key_results": [
      { "key": "mission", "valid": true },
      { "key": "style", "valid": true }
    ],
    "merkle_root": "a1b2c3...64hex..."
  }
}
```

Verification checks:
1. Proof `merkle_root` matches the canonical allowlist Merkle root.
2. Every top-level key in `core` has a valid membership proof.
3. No extra proofs for keys not present in `core`.
4. No forbidden substrings (`user`, `session`, `episode`, `email`, `phone`, `address`, `external_id`) at any depth.

Errors:
- `400` if proof verification fails (includes per-key error details).
- `404` if the agent does not exist.

### `POST /api/v1/agents/:agent_id/branches`

Create a COW (copy-on-write) branch from the agent's current identity.
Branches allow A/B testing of personality changes: create a branch, run it,
compare, then merge or discard.

```json
{
  "branch_name": "warmer-tone",
  "description": "Test a friendlier persona",
  "core_override": { "tone": "warm", "style": "conversational" }
}
```

- `branch_name`: 1-64 chars, alphanumeric + hyphens/underscores only.
- `core_override`: Optional. If omitted, branch starts with parent's current core.
- Returns: `BranchInfo` (metadata + identity).

### `GET /api/v1/agents/:agent_id/branches`

List all branches for an agent. Returns `Vec<BranchMetadata>`.

### `GET /api/v1/agents/:agent_id/branches/:branch_name`

Get branch details (metadata + current identity). Returns `BranchInfo`.

### `PUT /api/v1/agents/:agent_id/branches/:branch_name/identity`

Update a branch's identity core. Same body as `PUT /identity`.

### `POST /api/v1/agents/:agent_id/branches/:branch_name/merge`

Merge a branch back into the parent's main identity. The branch's current
`core` replaces the parent's `core` (just like a normal identity update).
Returns `MergeResult` with the merged identity and version info.

### `DELETE /api/v1/agents/:agent_id/branches/:branch_name`

Delete a branch without merging. Returns `204 No Content`.

### `POST /api/v1/agents/:agent_id/fork`

Fork an agent to create a new independent agent with selective experience transfer.
The new agent receives a copy of the parent's identity (optionally overridden) and
a filtered subset of the parent's experience events. Lineage metadata is stored so
the relationship between parent and child is always traceable.

```json
{
  "new_agent_id": "support-bot-emea",
  "core_override": {
    "persona": "You are a multilingual EMEA support agent",
    "tone": "formal"
  },
  "experience_filter": {
    "categories": ["interaction_pattern", "domain_knowledge"],
    "min_confidence": 0.7,
    "min_weight": 0.5,
    "max_events": 100
  },
  "description": "Regional fork for EMEA support"
}
```

| Field | Required | Description |
|---|---|---|
| `new_agent_id` | yes | Unique ID for the forked agent. 1-128 chars, alphanumeric + hyphens/underscores/dots. Must not contain `:`, `/`, or `..`. |
| `core_override` | no | JSON object to replace the parent's identity core. If omitted, the parent's core is copied verbatim. |
| `experience_filter` | no | Filter criteria for selecting which experience events to transfer. If omitted, all events are transferred. |
| `experience_filter.categories` | no | Only transfer events in these categories. Empty array = all categories. |
| `experience_filter.min_confidence` | no | Minimum confidence threshold (0.0–1.0). Events below are excluded. |
| `experience_filter.min_weight` | no | Minimum effective weight. Events below are excluded. |
| `experience_filter.max_events` | no | Maximum number of events to transfer. |
| `description` | no | Human-readable reason for the fork. |

Response (`ForkResult`):

```json
{
  "new_agent": {
    "agent_id": "support-bot-emea",
    "version": 1,
    "core": { "persona": "...", "tone": "formal" },
    "created_at": "2026-03-11T...",
    "updated_at": "2026-03-11T..."
  },
  "lineage": {
    "parent_agent_id": "support-bot",
    "parent_version": 5,
    "forked_at": "2026-03-11T...",
    "description": "Regional fork for EMEA support",
    "experience_events_transferred": 42,
    "experience_filter": { "categories": ["interaction_pattern", "domain_knowledge"], "min_confidence": 0.7, "min_weight": 0.5, "max_events": 100 }
  }
}
```

Errors:
- `400` if `new_agent_id` fails validation or the source agent does not exist.
- `409` if an agent with `new_agent_id` already exists.

### `POST /api/v1/agents/:agent_id/experience`

Add an adaptive experience event. The server computes a `fisher_importance` score
(EWC++ — Elastic Weight Consolidation) measuring how structurally important this
experience is to the agent's current identity. High-importance events resist
temporal decay, keeping load-bearing experiences influential even when old.

```json
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "session_id": "019513a4-8c1f-7000-8000-000000000002",
  "category": "interaction_pattern",
  "signal": "user_prefers_bulleted_action_plans",
  "confidence": 0.8,
  "weight": 0.7,
  "decay_half_life_days": 30
}
```

Response includes `fisher_importance` (0.0–1.0) computed from novelty, confidence alignment, and weight signal relative to existing events in the same category.

### `GET /api/v1/agents/:agent_id/experience/importance?limit=50`

Returns experience events ranked by Fisher importance (descending). Each entry includes:

| Field | Description |
|---|---|
| `id` | Event UUID |
| `category` | Event category |
| `signal` | The experience signal text |
| `fisher_importance` | EWC++ importance score (0.0–1.0) |
| `effective_weight` | EWC++-enhanced weight (resists decay for high-importance events) |
| `raw_weight` | Original weight value |
| `confidence` | Confidence score |
| `created_at` | UTC timestamp |

The `effective_weight` formula: `weight × confidence × decay × (1 + fisher_importance × λ)` where `λ = 2.0`.

### `POST /api/v1/agents/:agent_id/context`

Identity-aware context assembly. Combines identity core, recent experience signals, and user memory context.

### `POST /api/v1/agents/:agent_id/promotions`

Create a pending promotion proposal (gated). Requires at least 3 `source_event_ids`.

### `GET /api/v1/agents/:agent_id/promotions?limit=50`

List promotion proposals (newest first).

### `POST /api/v1/agents/:agent_id/promotions/:proposal_id/approve`

Approve a pending promotion and apply `candidate_core` to identity core.

### `POST /api/v1/agents/:agent_id/promotions/:proposal_id/reject`

Reject a pending promotion without identity mutation.

---

## GNN Retrieval Feedback

The GNN (Graph Neural Network) re-ranking layer optionally enhances retrieval by learning from feedback. When enabled (`MNEMO_GNN_ENABLED=true`), retrieval results are re-ranked using graph attention over the knowledge graph. The feedback endpoint lets you train the GNN by reporting which retrieved entities were actually useful.

### `POST /api/v1/memory/feedback`

Report which retrieved entities were useful (positive signal for GNN training).

```json
// Request
{
  "positive_entity_ids": ["uuid-1", "uuid-2"],  // required: entities that were actually used
  "all_entity_ids": ["uuid-1", "uuid-2", "uuid-3", "uuid-4"]  // optional: all entities returned by retrieval
}

// Response 200
{
  "accepted": true,
  "positive_count": 2
}
```

**Notes:**
- `positive_entity_ids` is required and must be non-empty
- `all_entity_ids` is optional; if omitted, only positive IDs are considered
- Feedback is a no-op if GNN re-ranking is not enabled
- The GNN model updates incrementally with each feedback call (<1ms)

---

## Users

Users represent end-users of your AI agent application. Each user has an isolated knowledge graph.

### `POST /api/v1/users`

Create a user.

```json
// Request
{
  "name": "Kendra",
  "email": "kendra@example.com",
  "external_id": "usr_abc123",
  "metadata": { "plan": "pro" }
}
```

All fields except `name` are optional. If `id` is omitted, a UUIDv7 is generated.

```json
// Response 201
{
  "id": "019513a4-7e2b-7000-8000-000000000001",
  "name": "Kendra",
  "email": "kendra@example.com",
  "external_id": "usr_abc123",
  "metadata": { "plan": "pro" },
  "created_at": "2026-03-01T12:00:00Z",
  "updated_at": "2026-03-01T12:00:00Z"
}
```

### `GET /api/v1/users/:id`

Get a user by Mnemo ID.

### `GET /api/v1/users/external/:external_id`

Get a user by your application's external ID. Useful when you don't want to store Mnemo IDs.

### `PUT /api/v1/users/:id`

Partial update. Only provided fields are changed.

```json
// Request
{
  "name": "Kendra Smith",
  "metadata": { "plan": "enterprise" }
}
```

### `DELETE /api/v1/users/:id`

Deletes the user and all associated vectors (GDPR-compliant). Sessions, episodes, entities, and edges remain in Redis for audit purposes unless you delete them separately.

### `GET /api/v1/users?limit=20&after=<uuid>`

List users with cursor-based pagination. `after` is the last user ID from the previous page.

---

## Sessions

Sessions represent conversation threads. They belong to a user and contain episodes.

### `POST /api/v1/sessions`

```json
// Request
{
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "name": "Support Chat #4521",
  "metadata": { "channel": "web" }
}
```

```json
// Response 201
{
  "id": "019513a4-8c1f-7000-8000-000000000002",
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "name": "Support Chat #4521",
  "metadata": { "channel": "web" },
  "episode_count": 0,
  "summary": null,
  "summary_tokens": 0,
  "head_episode_id": null,
  "head_updated_at": null,
  "head_version": 0,
  "created_at": "2026-03-01T12:00:01Z",
  "updated_at": "2026-03-01T12:00:01Z",
  "last_activity_at": null
}
```

### `GET /api/v1/sessions/:id`

### `PUT /api/v1/sessions/:id`

### `DELETE /api/v1/sessions/:id`

### `GET /api/v1/users/:user_id/sessions?limit=20&after=<uuid>`

List sessions for a user, newest first.

---

## Episodes

Episodes are the atomic unit of data ingestion. Every message, event, or document you send to Mnemo is an episode.

### `POST /api/v1/sessions/:session_id/episodes`

Add a single episode.

```json
// Request — chat message
{
  "type": "message",
  "content": "I just switched from Adidas to Nike running shoes!",
  "role": "user",
  "name": "Kendra"
}
```

```json
// Request — structured JSON event
{
  "type": "json",
  "content": "{\"event\":\"purchase\",\"item\":\"Nike Air Max\",\"price\":129.99}",
  "metadata": { "source": "crm" }
}
```

```json
// Request — unstructured text
{
  "type": "text",
  "content": "Meeting notes: Kendra mentioned she's training for the Boston Marathon..."
}
```

The `type` field determines how the content is processed:

| Type | Description | `role` | `name` |
|------|-------------|--------|--------|
| `message` | Chat message | Required | Optional (aids entity resolution) |
| `json` | Structured event data | Ignored | Ignored |
| `text` | Unstructured text | Ignored | Ignored |

**Processing:** Episodes are stored synchronously (the API returns immediately) and processed asynchronously. The background ingestion worker extracts entities and relationships, builds the knowledge graph, and generates embeddings.

```json
// Response 201
{
  "id": "019513a4-9d3a-7000-8000-000000000003",
  "session_id": "019513a4-8c1f-7000-8000-000000000002",
  "user_id": "019513a4-7e2b-7000-8000-000000000001",
  "type": "message",
  "content": "I just switched from Adidas to Nike running shoes!",
  "role": "user",
  "name": "Kendra",
  "metadata": {},
  "created_at": "2026-03-01T12:00:02Z",
  "ingested_at": "2026-03-01T12:00:02Z",
  "processing_status": "pending",
  "entity_ids": [],
  "edge_ids": []
}
```

The `processing_status` field tracks the episode through the pipeline:

| Status | Meaning |
|--------|---------|
| `pending` | Stored, awaiting extraction |
| `processing` | Currently being processed by the ingestion worker |
| `completed` | Entities and relationships extracted successfully |
| `failed` | Extraction failed (see `processing_error`) |
| `skipped` | Not processed (empty content or system message) |

### `POST /api/v1/sessions/:session_id/episodes/batch`

Add multiple episodes at once. Ideal for backfilling conversation history.

```json
// Request
{
  "episodes": [
    { "type": "message", "role": "user", "name": "Kendra", "content": "What running shoes do you recommend?" },
    { "type": "message", "role": "assistant", "content": "Based on your preferences, I'd suggest the Nike Pegasus!" },
    { "type": "message", "role": "user", "name": "Kendra", "content": "I'll check those out, thanks!" }
  ]
}
```

### `GET /api/v1/sessions/:session_id/episodes?limit=50&after=<uuid>`

### `GET /api/v1/episodes/:id`

---

## Context (Primary Endpoint)

This is the endpoint your agent calls on every turn. It retrieves relevant knowledge from the user's graph and assembles a context string ready for LLM injection.

### Retrieval reranking

After the parallel semantic + graph search, results are merged and reranked. The strategy is set in `config/default.toml` under `[retrieval]`:

```toml
[retrieval]
# "rrf"  — Reciprocal Rank Fusion (default): boosts candidates appearing in multiple ranked lists.
# "mmr"  — Maximal Marginal Relevance: penalises near-duplicate results for more diverse context.
reranker = "rrf"
```

There is no environment-variable override for `reranker` — use the TOML config file (`MNEMO_CONFIG=/path/to/mnemo.toml`) to change it.

### `POST /api/v1/users/:user_id/context`

```json
// Request
{
  "session_id": "019513a4-8c1f-7000-8000-000000000002",
  "messages": [
    { "role": "user", "content": "What running shoes should I recommend to Kendra?" }
  ],
  "max_tokens": 500,
  "min_relevance": 0.3,
  "search_types": ["hybrid"]
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `session_id` | UUID | null | Optional session scope |
| `messages` | array | `[]` | Recent messages used as the retrieval query |
| `max_tokens` | int | `500` | Token budget for the context string |
| `min_relevance` | float | `0.3` | Minimum similarity threshold (0.0–1.0) |
| `search_types` | array | `["hybrid"]` | `semantic`, `full_text`, `graph`, `hybrid` |
| `temporal_filter` | ISO 8601 | null | Only return facts valid at this time |
| `as_of` | ISO 8601 | null | Point-in-time target (historical recall) |
| `mode` | enum | `hybrid` | `head`, `hybrid`, `historical` |
| `time_intent` | enum | `auto` | `auto`, `current`, `recent`, `historical` |
| `temporal_weight` | float | null | Override temporal influence (0.0–1.0) |
| `filters` | object | null | Metadata prefilter for episode candidates |

> **Note:** `temporal_diagnostics` and `metadata_filter_diagnostics` are **response-only** fields (not request parameters). They appear in the 200 response with resolved temporal intent/scored result counts and candidate counts before/after metadata filtering, respectively.

```json
// Response 200
{
  "context": "Known entities:\n- Kendra (person): A runner training for the Boston Marathon\n- Nike (organization): Athletic shoe company\n\nCurrent facts:\n- Kendra recently switched from Adidas to Nike running shoes\n- Kendra is training for the Boston Marathon\n",
  "token_count": 47,
  "entities": [
    {
      "id": "...",
      "name": "Kendra",
      "entity_type": "person",
      "summary": "A runner training for the Boston Marathon",
      "relevance": 0.95
    }
  ],
  "facts": [
    {
      "id": "...",
      "source_entity": "Kendra",
      "target_entity": "Nike",
      "label": "prefers",
      "fact": "Kendra recently switched from Adidas to Nike running shoes",
      "valid_at": "2026-03-01T12:00:02Z",
      "relevance": 0.92
    }
  ],
  "episodes": [],
  "latency_ms": 23,
  "sources": ["semantic_search", "graph_traversal"]
}
```

**Usage pattern:** Inject `context.context` into your agent's system prompt:

```
System: You are a helpful shopping assistant.

{context.context}

User: What running shoes should I recommend to Kendra?
```

---

## Entities

Entities are nodes in the knowledge graph — automatically extracted from episodes.

### `GET /api/v1/users/:user_id/entities?limit=20&after=<uuid>`

### `GET /api/v1/entities/:id`

### `DELETE /api/v1/entities/:id`

---

## Edges

Edges are temporal facts connecting two entities.

### `GET /api/v1/users/:user_id/edges`

Query edges with filters:

| Param | Type | Description |
|-------|------|-------------|
| `source_entity_id` | UUID | Filter by source |
| `target_entity_id` | UUID | Filter by target |
| `label` | string | Filter by relationship type |
| `include_invalidated` | bool | Include superseded facts (default: false) |
| `limit` | int | Max results (default: 100) |

```
GET /api/v1/users/:user_id/edges?label=prefers&include_invalidated=true
```

### `GET /api/v1/edges/:id`

### `DELETE /api/v1/edges/:id`

---

## Graph

The graph API provides first-class access to a user's knowledge graph. Entity and edge filtering, neighborhood traversal, shortest-path search, and community detection are all available.

All graph endpoints accept a `:user` path parameter which can be a UUID, `external_id`, or user name.

### `GET /api/v1/graph/:user/entities`

List all entities for a user with optional filtering.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | int | `20` | Max results (1–1000) |
| `after` | UUID | null | Cursor for pagination |
| `entity_type` | string | null | Filter by type (case-insensitive: `person`, `concept`, `organization`, `product`, `location`, `event`) |
| `name` | string | null | Filter by name (case-insensitive substring match) |

```
GET /api/v1/graph/kendra/entities?entity_type=person&limit=10
```

```json
// Response 200
{
  "data": [
    {
      "id": "...", "name": "Kendra", "entity_type": "person",
      "summary": "A runner", "mention_count": 5,
      "community_id": null, "created_at": "...", "updated_at": "..."
    }
  ],
  "count": 1,
  "user_id": "..."
}
```

### `GET /api/v1/graph/:user/entities/:entity_id`

Get a single entity with its outgoing and incoming edges. Returns 404 if the entity does not belong to the specified user (prevents cross-user data leaks).

```json
// Response 200
{
  "id": "...", "name": "Kendra", "entity_type": "person",
  "summary": "A runner", "mention_count": 5,
  "community_id": null, "created_at": "...", "updated_at": "...",
  "outgoing_edges": [
    { "id": "...", "target_entity_id": "...", "label": "prefers", "fact": "...", "valid": true }
  ],
  "incoming_edges": [
    { "id": "...", "source_entity_id": "...", "label": "knows", "fact": "...", "valid": true }
  ]
}
```

### `GET /api/v1/graph/:user/edges`

List edges for a user with optional label, source, and target filters.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | int | `20` | Max results (1–1000) |
| `label` | string | null | Filter by edge label |
| `valid_only` | bool | `true` | Exclude invalidated edges |
| `source_entity_id` | UUID | null | Filter by source entity |
| `target_entity_id` | UUID | null | Filter by target entity |

```
GET /api/v1/graph/kendra/edges?label=prefers&source_entity_id=019...&valid_only=true
```

```json
// Response 200
{
  "data": [
    {
      "id": "...", "source_entity_id": "...", "target_entity_id": "...",
      "label": "prefers", "fact": "Kendra prefers Nike", "confidence": 0.95,
      "valid_at": "...", "invalid_at": null, "valid": true, "created_at": "..."
    }
  ],
  "count": 1,
  "user_id": "..."
}
```

### `GET /api/v1/graph/:user/neighbors/:entity_id`

Multi-hop neighborhood traversal from a seed entity using BFS. Returns 404 if the entity does not belong to the specified user.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `depth` | int | `1` | Max hops (capped at 10) |
| `max_nodes` | int | `50` | Max entities (capped at 500) |
| `valid_only` | bool | `true` | Only follow valid edges |

```json
// Response 200
{
  "seed_entity_id": "...",
  "depth": 1,
  "nodes": [
    { "id": "...", "name": "Kendra", "entity_type": "person", "summary": "...", "depth": 0 },
    { "id": "...", "name": "Nike", "entity_type": "organization", "summary": null, "depth": 1 }
  ],
  "edges": [
    { "id": "...", "source_entity_id": "...", "target_entity_id": "...", "label": "prefers", "fact": "...", "valid": true }
  ],
  "entities_visited": 3
}
```

### `GET /api/v1/graph/:user/path`

Find the shortest path between two entities in the user's knowledge graph using BFS. Returns 404 if either entity does not belong to the specified user.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `from` | UUID | **required** | Source entity ID |
| `to` | UUID | **required** | Target entity ID |
| `max_depth` | int | `10` | Max hops to search (capped at 20) |
| `valid_only` | bool | `true` | Only follow valid edges |

```
GET /api/v1/graph/kendra/path?from=019...abc&to=019...def
```

```json
// Response 200
{
  "from": "019...abc",
  "to": "019...def",
  "found": true,
  "path_length": 2,
  "steps": [
    { "entity_id": "019...abc", "entity_name": "Kendra", "entity_type": "person", "depth": 0 },
    {
      "entity_id": "019...mid", "entity_name": "Running", "entity_type": "concept", "depth": 1,
      "edge": { "id": "...", "source_entity_id": "...", "target_entity_id": "...", "label": "enjoys", "fact": "...", "valid": true }
    },
    {
      "entity_id": "019...def", "entity_name": "Nike", "entity_type": "organization", "depth": 2,
      "edge": { "id": "...", "source_entity_id": "...", "target_entity_id": "...", "label": "produces", "fact": "...", "valid": true }
    }
  ],
  "entities_visited": 8
}
```

When no path exists, `found` is `false`, `path_length` is `0`, and `steps` is empty.

### `GET /api/v1/graph/:user/community`

Run community detection (label propagation) over the user's entity graph.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `max_iterations` | int | `20` | Max label propagation iterations (1–100) |

```json
// Response 200
{
  "user_id": "...",
  "total_entities": 12,
  "community_count": 3,
  "communities": [
    { "community_id": "...", "member_count": 5, "entity_ids": ["...", "..."] },
    { "community_id": "...", "member_count": 4, "entity_ids": ["...", "..."] },
    { "community_id": "...", "member_count": 3, "entity_ids": ["...", "..."] }
  ]
}
```

### `GET /api/v1/entities/:id/subgraph?depth=2&max_nodes=50`

Traverse the knowledge graph from a seed entity using BFS. This is a lower-level endpoint that does not require a user path parameter (the entity ID is globally unique).

```json
// Response 200
{
  "nodes": [
    {
      "entity": { "id": "...", "name": "Kendra", "entity_type": "person", "summary": "..." },
      "depth": 0,
      "outgoing_edges": 3,
      "incoming_edges": 1
    },
    {
      "entity": { "id": "...", "name": "Nike", "entity_type": "organization", "summary": null },
      "depth": 1,
      "outgoing_edges": 0,
      "incoming_edges": 2
    }
  ],
  "edges": [
    {
      "id": "...",
      "source_entity_id": "...",
      "target_entity_id": "...",
      "label": "prefers",
      "fact": "Kendra recently switched to Nike running shoes",
      "valid_at": "2026-03-01T12:00:02Z",
      "invalid_at": null
    }
  ],
  "entities_visited": 5
}
```

---

## Session Messages API

These endpoints provide raw message access for framework adapters (LangChain, LlamaIndex). Messages are episodes projected as role+content pairs, ordered chronologically.

Session IDs are UUIDs (from the `session_id` field returned by `POST /api/v1/memory`).

### `GET /api/v1/sessions/:session_id/messages`

Return all messages for a session in chronological order.

Query params:
- `limit` (optional, default `100`, max `1000`) — maximum messages to return
- `after` (optional, episode UUID) — return only messages after this episode ID

```json
// Response 200
{
  "messages": [
    {
      "idx": 0,
      "id": "019cba12-...",
      "role": "user",
      "content": "Hello from LangChain",
      "created_at": "2026-03-04T12:00:00Z"
    },
    {
      "idx": 1,
      "id": "019cba13-...",
      "role": "assistant",
      "content": "Hello back from AI",
      "created_at": "2026-03-04T12:00:01Z"
    }
  ],
  "count": 2,
  "session_id": "019cba10-..."
}
```

### `DELETE /api/v1/sessions/:session_id/messages`

Clear all messages (episodes) for a session without deleting the session itself.

Required by `MnemoChatMessageHistory.clear()` and `MnemoChatStore.delete_messages()`.

```json
// Response 200
{
  "deleted": true,
  "count": 2
}
```

### `DELETE /api/v1/sessions/:session_id/messages/:idx`

Delete a specific message by 0-based ordinal index within the session.

Returns `400` with `validation_error` if the index is out of bounds.

Required by `MnemoChatStore.delete_message()` and `delete_last_message()`.

```json
// Response 200
{
  "deleted": true,
  "episode_id": "019cba13-..."
}
```

---

## Raw Vector API

These endpoints expose Mnemo as a general-purpose vector database for external systems like [AnythingLLM](https://github.com/Mintplex-Labs/anything-llm). Namespaces are fully isolated from Mnemo's internal entity/edge/episode collections.

Vector IDs can be any string (they are deterministically hashed to UUIDs internally). The original IDs are preserved and returned in search results.

### `POST /api/v1/vectors/:namespace`

Upsert vectors into a namespace. Creates the namespace (Qdrant collection) automatically if it doesn't exist, using the dimension of the first vector.

```json
// Request
{
  "vectors": [
    {
      "id": "doc-chunk-1",
      "vector": [0.1, -0.3, 0.5, ...],
      "metadata": {
        "text": "The quick brown fox",
        "docId": "readme.md",
        "source": "upload"
      }
    }
  ]
}
```

```json
// Response 200
{
  "ok": true,
  "namespace": "workspace-abc",
  "upserted": 1
}
```

Upserting with an existing ID overwrites the vector and metadata (idempotent). Vectors are batched internally in chunks of 500.

### `POST /api/v1/vectors/:namespace/query`

Search vectors by cosine similarity.

```json
// Request
{
  "vector": [0.1, -0.3, 0.5, ...],
  "top_k": 5,
  "min_score": 0.25
}
```

`top_k` defaults to 10. `min_score` defaults to 0.0.

```json
// Response 200
{
  "results": [
    {
      "id": "doc-chunk-1",
      "score": 0.92,
      "payload": {
        "text": "The quick brown fox",
        "docId": "readme.md",
        "source": "upload"
      }
    }
  ],
  "namespace": "workspace-abc"
}
```

Querying a non-existent namespace returns an empty `results` array (not an error).

### `POST /api/v1/vectors/:namespace/delete`

Delete specific vectors by ID.

```json
// Request
{
  "ids": ["doc-chunk-1", "doc-chunk-2"]
}
```

```json
// Response 200
{
  "ok": true,
  "namespace": "workspace-abc",
  "deleted": 2
}
```

Deleting non-existent IDs is a no-op (idempotent).

### `DELETE /api/v1/vectors/:namespace`

Delete an entire namespace and all its vectors.

```json
// Response 200
{
  "ok": true,
  "namespace": "workspace-abc",
  "deleted": true
}
```

Deleting a non-existent namespace is a no-op (idempotent).

### `GET /api/v1/vectors/:namespace/count`

Count total vectors in a namespace.

```json
// Response 200
{
  "namespace": "workspace-abc",
  "count": 1024
}
```

Returns `count: 0` for non-existent namespaces.

### `GET /api/v1/vectors/:namespace/exists`

Check whether a namespace exists.

```json
// Response 200
{
  "namespace": "workspace-abc",
  "exists": true
}
```

---

## Operator Incidents

### `GET /api/v1/ops/incidents`

Returns active incident cards for the operator dashboard. Each incident represents an actionable operational issue (dead-letter backlog, circuit-open webhooks, server errors, policy violations).

Query params:

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `window_seconds` | integer | `300` | Lookback window for time-bounded checks. Clamped to `[1, 86400]`. |

```json
// Response 200
{
  "window_seconds": 300,
  "total_active": 2,
  "incidents": [
    {
      "id": "dead-letter-backlog",
      "kind": "dead_letter_spike",
      "severity": "high",
      "title": "Dead-letter backlog: 12 event(s)",
      "summary": "Webhook delivery has 12 dead-letter event(s) awaiting operator action.",
      "action_label": "Review dead-letter queue",
      "action_href": "/_/webhooks?filter=dead-letter",
      "resource_id": null,
      "resource_label": null,
      "request_id": null,
      "opened_at": null
    }
  ]
}
```

Incident kinds:

| `kind` | `severity` | Trigger |
|--------|-----------|---------|
| `dead_letter_spike` | `high` (>=10) or `medium` | Dead-letter backlog > 0 |
| `pending_backlog` | `medium` | Pending webhook events >= 25 |
| `server_errors` | `high` | HTTP 5xx responses > 0 (process lifetime) |
| `policy_violation` | `medium` | Governance violations within the window |
| `circuit_open` | `high` | Webhook circuit breaker is open |

Incidents are sorted by severity (high first), then recency.

---

## Evidence Export Bundles

Evidence export endpoints return self-contained JSON bundles suitable for SOC 2 auditors, SIEM ingestion, or incident post-mortems. Each bundle wraps the payload in a standard `EvidenceBundleEnvelope` with `kind`, `exported_at`, and `source_path` metadata.

### `GET /api/v1/evidence/webhooks/:id/export`

Export an evidence bundle for a webhook subscription, including subscription config, delivery stats, dead-letter queue, and audit trail.

Path params:
- `id` — UUID of the webhook subscription

Query params:

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `focus` | string | (none) | Free-text annotation embedded in the bundle (e.g. "investigating delivery failures") |
| `source_path` | string | `/_/webhooks/{id}` | Override the envelope `source_path` field |

```json
// Response 200
{
  "kind": "webhook_evidence_bundle",
  "exported_at": "2026-03-10T12:00:00Z",
  "source_path": "/_/webhooks/019...",
  "payload": {
    "webhook": { /* MemoryWebhookSubscription object */ },
    "stats": {
      "webhook_id": "...",
      "total_events": 100,
      "delivered_events": 90,
      "pending_events": 5,
      "dead_letter_events": 5,
      "failed_events": 8,
      "recent_failures": 2,
      "circuit_open": false,
      "circuit_open_until": null,
      "rate_limit_per_minute": 60
    },
    "dead_letters": {
      "webhook_id": "...",
      "count": 5,
      "events": [ /* up to 50 MemoryWebhookEventRecord objects */ ]
    },
    "audit": {
      "webhook_id": "...",
      "count": 30,
      "audit": [ /* up to 50 MemoryWebhookAuditRecord objects, newest-first */ ]
    },
    "focus": "investigating delivery failures"
  }
}
```

| Status | Condition |
|--------|-----------|
| 200 | Success |
| 404 | Webhook UUID not found |

### `GET /api/v1/evidence/governance/:user/export`

Export an evidence bundle for a user's governance posture, including their active policy, recent violations, and full audit trail.

Path params:
- `user` — User UUID or `external_id`

Query params:

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `focus` | string | (none) | Free-text annotation |
| `source_path` | string | `/_/governance/{user_uuid}` | Override envelope `source_path` |
| `violations_from` | RFC3339 datetime | `now - 24h` | Start of violations window |
| `violations_to` | RFC3339 datetime | `now` | End of violations window |
| `limit` | integer | `50` | Max audit/violation rows. Clamped to `[1, 200]`. |

```json
// Response 200
{
  "kind": "governance_evidence_bundle",
  "exported_at": "2026-03-10T12:00:00Z",
  "source_path": "/_/governance/019...",
  "payload": {
    "user": "alice",
    "policy": { /* GovernancePolicy object */ },
    "violations": [ /* GovernanceAuditRecord objects within window */ ],
    "audit": [ /* All GovernanceAuditRecord objects for user, newest-first */ ],
    "violations_window": {
      "from": "2026-03-09T12:00:00Z",
      "to": "2026-03-10T12:00:00Z"
    },
    "focus": null
  }
}
```

| Status | Condition |
|--------|-----------|
| 200 | Success |
| 400 | `violations_to` must be after `violations_from` |
| 404 | User not found |

### `GET /api/v1/evidence/traces/:request_id/export`

Export an evidence bundle for a cross-pipeline request trace, including matched episodes, webhook events, and audit records.

Path params:
- `request_id` — The `x-mnemo-request-id` value to trace

Query params:

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `from` | RFC3339 datetime | `now - 30d` | Start of time window |
| `to` | RFC3339 datetime | `now` | End of time window |
| `limit` | integer | `100` | Max matches per category. Clamped to `[1, 500]`. |
| `include_episodes` | bool | `true` | Include matched episodes |
| `include_webhook_events` | bool | `true` | Include matched webhook events |
| `include_webhook_audit` | bool | `true` | Include matched webhook audit records |
| `include_governance_audit` | bool | `true` | Include matched governance audit records |
| `user` | string | (none) | Optional user UUID/external_id filter |
| `focus` | string | (none) | Free-text annotation |
| `source_path` | string | `/_/traces/{request_id}` | Override envelope `source_path` |

```json
// Response 200
{
  "kind": "trace_evidence_bundle",
  "exported_at": "2026-03-10T12:00:00Z",
  "source_path": "/_/traces/req-abc-123",
  "payload": {
    "request_id": "req-abc-123",
    "focus": null,
    "trace": {
      "request_id": "req-abc-123",
      "matched_episodes": [ /* EpisodeMatch objects */ ],
      "matched_webhook_events": [ /* MemoryWebhookEventRecord objects */ ],
      "matched_webhook_audit": [ /* MemoryWebhookAuditRecord objects */ ],
      "matched_governance_audit": [ /* GovernanceAuditRecord objects */ ],
      "summary": {
        "episode_matches": 2,
        "webhook_event_matches": 1,
        "webhook_audit_matches": 0,
        "governance_audit_matches": 0,
        "filters": { "from": "...", "to": "...", "limit": 100 }
      }
    }
  }
}
```

| Status | Condition |
|--------|-----------|
| 200 | Success |
| 400 | `to` must be after `from`, or `request_id` is blank |

---

## LLM Span Tracing

LLM span endpoints expose per-request and per-user LLM call telemetry (provider, model, token counts, latency). Spans are persisted to Redis and also held in a 500-span in-memory ring buffer as fallback.

### `GET /api/v1/spans/request/:request_id`

List all LLM spans associated with a request correlation ID.

Path params:
- `request_id` — The `x-mnemo-request-id` value

```json
// Response 200
{
  "request_id": "req-abc-123",
  "spans": [
    {
      "id": "...",
      "request_id": "req-abc-123",
      "user_id": "...",
      "provider": "anthropic",
      "model": "claude-haiku-4-20250514",
      "operation": "extract",
      "prompt_tokens": 1200,
      "completion_tokens": 300,
      "total_tokens": 1500,
      "latency_ms": 850,
      "success": true,
      "error": null,
      "started_at": "2026-03-10T11:59:59Z",
      "finished_at": "2026-03-10T12:00:00Z"
    }
  ],
  "count": 1,
  "total_tokens": 1500,
  "total_latency_ms": 850
}
```

Always returns 200. If no spans match, `spans` is an empty array.

### `GET /api/v1/spans/user/:user_id`

List recent LLM spans for a user, most-recent first.

Path params:
- `user_id` — UUID of the user

Query params:

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | integer | `100` | Max spans to return. Clamped to `[1, 1000]`. |

```json
// Response 200
{
  "user_id": "019...",
  "spans": [ /* LlmSpan objects, most-recent first */ ],
  "count": 15,
  "total_tokens": 45000,
  "total_latency_ms": 12300
}
```

Always returns 200. If no spans match, `spans` is an empty array.

---

## Memory Digest

Memory digests are LLM-generated summaries of a user's knowledge graph — a compact overview of entities, relationships, and dominant topics. Digests are generated during sleep-time idle windows or on-demand via the refresh endpoint.

### `GET /api/v1/memory/:user/digest`

Retrieve the cached memory digest for a user.

Path params:
- `user` — User UUID or `external_id`

```json
// Response 200
{
  "user_id": "019...",
  "summary": "This person is a Rust developer interested in AI agents, knowledge graphs, and ...",
  "entity_count": 42,
  "edge_count": 87,
  "dominant_topics": ["rust", "knowledge_graphs", "ai_agents"],
  "generated_at": "2026-03-10T08:00:00Z",
  "model": "claude-haiku-4-20250514"
}
```

| Status | Condition |
|--------|-----------|
| 200 | Success |
| 404 | User not found, or no digest has been generated yet |

### `POST /api/v1/memory/:user/digest`

Force-regenerate the memory digest using the configured LLM. No request body required.

Path params:
- `user` — User UUID or `external_id`

```json
// Response 200
{
  "user_id": "019...",
  "summary": "This person is a Rust developer interested in AI agents...",
  "entity_count": 42,
  "edge_count": 87,
  "dominant_topics": ["rust", "knowledge_graphs", "ai_agents"],
  "generated_at": "2026-03-10T12:00:00Z",
  "model": "claude-haiku-4-20250514"
}
```

| Status | Condition |
|--------|-----------|
| 200 | Success |
| 400 | LLM provider not configured, or no entities exist for user |
| 404 | User not found |
| 502 | LLM provider error |

---

## MCP Server (Model Context Protocol)

Mnemo ships an MCP server binary (`mnemo-mcp-server`) that exposes memory tools
to any MCP-compatible client (Claude Code, Cursor, etc.) over the stdio transport.

### Configuration

| Env Var | Default | Description |
|---------|---------|-------------|
| `MNEMO_MCP_BASE_URL` | `http://localhost:3000` | URL of the running Mnemo HTTP server |
| `MNEMO_API_KEY` | *(none)* | API key for Mnemo auth (optional) |
| `MNEMO_MCP_DEFAULT_USER` | *(none)* | Default user identifier for tools that need one |
| `RUST_LOG` | `warn` | Log level (logs go to stderr) |

### Claude Code integration

```json
{
  "mcpServers": {
    "mnemo": {
      "command": "mnemo-mcp-server",
      "env": {
        "MNEMO_MCP_BASE_URL": "http://localhost:3000",
        "MNEMO_MCP_DEFAULT_USER": "your-user-id"
      }
    }
  }
}
```

### Tools (7)

| Tool | Description |
|------|-------------|
| `mnemo_remember` | Store a memory (text → extract → graph update) |
| `mnemo_recall` | Retrieve context for a query (hybrid retrieval) |
| `mnemo_graph_query` | Query knowledge graph (list entities/edges, communities) |
| `mnemo_agent_identity` | Get or update an agent identity profile |
| `mnemo_digest` | Get or generate a prose memory digest |
| `mnemo_coherence` | Get coherence report for a user's knowledge graph |
| `mnemo_health` | Health check on the Mnemo server |

### Resources (2 templates)

| URI Template | Description |
|-------------|-------------|
| `mnemo://users/{user}/memory` | User memory summary (coherence report) |
| `mnemo://agents/{agent_id}/identity` | Agent identity profile |

---

## Errors

All errors follow a consistent format:

```json
{
  "error": {
    "code": "user_not_found",
    "message": "User not found: 019513a4-7e2b-7000-8000-000000000099"
  }
}
```

| HTTP Status | Code | Meaning |
|-------------|------|---------|
| 400 | `validation_error` | Invalid request body or parameters |
| 401 | `unauthorized` | Missing or invalid authentication |
| 403 | `forbidden` | Insufficient permissions |
| 404 | `user_not_found` | User ID not found |
| 404 | `session_not_found` | Session ID not found |
| 404 | `episode_not_found` | Episode ID not found |
| 404 | `entity_not_found` | Entity ID not found |
| 404 | `edge_not_found` | Edge ID not found |
| 409 | `duplicate` | Resource already exists |
| 429 | `rate_limited` | LLM provider rate limited (includes `retry_after_ms`) |
| 500 | `internal_error` | Unexpected server error |
| 502 | Various | LLM/embedding provider error |

---

## Pagination

All list endpoints use cursor-based pagination:

```
GET /api/v1/users?limit=10&after=019513a4-7e2b-7000-8000-000000000005
```

- `limit`: Maximum items to return (default varies by endpoint)
- `after`: UUID of the last item from the previous page

Results are ordered newest-first (by `created_at`). To page forward, pass the `id` of the last item in the current page as `after`.

```json
{
  "data": [...],
  "count": 10
}
```

When `count < limit`, you've reached the last page.

---

## Operator Dashboard

The embedded operator dashboard is served at `/_/`. No deployment or configuration is needed — the static assets (HTML, CSS, JS) are compiled into the server binary via `rust-embed`.

| Route | Description |
|-------|-------------|
| `GET /_/` | Dashboard home page (SPA index) |
| `GET /_/static/*` | Embedded CSS, JS assets |
| `GET /_/webhooks` | Webhook operations page (SPA route) |
| `GET /_/rca` | Root cause analysis page (SPA route) |
| `GET /_/governance` | Governance policies page (SPA route) |
| `GET /_/traces` | Request traces page (SPA route) |
| `GET /_/explorer` | Knowledge graph explorer page (SPA route) |

All `/_/*` routes that don't match a static asset serve the SPA index; client-side JavaScript handles routing.

---

## Authentication (v0.6.0)

All API endpoints (except `/health`, `/healthz`, `/metrics`, and dashboard `/_/` routes) require authentication when auth is enabled.

### Headers

Provide your API key via either:

- `Authorization: Bearer <key>`
- `X-Api-Key: <key>`

When auth is disabled (`MNEMO_AUTH_ENABLED=false`), all callers receive implicit Admin context.

### Roles

| Role | Ordinal | Permissions |
|------|---------|-------------|
| `read` | 0 | Read-only access to memory, views, guardrails |
| `write` | 1 | Read + create/update episodes, entities, regions |
| `admin` | 2 | Full access including key management, view/guardrail CRUD, region deletion |

Higher roles inherit all lower-role permissions. A handler requiring `write` accepts both `write` and `admin` keys.

### Scoped Keys

API keys may include an optional `scope` restricting access to specific users, agents, or classification levels:

```json
{
  "allowed_user_ids": ["uuid-1", "uuid-2"],
  "allowed_agent_ids": ["bot-a", "bot-b"],
  "max_classification": "confidential"
}
```

---

## API Keys

Manage API keys for RBAC authentication. All key management endpoints require `admin` role.

### Create API Key

```
POST /api/v1/keys
```

**Auth**: `admin`

**Request Body**:

```json
{
  "name": "my-service-key",
  "role": "write",
  "scope": {
    "allowed_user_ids": ["550e8400-e29b-41d4-a716-446655440000"],
    "allowed_agent_ids": ["bot-a"],
    "max_classification": "confidential"
  },
  "expires_at": "2026-12-31T23:59:59Z"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Display name (1-128 chars, trimmed) |
| `role` | string | Yes | `"read"`, `"write"`, or `"admin"` |
| `scope` | object | No | Access restrictions (see Scoped Keys above) |
| `expires_at` | datetime | No | Automatic expiry timestamp |

**Response**: `201 Created`

```json
{
  "raw_key": "mnk_a1b2c3d4...",
  "id": "uuid",
  "name": "my-service-key",
  "key_hash": "sha256-hash",
  "key_prefix": "mnk_a1b2",
  "role": "write",
  "scope": { ... },
  "created_by": "admin-bootstrap",
  "created_at": "2026-03-13T...",
  "expires_at": "2026-12-31T...",
  "revoked": false
}
```

> **Important**: The `raw_key` is returned exactly once and cannot be retrieved again. Store it securely.

### List API Keys

```
GET /api/v1/keys?limit=50
```

**Auth**: `admin`

| Query Param | Type | Default | Description |
|-------------|------|---------|-------------|
| `limit` | integer | 50 | Max keys to return (1-200) |

**Response**: `200 OK`

```json
{
  "keys": [
    {
      "id": "uuid",
      "name": "my-service-key",
      "key_prefix": "mnk_a1b2",
      "role": "write",
      "scope": { ... },
      "created_by": "admin-bootstrap",
      "created_at": "2026-03-13T...",
      "last_used_at": null,
      "expires_at": "2026-12-31T...",
      "revoked": false
    }
  ]
}
```

> Note: `key_hash` is intentionally omitted from list responses.

### Revoke API Key

```
DELETE /api/v1/keys/:key_id
```

**Auth**: `admin`

**Response**: `204 No Content`

### Rotate API Key

```
POST /api/v1/keys/:key_id/rotate
```

**Auth**: `admin`

Revokes the old key and creates a new one with the same name, role, scope, and expiry. Returns the new key in the same format as Create.

**Response**: `201 Created` (same shape as Create API Key)

---

## Data Classification

Entities and edges carry a classification label that controls access:

| Level | Ordinal | Description |
|-------|---------|-------------|
| `public` | 0 | Unrestricted |
| `internal` | 1 | Default for all data |
| `confidential` | 2 | Sensitive business data |
| `restricted` | 3 | Highest sensitivity |

Classification is assigned at ingestion time and enforced during retrieval. API key scope can set a `max_classification` ceiling — facts above the ceiling are excluded from context assembly.

---

## Memory Views

Named, reusable access policies that filter memory by classification, entity types, edge labels, and temporal scope.

### Create View

```
POST /api/v1/views
```

**Auth**: `admin`

**Request Body**:

```json
{
  "name": "customer-facing",
  "description": "Read-only view for customer-facing agents",
  "max_classification": "internal",
  "allowed_entity_types": ["person", "organization"],
  "blocked_edge_labels": ["internal_note", "salary"],
  "max_facts": 100,
  "include_narrative": true,
  "temporal_scope": {
    "type": "last_n_days",
    "days": 90
  }
}
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | Yes | — | Unique name (1-128 chars) |
| `description` | string | Yes | — | Human-readable description |
| `max_classification` | string | Yes | — | Classification ceiling |
| `allowed_entity_types` | string[] | No | all types | Whitelist of entity types |
| `blocked_edge_labels` | string[] | No | none | Blacklist of edge labels |
| `max_facts` | integer | No | unlimited | Max facts in context |
| `include_narrative` | boolean | No | `true` | Include user narrative |
| `temporal_scope` | object | No | all time | Time-based filter |

**Temporal Scope** variants:

| Type | Fields | Description |
|------|--------|-------------|
| `last_n_days` | `days: integer` | Facts from the last N days |
| `since` | `since: datetime` | Facts since a specific timestamp |
| `current_only` | — | Only the most recent session |

**Response**: `201 Created` with the created `MemoryView`.

### List Views

```
GET /api/v1/views
```

**Auth**: `read`

**Response**: `200 OK`

```json
{
  "views": [ ... ]
}
```

### Get View

```
GET /api/v1/views/:name
```

**Auth**: `read`

Path parameter is the view **name** (not UUID).

**Response**: `200 OK` with `MemoryView`.

### Update View

```
PUT /api/v1/views/:name
```

**Auth**: `admin`

Full replacement of all mutable fields. Same request body as Create.

**Response**: `200 OK` with updated `MemoryView`.

### Delete View

```
DELETE /api/v1/views/:name
```

**Auth**: `admin`

**Response**: `204 No Content`

### Using Views in Context Requests

Apply a view by adding the `?view=` query parameter to context endpoints:

```
GET /api/v1/memory/:user/context?query=...&view=customer-facing
```

The view's constraints are applied during context assembly, filtering facts by classification ceiling, entity types, edge labels, temporal scope, and fact count limit.

---

## Memory Guardrails

Rule-based policy engine for memory access control. Guardrails evaluate conditions against memory operations and execute actions (block, redact, reclassify, audit, warn).

### Create Guardrail

```
POST /api/v1/guardrails
```

**Auth**: `admin`

**Request Body**:

```json
{
  "name": "block-restricted-for-readers",
  "description": "Prevent read-only keys from accessing restricted data",
  "trigger": "on_retrieval",
  "condition": {
    "type": "and",
    "conditions": [
      { "type": "classification_above", "classification": "confidential" },
      { "type": "caller_role_below", "role": "write" }
    ]
  },
  "action": {
    "type": "block",
    "reason": "Insufficient role for restricted data"
  },
  "priority": 10,
  "enabled": true,
  "scope": { "type": "global" }
}
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | Yes | — | Unique name (1-64 chars, alphanumeric/`_`/`-`) |
| `description` | string | Yes | — | Human-readable description |
| `trigger` | string | Yes | — | When to evaluate (see Triggers) |
| `condition` | object | Yes | — | Predicate tree (see Conditions) |
| `action` | object | Yes | — | What to do when matched (see Actions) |
| `priority` | integer | No | 0 | Evaluation order (lower = first) |
| `enabled` | boolean | No | `true` | Whether this rule is active |
| `scope` | object | No | `global` | Applicability scope |

**Triggers**: `on_ingest`, `on_fact_creation`, `on_retrieval`, `on_entity_creation`, `on_any`

**Conditions** (composable tree):

| Type | Fields | Description |
|------|--------|-------------|
| `classification_above` | `classification` | Matches if fact classification > threshold |
| `entity_type_in` | `entity_types: string[]` | Matches entity types |
| `edge_label_in` | `labels: string[]` | Matches edge labels |
| `content_matches_regex` | `pattern: string` | Regex match on content |
| `caller_role_below` | `role` | Matches if caller role < threshold |
| `fact_age_above_days` | `days: integer` | Matches facts older than N days |
| `confidence_below` | `confidence: float` | Matches low-confidence facts |
| `and` | `conditions: condition[]` | All must match |
| `or` | `conditions: condition[]` | Any must match |
| `not` | `condition: condition` | Invert match |

**Actions**:

| Type | Fields | Description |
|------|--------|-------------|
| `block` | `reason: string` | Reject the operation |
| `redact` | — | Remove content from output |
| `reclassify` | `classification` | Upgrade classification level |
| `audit_only` | `severity: string` | Allow but log |
| `warn` | `message: string` | Allow with warning |

**Scope**: `{ "type": "global" }` or `{ "type": "user", "user_id": "uuid" }`

**Response**: `201 Created` with `GuardrailRule`.

### List Guardrails

```
GET /api/v1/guardrails
```

**Auth**: `read`

**Response**: `200 OK` with `{ "guardrails": [...] }`

### Get Guardrail

```
GET /api/v1/guardrails/:id
```

**Auth**: `read`

**Response**: `200 OK` with `GuardrailRule`.

### Update Guardrail

```
PUT /api/v1/guardrails/:id
```

**Auth**: `admin`

Full replacement. Same request body as Create.

**Response**: `200 OK` with updated `GuardrailRule`.

### Delete Guardrail

```
DELETE /api/v1/guardrails/:id
```

**Auth**: `admin`

**Response**: `204 No Content`

### Evaluate Guardrails (Dry-Run)

```
POST /api/v1/guardrails/evaluate
```

**Auth**: `read`

Test which guardrail rules would fire for a given context without executing any actions.

**Request Body**:

```json
{
  "trigger": "on_retrieval",
  "classification": "restricted",
  "entity_type": "person",
  "edge_label": "salary",
  "content": "annual compensation is $150,000",
  "caller_role": "read",
  "fact_age_days": 30,
  "confidence": 0.6,
  "user_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

All fields except `trigger` are optional.

**Response**: `200 OK`

```json
{
  "blocked": true,
  "block_reason": "Insufficient role for restricted data",
  "redact": false,
  "reclassify_to": null,
  "warnings": [],
  "audit_entries": [],
  "rule_results": [
    {
      "rule_id": "uuid",
      "rule_name": "block-restricted-for-readers",
      "matched": true,
      "action": { "type": "block", "reason": "..." }
    }
  ]
}
```

---

## Agent Promotions & Governance

Promotion proposals enable controlled, auditable changes to agent identity with approval workflows, conflict analysis, and configurable policies.

### Create Promotion Proposal

```
POST /api/v1/agents/:agent_id/promotions
```

**Request Body**:

```json
{
  "proposal": "Increase formality based on enterprise feedback",
  "candidate_core": {
    "tone": "formal",
    "domain_expertise": ["enterprise", "compliance"]
  },
  "reason": "Customer satisfaction improved 15% with formal tone in pilot",
  "risk_level": "medium",
  "source_event_ids": ["uuid-1", "uuid-2", "uuid-3"]
}
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `proposal` | string | Yes | — | Description of proposed change |
| `candidate_core` | object | Yes | `{}` | Proposed identity core values |
| `reason` | string | Yes | — | Justification for the change |
| `risk_level` | string | No | `"medium"` | `"low"`, `"medium"`, or `"high"` |
| `source_event_ids` | uuid[] | Yes | — | Supporting evidence (minimum 3 events) |

**Response**: `201 Created` with `PromotionProposal`.

### List Promotion Proposals

```
GET /api/v1/agents/:agent_id/promotions?limit=50
```

| Query Param | Type | Default | Description |
|-------------|------|---------|-------------|
| `limit` | integer | 50 | Max proposals to return (1-500) |

**Response**: `200 OK` with `[PromotionProposal, ...]`

### Approve Promotion Proposal

```
POST /api/v1/agents/:agent_id/promotions/:proposal_id/approve
```

No request body required. The caller's identity is recorded as the approver.

**Behavior**:
1. Checks proposal is `pending` (rejects if expired, already approved/rejected)
2. Validates cooling period from approval policy has elapsed
3. Records approver (deduplicated by key name)
4. Checks quorum requirement from approval policy
5. If quorum not met: saves partial approval, returns proposal still as `pending`
6. If quorum met: applies `candidate_core` to live agent identity, sets status to `approved`

**Response**: `200 OK` with `PromotionProposal` (check `status` field).

### Reject Promotion Proposal

```
POST /api/v1/agents/:agent_id/promotions/:proposal_id/reject
```

**Request Body** (optional):

```json
{
  "reason": "Insufficient evidence for this personality change"
}
```

**Response**: `200 OK` with `PromotionProposal` (status = `rejected`).

### Get Promotion Conflicts

```
GET /api/v1/agents/:agent_id/promotions/:proposal_id/conflicts
```

Analyzes the agent's experience events for evidence supporting or conflicting with the proposal.

**Response**: `200 OK`

```json
{
  "proposal_id": "uuid",
  "agent_id": "bot-a",
  "supporting_signals": ["event-uuid-1", "event-uuid-2"],
  "conflicting_signals": ["event-uuid-3"],
  "conflict_score": 0.35,
  "recommendation": "review_conflicts"
}
```

| `conflict_score` | `recommendation` |
|-------------------|------------------|
| < 0.3 | `proceed` |
| 0.3 – 0.7 | `review_conflicts` |
| >= 0.7 | `reject` |

### Set Approval Policy

```
PUT /api/v1/agents/:agent_id/approval-policy
```

**Auth**: `admin`

**Request Body**:

```json
{
  "low_risk": {
    "min_approvers": 1,
    "cooling_period_hours": null,
    "auto_reject_after_hours": 168
  },
  "medium_risk": {
    "min_approvers": 2,
    "cooling_period_hours": 24,
    "auto_reject_after_hours": 72
  },
  "high_risk": {
    "min_approvers": 3,
    "cooling_period_hours": 48,
    "auto_reject_after_hours": 48
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `min_approvers` | integer | Minimum approvals needed for quorum |
| `cooling_period_hours` | integer? | Hours to wait after creation before approvals count |
| `auto_reject_after_hours` | integer? | Auto-reject pending proposals after this many hours |

**Response**: `200 OK` with `ApprovalPolicy`.

### Get Approval Policy

```
GET /api/v1/agents/:agent_id/approval-policy
```

Returns the agent's approval policy, or a default policy (1 approver for all risk levels) if none is configured.

**Response**: `200 OK` with `ApprovalPolicy`.

---

## Memory Regions & ACLs

Shared memory regions with granular access control for multi-agent architectures.

### Create Region

```
POST /api/v1/regions
```

**Auth**: `write`

**Request Body**:

```json
{
  "name": "shared-customer-context",
  "owner_agent_id": "sales-bot",
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "entity_filter": {
    "entity_types": ["person", "organization"],
    "name_patterns": ["acme"]
  },
  "edge_filter": {
    "labels": ["works_at", "purchased"],
    "min_confidence": 0.7
  },
  "classification_ceiling": "confidential"
}
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | Yes | — | Region name (1-128 chars, no control chars) |
| `owner_agent_id` | string | Yes | — | Owning agent (1-128 chars, validated format) |
| `user_id` | uuid | Yes | — | User this region belongs to (must exist) |
| `entity_filter` | object | No | all entities | Filter by entity types and name patterns |
| `edge_filter` | object | No | all edges | Filter by labels and min confidence |
| `classification_ceiling` | string | No | `"internal"` | Max classification level in this region |

**Agent ID validation**: Must be 1-128 characters, alphanumeric plus `-`, `_`, `.`, and space. Rejects `:`, `/`, `..`, control characters, null bytes, and non-ASCII unicode.

**Response**: `201 Created` with `MemoryRegion`.

### List Regions

```
GET /api/v1/regions?user_id=uuid&agent_id=bot-a
```

**Auth**: `read`

| Query Param | Type | Description |
|-------------|------|-------------|
| `user_id` | uuid | Filter regions by user |
| `agent_id` | string | Filter regions by owner or ACL-granted agent |

Both filters are optional and can be combined.

**Response**: `200 OK` with `[MemoryRegion, ...]`

### Get Region

```
GET /api/v1/regions/:region_id
```

**Auth**: `read`

**Response**: `200 OK` with `MemoryRegion`.

### Update Region

```
PUT /api/v1/regions/:region_id
```

**Auth**: `write`

**Ownership**: Only the region owner or an `admin` key may update. Returns `403 Forbidden` otherwise.

**Request Body** (partial update — only provided fields are changed):

```json
{
  "name": "renamed-region",
  "classification_ceiling": "restricted",
  "entity_filter": {
    "entity_types": ["person"]
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | No | New name (1-128 chars) |
| `entity_filter` | object | No | Replace entity filter |
| `edge_filter` | object | No | Replace edge filter |
| `classification_ceiling` | string | No | New classification ceiling |

**Response**: `200 OK` with updated `MemoryRegion`.

### Delete Region

```
DELETE /api/v1/regions/:region_id
```

**Auth**: `admin`

Deletes the region and all associated ACL entries. Cleans all reverse indices atomically.

**Response**: `204 No Content`

### Grant Region Access

```
POST /api/v1/regions/:region_id/acl
```

**Auth**: `write`

**Ownership**: Only the region owner or an `admin` key may grant access. Returns `403 Forbidden` otherwise.

**Request Body**:

```json
{
  "agent_id": "support-bot",
  "permission": "read",
  "expires_at": "2026-06-01T00:00:00Z"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agent_id` | string | Yes | Agent to grant access to (validated format) |
| `permission` | string | Yes | `"read"`, `"write"`, or `"manage"` |
| `expires_at` | datetime | No | Auto-expiry timestamp |

**Permission hierarchy**: `read` (0) < `write` (1) < `manage` (2). Higher permissions include all lower ones.

**Response**: `201 Created` with `MemoryRegionAcl`.

```json
{
  "region_id": "uuid",
  "agent_id": "support-bot",
  "permission": "read",
  "granted_by": "admin-key",
  "granted_at": "2026-03-13T...",
  "expires_at": "2026-06-01T..."
}
```

### List Region ACLs

```
GET /api/v1/regions/:region_id/acl
```

**Auth**: `read`

Returns only active (non-expired) ACL entries. Expired entries are filtered out.

**Response**: `200 OK` with `[MemoryRegionAcl, ...]`

### Revoke Region Access

```
DELETE /api/v1/regions/:region_id/acl/:agent_id
```

**Auth**: `write`

**Ownership**: Only the region owner or an `admin` key may revoke access. Returns `403 Forbidden` otherwise.

**Response**: `204 No Content`

---

## TinyLoRA — Homeoadaptive Embedding Personalization

TinyLoRA adds per-`(user, agent)` learned linear transformations on top of the shared
embedding space. Each adapter is a rank-8 LoRA residual that rotates base embeddings
toward the agent's observed relevance history. Enable with `MNEMO_LORA_ENABLED=true`.

**Homeoadaptive** *(adj.)* — of an embedding system: self-regulating toward a stable
operating point for each `(user, agent)` pair through continuous implicit and explicit
feedback, such that successive retrievals converge on each agent's equilibrium
representation of relevance without external configuration. The B matrix begins at zero
and is nudged toward a bounded fixed point by each access or rating; the Frobenius clamp
(`||B||_F ≤ 10`) prevents runaway drift while the small learning rate (`lr=0.005`) allows
gradual evolution as user interests shift.

**Adaptation formula:**
```
v_adapted = v_base + 0.125 * B · (A · v_base)
A ∈ ℝ^{8×384}  (fixed random projection, unique per user+agent pair)
B ∈ ℝ^{384×8}  (zero-init; updated implicitly from retrieval access patterns)
```

Adapters are persisted in Redis and cached in-memory. When LoRA is disabled,
`embed_for_agent` is a zero-overhead pass-through to the base embedder.

---

### Get LoRA Adapter Stats

```
GET /api/v1/agents/:agent_id/lora/stats?user=<identifier>&include_global=true|false
```

**Auth**: `read`

Returns statistics for the agent-specific adapter. Optionally includes the user-level
(agentless) global adapter when `include_global=true`.

**Query Parameters**:

| Parameter | Type | Description |
|-----------|------|-------------|
| `user` | string | **Required.** User identifier (email or UUID). |
| `include_global` | bool | Also return the user-level global adapter stats. |

**Response** `200 OK`:
```json
[
  {
    "user_id": "uuid",
    "agent_id": "my-agent",
    "update_count": 42,
    "last_updated": 1700000000,
    "dims": 384,
    "rank": 8,
    "scale": 0.125,
    "b_frobenius_norm": 2.34
  }
]
```

`b_frobenius_norm` is a proxy for adapter divergence from identity — values close to 0
indicate the adapter is near-identity (few updates). Values approaching 10.0 indicate
the Frobenius clamp has been reached.

**Errors**: `404 Not Found` when no adapter exists for the given user/agent pair.

---

### Reset LoRA Adapter

```
DELETE /api/v1/agents/:agent_id/lora?user=<identifier>&scope=agent|global|all
```

**Auth**: `write`

Deletes the LoRA adapter weights from Redis and evicts the in-memory cache for the
given scope. After a reset, the next retrieval request will use a fresh identity adapter.

**Query Parameters**:

| Parameter | Type | Description |
|-----------|------|-------------|
| `user` | string | **Required.** User identifier. |
| `scope` | string | `agent` (default) — agent-specific adapter only. `global` — user-level adapter. `all` — both. |

**Response**: `204 No Content`

---

### Submit Explicit Relevance Feedback

```
POST /api/v1/agents/:agent_id/feedback
```

**Auth**: `write`

Submit explicit relevance ratings for retrieved items, triggering a **homeoadaptive**
adapter update. Each rating nudges the `(user, agent)` LoRA adapter toward highly-rated
items and away from irrelevant ones — providing stronger signal than implicit access alone.

The server resolves each item UUID to its stored edge fact text, re-embeds it, then
applies a signed gradient update. Items that cannot be resolved (unknown ID, wrong user)
are silently skipped.

When `MNEMO_LORA_ENABLED=false`, this endpoint accepts the request and returns
`items_updated: 0` without error — callers need not gate on the feature flag.

**Request body**:
```json
{
  "user": "alice@example.com",
  "query_text": "What are our Q3 revenue projections?",
  "ratings": {
    "edge-uuid-1": 1.0,
    "edge-uuid-2": -0.5,
    "edge-uuid-3": 0.0
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `user` | string | **Required.** User identifier (email or UUID). |
| `query_text` | string | **Required.** The query text used for retrieval. |
| `ratings` | object | **Required.** Edge/entity/episode UUIDs → rating in `[-1.0, 1.0]`. `1.0` = highly relevant, `-1.0` = irrelevant, `0.0` = neutral (skipped). At least one non-zero rating required. |

**Response** `200 OK`:
```json
{
  "items_updated": 2,
  "total_update_count": 47,
  "b_frobenius_norm": 3.12
}
```

| Field | Description |
|-------|-------------|
| `items_updated` | Number of items for which a weight update was applied. |
| `total_update_count` | Cumulative update steps on this adapter (implicit + explicit). |
| `b_frobenius_norm` | Frobenius norm of B after this batch. Approaches 10.0 as the adapter saturates. |

---

### List Users with LoRA Adapters (Agent View)

```
GET /api/v1/agents/:agent_id/lora/users
```

**Auth**: `read`

Returns all users that have a trained LoRA adapter with this agent. Ordered by
`last_updated` descending — most recently active users first.

This is the **agent-view** of the homeoadaptive index: an operator or agent system
can use it to inspect which users have converged adapters, compare `b_frobenius_norm`
distributions, or identify users whose adapters are stale.

Returns an empty array when no adapters exist for this agent yet — never 404.

**Response** `200 OK`:
```json
[
  {
    "user_id": "uuid-of-user-a",
    "agent_id": "my-agent",
    "update_count": 87,
    "last_updated": 1700050000,
    "dims": 384,
    "rank": 8,
    "scale": 0.125,
    "b_frobenius_norm": 6.71
  },
  {
    "user_id": "uuid-of-user-b",
    "agent_id": "my-agent",
    "update_count": 12,
    "last_updated": 1700000000,
    "dims": 384,
    "rank": 8,
    "scale": 0.125,
    "b_frobenius_norm": 1.03
  }
]
```

**No `user` parameter required.** This endpoint enumerates users via the
`lora_agent_idx:{agent_id}` reverse index maintained in Redis (see Spec 07).
