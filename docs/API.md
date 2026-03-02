# API Reference

Base URL: `http://localhost:8080`

All request and response bodies are JSON. IDs are UUIDv7 strings.

---

## Health

### `GET /health`

Returns server status and version.

```json
// Response 200
{
  "status": "ok",
  "version": "0.1.0"
}
```

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
  "min_relevance": 0.3
}
```

`session`, `max_tokens`, and `min_relevance` are optional.

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
  "sources": ["semantic_search", "full_text_search"]
}
```

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
