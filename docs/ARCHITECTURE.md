# Architecture

This document explains how Mnemo works internally: the data model, temporal reasoning, ingestion pipeline, retrieval strategy, and deployment topology.

---

## System Overview

```
Your AI Agent
     │
     │  POST /api/v1/sessions/:id/episodes   (ingest)
     │  POST /api/v1/users/:id/context        (retrieve)
     │
     ▼
┌────────────────────────────────────────────┐
│              Mnemo Server                   │
│                                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │  REST API │  │ Ingest   │  │ Retrieval│ │
│  │  (Axum)  │  │ Worker   │  │ Engine   │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘ │
│       │              │              │       │
│  ┌────▼──────────────▼──────────────▼────┐ │
│  │          Core Domain Layer             │ │
│  │  (Models, Traits, Error Handling)      │ │
│  └────┬─────────────────────────────┬────┘ │
│       │                             │       │
│  ┌────▼─────┐              ┌───────▼─────┐ │
│  │  Redis   │              │   Qdrant    │ │
│  │  (State) │              │  (Vectors)  │ │
│  └──────────┘              └─────────────┘ │
└────────────────────────────────────────────┘
```

Mnemo is a single Rust binary. It connects to two external services: Redis for structured state and Qdrant for vector embeddings. There is no Neo4j, no JVM, no garbage collector.

---

## Data Model

### Core Types

**User** → owns everything. All data is isolated per-user (multi-tenant).

**Session** → a conversation thread. Contains an ordered sequence of episodes.

**Episode** → the atomic unit of data ingestion. Three types:
- `message`: A chat message with role (user/assistant/system/tool) and optional speaker name
- `json`: Structured event data (CRM events, app telemetry, etc.)
- `text`: Unstructured text (documents, meeting notes, transcripts)

**Entity** → a node in the knowledge graph. Represents a person, organization, product, location, event, concept, or custom type. Entities are automatically extracted from episodes and deduplicated across the user's history.

**Edge** → a temporal fact connecting two entities. This is where Mnemo's temporal reasoning lives.

### Relationships

```
User 1──* Session 1──* Episode
User 1──* Entity
User 1──* Edge

Episode *──* Entity  (via entity_ids / storage layer)
Edge    ──1 Entity   (source)
Edge    ──1 Entity   (target)
Edge    ──1 Episode  (source_episode_id)
```

---

## Temporal Model

Mnemo's core differentiator is its bi-temporal knowledge graph. Every edge (fact) tracks two time dimensions:

### Event Time (`valid_at`)
When the fact became true in the real world. This comes from the episode's timestamp or from temporal cues extracted by the LLM.

### Ingestion Time (`ingested_at`)
When Mnemo learned about this fact. Useful for debugging and auditing.

### Fact Lifecycle

Consider this conversation:

```
Aug 2024: "I love my Adidas running shoes!"
Feb 2025: "My Adidas fell apart. I switched to Nike."
```

Mnemo's graph evolves:

**After message 1:**
```
[Kendra] ──prefers──▶ [Adidas shoes]
         valid_at: 2024-08-10
         invalid_at: null         ← currently valid
```

**After message 2:**
```
[Kendra] ──prefers──▶ [Adidas shoes]
         valid_at: 2024-08-10
         invalid_at: 2025-02-28   ← invalidated

[Kendra] ──prefers──▶ [Nike shoes]
         valid_at: 2025-02-28
         invalid_at: null         ← currently valid
```

### Point-in-Time Queries

Because invalidated edges are preserved (not deleted), you can query the graph at any point in time:

```json
POST /api/v1/users/:id/context
{
  "temporal_filter": "2024-10-01T00:00:00Z",
  "messages": [{"role": "user", "content": "What shoes does Kendra like?"}]
}
```

This returns "Kendra prefers Adidas shoes" because that's what was true in October 2024.

### Contradiction Detection

When the ingestion pipeline processes a new episode, it:
1. Extracts relationships from the text
2. Checks for existing edges with the same source entity, target entity, and label
3. If a conflict is found, marks the old edge as invalidated (`invalid_at` = now)
4. Creates the new edge

This happens automatically. You don't need to tell Mnemo that facts have changed.

Related design notes:

- `docs/THREAD_HEAD.md` (explicit Git-like session HEAD model)
- `docs/TEMPORAL_VECTORIZATION.md` (time-aware retrieval scoring)

---

## Ingestion Pipeline

Episodes flow through a background worker that runs continuously:

