# API Reference

Base URL: `http://localhost:8080`

All request and response bodies are JSON. IDs are UUIDv7 strings.

All responses include `x-mnemo-request-id`. Clients may also provide `x-mnemo-request-id` and Mnemo will propagate it.

---

## Health

### `GET /health`

Returns server status and version. Also available at `/healthz` for Kubernetes-style liveness probes.

```json
// Response 200
{
  "status": "ok",
  "version": "0.3.4"
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

- `mode`: `head | hybrid | historical`
- `contract`: `default | support_safe | current_strict | historical_strict`
- `retrieval_policy`: `balanced | precision | recall | stability`
- `time_intent`: `auto | current | recent | historical`
- `as_of`: point-in-time target for historical recall
- `temporal_weight`: override temporal influence (0.0–1.0)
- `filters`: optional metadata prefilter (`roles`, `tags_any`, `tags_all`, `created_after`, `created_before`, `processing_status`)

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

### `POST /api/v1/agents/:agent_id/identity/rollback`

Rollback identity core to a prior version while preserving an append-only version history.

```json
{
  "target_version": 2,
  "reason": "revert unsafe identity mutation"
}
```

### `POST /api/v1/agents/:agent_id/experience`

Add an adaptive experience event.

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

### `GET /api/v1/entities/:id/subgraph?depth=2&max_nodes=50`

Traverse the knowledge graph from a seed entity using BFS.

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