```
Episode arrives via API
        │
        ▼
   ┌─ Stored in Redis (sync, <5ms) ──┐
   │   processing_status: "pending"    │
   │   Added to pending queue          │
   └───────────────────────────────────┘
        │
        ▼  (async, background worker)
   ┌─ Claimed atomically ─────────────┐
   │   ZREM from pending set           │
   │   processing_status: "processing" │
   └───────────────────────────────────┘
        │
        ▼
   ┌─ LLM Extraction ────────────────┐
   │   Prompt includes:               │
   │   - Episode content              │
   │   - Existing entity names (dedup)│
   │   Returns:                       │
   │   - Extracted entities           │
   │   - Extracted relationships      │
   └───────────────────────────────────┘
        │
        ▼
   ┌─ Entity Resolution ─────────────┐
   │   For each extracted entity:     │
   │   - Search by name in user graph │
   │   - If found: increment mention  │
   │   - If new: create + embed       │
   └───────────────────────────────────┘
        │
        ▼
   ┌─ Edge Construction ──────────────┐
   │   For each relationship:         │
   │   - Resolve entity names to IDs  │
   │   - Check for conflicts          │
   │   - Invalidate stale edges       │
   │   - Create new edge + embed      │
   └───────────────────────────────────┘
        │
        ▼
   ┌─ Embed Episode ──────────────────┐
   │   Store episode embedding in     │
   │   Qdrant for semantic search     │
   └───────────────────────────────────┘
        │
        ▼
   processing_status: "completed"
   entity_ids: [...]
   edge_ids: [...]
```

### Concurrency

The worker uses atomic `ZREM` on the Redis pending set to claim episodes. This means you can run multiple Mnemo instances and they won't double-process episodes. The first instance to `ZREM` a given episode ID wins.

### Failure Handling

If extraction fails (LLM timeout, parse error, etc.), the episode is marked `failed` with the error message stored in `processing_error`. Failed episodes can be retried by resetting their status to `pending`.

---

## Retrieval Strategy

When your agent calls `POST /api/v1/memory/:user/context` (or the lower-level `POST /api/v1/users/:id/context`), Mnemo runs a seven-step hybrid retrieval pipeline:

### 1. Query Embedding

The query string is embedded via the configured embedding provider. When Ollama is the provider, embedding requests go to the native `/api/embed` endpoint with `keep_alive: -1` so the model remains pinned in GPU/CPU memory indefinitely.

### 2. Metadata Prefilter Planner

Before running the vector search, Mnemo scans up to `MNEMO_METADATA_SCAN_LIMIT` (default: 400) candidate document IDs from Redis sorted sets using the user/session scope. This produces an Qdrant payload filter (`user_id`, `session_id`) that restricts the ANN search to the user's own data without a full-collection scan. If the candidate set is empty and `MNEMO_METADATA_RELAX_IF_EMPTY=true`, the filter is relaxed to prevent empty results.

### 3. Semantic Search (Qdrant)

Three parallel ANN searches run against Qdrant collections, constrained by the metadata prefilter:
- **Entity search**: Find entities whose names/summaries are semantically similar to the query
- **Edge search**: Find facts whose descriptions match the query
- **Episode search**: Find relevant conversation history

Each search is filtered by `user_id` for tenant isolation and by `min_relevance` threshold. All three collections have indexed payload fields (`user_id`, `session_id`, `processing_status`, `created_at`) for O(1) filtering.

### 4. Graph Traversal

For the top 3 matched entities, Mnemo traverses their outgoing edges to find connected facts. Graph-traversed results receive a 0.8x relevance discount relative to their seed entity's score.

### 5. Temporal Filtering

If `temporal_filter` is set, edges are filtered by validity at that point in time using the bi-temporal `valid_at` / `invalid_at` fields. Otherwise, only currently valid edges are included.

### 6. Reranking (RRF or MMR)

Results from the parallel searches are merged and reranked using one of two strategies, configured via `reranker` in `config/default.toml` (or `[retrieval]` section):

- **`rrf` (Reciprocal Rank Fusion)** — Default. Boosts candidates that appear in multiple ranked lists (entity + edge + episode). Effective for most workloads where relevance diversity is less important than consensus.
- **`mmr` (Maximal Marginal Relevance)** — Penalises near-duplicate results. Useful when queries tend to surface many semantically similar facts that would otherwise crowd the context window.

### 7. Context Assembly

Reranked results are assembled into a token-budgeted string with three sections:

1. **Known entities** — Name, type, and summary
2. **Current facts** — Natural language descriptions of relationships
3. **Relevant conversation history** — Episode previews with timestamps

Section headers are counted against the `max_tokens` budget. Empty sections are never included.

---

## Storage Architecture

### Redis (State)

All structured data lives in Redis using a consistent key schema:

| Key Pattern | Value | Purpose |
|---|---|---|
| `mnemo:user:{id}` | JSON | User data |
| `mnemo:user_ext:{external_id}` | UUID string | External ID → user ID index |
| `mnemo:users` | Sorted Set | All users, scored by timestamp |
| `mnemo:session:{id}` | JSON | Session data |
| `mnemo:user_sessions:{user_id}` | Sorted Set | User's sessions |
| `mnemo:episode:{id}` | JSON | Episode data |
| `mnemo:session_episodes:{session_id}` | Sorted Set | Session's episodes |
| `mnemo:pending_episodes` | Sorted Set | Episodes awaiting processing |
| `mnemo:entity:{id}` | JSON | Entity data |
| `mnemo:user_entities:{user_id}` | Sorted Set | User's entities |
| `mnemo:entity_name:{user_id}:{lowercase_name}` | UUID string | Name → entity ID index |
| `mnemo:edge:{id}` | JSON | Edge data |
| `mnemo:adj_out:{entity_id}` | Sorted Set | Outgoing adjacency list |
| `mnemo:adj_in:{entity_id}` | Sorted Set | Incoming adjacency list |
| `mnemo:user_edges:{user_id}` | Sorted Set | User's edges |
| `mnemo:rid_episodes:{request_id}` | Sorted Set | O(1) request-ID → episode trace index |
| `mnemo:webhooks:{user_id}` | JSON (Hash) | Webhook subscriptions per user |
| `mnemo:webhook_events:{webhook_id}` | JSON (Hash) | Delivery event rows per webhook |
| `mnemo:webhook_audit:{webhook_id}` | JSON (Hash) | Webhook audit records |
| `mnemo:governance_audit:{user_id}` | JSON (Hash) | Governance policy change audit log |
| `mnemo:user_policies:{user_id}` | JSON | Per-user governance policy |
| `mnemo:import_jobs:{job_id}` | JSON | Async chat-history import job status |
| `mnemo:agent_identity:{agent_id}` | JSON | Agent identity profile (current version) |
| `mnemo:agent_identity_versions:{agent_id}` | Sorted Set | Agent identity version history |
| `mnemo:agent_experiences:{agent_id}` | Sorted Set | Agent experience event log |
| `mnemo:agent_promotions:{agent_id}` | Sorted Set | Promotion proposals per agent |

Sorted sets are scored by timestamp for time-ordered pagination. Adjacency lists are scored by `valid_at` for temporal ordering.

### Qdrant (Vectors)

Three collections, each with `user_id` in the payload for tenant filtering:

| Collection | Content | Payload Fields |
|---|---|---|
| `mnemo_entities` | Entity name + type + summary embeddings | `user_id`, `name`, `entity_type` |
| `mnemo_edges` | Fact description embeddings | `user_id`, `label`, `fact` |
| `mnemo_episodes` | Episode content embeddings | `user_id`, `session_id`, `processing_status`, `created_at` |

All collections use cosine distance. Dimensions match the configured embedding model (default: 1536 for `text-embedding-3-small`).

All payload fields used in filters have dedicated Qdrant payload indexes (created at startup via `CreateFieldIndexCollectionBuilder`). This ensures ANN searches with `user_id` filters are O(1) per-collection rather than full-collection scans.

---

## Agent Identity Substrate

Alongside per-user memory (episodes, entities, edges), Mnemo maintains a parallel **agent-level identity layer** for the AI agent itself — distinct from any individual user's data.

### Data Model

| Object | Description |
|--------|-------------|
| `AgentIdentityProfile` | Versioned JSON `core` blob representing the agent's learned identity. Mutated by approved promotions or explicit PUT. |
| `ExperienceEvent` | A signal recorded from an interaction — category, signal text, confidence, weight, and a time-decay half-life (days). |
| `PromotionProposal` | A candidate identity update proposed from experience signals. Requires manual approval (`POST …/approve`) before it is applied to the profile. |
| `AgentIdentityAuditEvent` | Append-only audit record of every identity `created`, `updated`, or `rolled_back` event. |

### Key properties

- **Versioned**: every write increments `version`; all prior versions are retained in a Redis sorted set for rollback.
- **Approval gating**: `PromotionProposal` moves through `pending → approved | rejected`; only approved promotions update the live profile.
- **Risk levels**: proposals carry a `risk_level` field (`low | medium | high`) for human review workflows.
- **Audit trail**: all mutations are appended to an identity audit log, queryable via `GET /api/v1/agents/:agent_id/identity/audit`.

### Webhook Delivery Architecture

Mnemo supports outbound webhooks for real-time notification of memory events. The delivery system is built into the server process (no separate worker required):

- **Subscriptions** — stored per user in Redis (`mnemo:webhooks:{user_id}`).
- **Event log** — each delivery attempt is recorded as a `WebhookEvent` row.
- **Retry / dead-letter** — up to `MNEMO_WEBHOOKS_MAX_ATTEMPTS` retries with exponential backoff (`MNEMO_WEBHOOKS_BASE_BACKOFF_MS`). Events that exhaust retries are dead-lettered.
- **Rate limiting** — per-webhook rate limiter (`MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE`).
- **Circuit breaker** — after `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD` consecutive failures the circuit opens; delivery resumes after `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS`.
- **Persistence** — subscriptions and event rows survive server restarts (stored in Redis, loaded at startup via `restore_webhook_state`).
- **Audit** — all delivery decisions are appended to `mnemo:webhook_audit:{webhook_id}`.

---

## LLM Provider Configuration

Mnemo separates the **extraction LLM** (entity/relationship extraction from episodes) from the **embedding model** (vector generation for semantic search). Each is configured independently.

### Supported Providers

| Role | Env Var | Supported Values |
|------|---------|-----------------|
| LLM provider | `MNEMO_LLM_PROVIDER` | `anthropic`, `openai`, `ollama`, `liquid` |
| LLM API key | `MNEMO_LLM_API_KEY` | provider key |
| LLM model | `MNEMO_LLM_MODEL` | e.g. `claude-haiku-4-5` |
| Embedding base URL | `MNEMO_EMBEDDING_BASE_URL` | OpenAI-compatible endpoint |
| Embedding model | `MNEMO_EMBEDDING_MODEL` | e.g. `nomic-embed-text`, `text-embedding-3-small` |
| Embedding dimensions | `MNEMO_EMBEDDING_DIMENSIONS` | integer matching the model |

### Inference Policy

**Online (API-backed):** Use Anthropic with the project API key. Preferred extraction model: `claude-haiku-4-5` (fast, cheap) or `claude-sonnet-4-6` (higher quality).

**Local / offline:** Use **Liquid AI LFM2-24B-A2B** exclusively via Ollama (`MNEMO_LLM_PROVIDER=ollama`, `MNEMO_LLM_MODEL=hf.co/LiquidAI/LFM2-24B-A2B-GGUF`). Do not use other local models for extraction — LFM2-24B is the only validated local model for this workload.

**Embeddings:** Use `nomic-embed-text` via Ollama for local/offline. Set `MNEMO_EMBEDDING_DIMENSIONS=768`. For production with API access, `text-embedding-3-small` (1536 dims) via OpenAI is the default.

### Example: Local-only stack

```bash
MNEMO_AUTH_ENABLED=false \
MNEMO_LLM_PROVIDER=ollama \
MNEMO_LLM_BASE_URL=http://localhost:11434/v1 \
MNEMO_LLM_MODEL=hf.co/LiquidAI/LFM2-24B-A2B-GGUF \
MNEMO_EMBEDDING_BASE_URL=http://localhost:11434/v1 \
MNEMO_EMBEDDING_MODEL=nomic-embed-text \
MNEMO_EMBEDDING_DIMENSIONS=768 \
cargo run -p mnemo-server
```

### Example: Anthropic LLM + local embeddings

```bash
MNEMO_AUTH_ENABLED=false \
MNEMO_LLM_PROVIDER=anthropic \
MNEMO_LLM_API_KEY=<key> \
MNEMO_LLM_MODEL=claude-haiku-4-5 \
MNEMO_EMBEDDING_BASE_URL=http://localhost:11434/v1 \
MNEMO_EMBEDDING_MODEL=nomic-embed-text \
MNEMO_EMBEDDING_DIMENSIONS=768 \
cargo run -p mnemo-server
```

---

## Deployment

### Development

```bash
docker compose up -d    # Redis + Qdrant
cargo run --bin mnemo-server
```

### Production (Docker)

```bash
docker compose up -d    # Includes mnemo-server container
```

The Dockerfile uses a multi-stage build: Rust compilation in a builder stage, then a minimal Debian slim image. The release binary is compiled with LTO and stripped, targeting <50MB.

### Production (Kubernetes)

A Helm chart is not yet published. The architecture is fully stateless (all state in Redis/Qdrant), so horizontal scaling is straightforward: run multiple Mnemo replicas behind a load balancer. Each replica connects to the same Redis and Qdrant instances; no sticky sessions or shared local storage are required.

**Scaling considerations:**
- Mnemo replicas: Stateless, scale horizontally
- Redis: Use Redis Cluster for >100K users
- Qdrant: Use Qdrant's distributed mode for >10M vectors
- Ingestion throughput: Scales with replica count (atomic episode claiming)
