# Architecture

This document explains how Mnemo works internally: the data model, temporal reasoning, ingestion pipeline, retrieval strategy, and deployment topology.

---

## System Overview

```
Your AI Agent
     │
     │  POST /api/v1/sessions/:id/episodes   (ingest)
     │  POST /api/v1/users/:id/context        (retrieve)
     │  MCP tools: mnemo_remember / mnemo_recall
     │
     ▼
┌──────────────────────────────────────────────────────┐
│                    Mnemo Server                       │
│                                                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │  REST API │  │ Ingest   │  │ Retrieval│           │
│  │  (Axum)  │  │ Worker   │  │ Engine   │           │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘           │
│       │              │              │                 │
│  ┌────▼──────────────▼──────────────▼──────────────┐ │
│  │            Core Domain Layer                     │ │
│  │  (Models, Traits, Error Handling)                │ │
│  └────┬──────────────┬──────────────┬──────────────┘ │
│       │              │              │                 │
│  ┌────▼─────┐  ┌─────▼─────┐  ┌────▼──────┐         │
│  │  Redis   │  │  Qdrant   │  │  GNN      │         │
│  │  (State) │  │ (Vectors) │  │ (Rerank)  │         │
│  └──────────┘  └───────────┘  └───────────┘         │
│                                                      │
│  ┌────────────┐                                      │
│  │  MCP Server│  (stdio / SSE transport)             │
│  └────────────┘                                      │
└──────────────────────────────────────────────────────┘
```

### Workspace Crates (v0.5.5)

| Crate | Purpose |
|-------|---------|
| `mnemo-core` | Domain models (14 modules), storage/LLM traits, error types |
| `mnemo-server` | Axum HTTP server, routes, AppState, dashboard SPA |
| `mnemo-storage` | Redis state store + Qdrant vector store implementations |
| `mnemo-graph` | Graph traversal, community detection, shortest path |
| `mnemo-ingest` | Background episode processing worker with sleep-time compute |
| `mnemo-retrieval` | Hybrid retrieval pipeline (semantic + graph + temporal) |
| `mnemo-llm` | LLM provider abstraction (Anthropic, OpenAI, Ollama, Liquid, local embeddings) |
| `mnemo-gnn` | GNN-enhanced retrieval re-ranking (GAT/GCN/GraphSAGE) |
| `mnemo-mcp` | Model Context Protocol server (stdio + SSE transport) |

Mnemo compiles to a single Rust binary (plus the `mnemo-mcp-server` binary for MCP transport). It connects to two external services: Redis for structured state and Qdrant for vector embeddings. There is no Neo4j, no JVM, no garbage collector.

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

When your agent calls `POST /api/v1/memory/:user/context` (or the lower-level `POST /api/v1/users/:id/context`), Mnemo runs a multi-step hybrid retrieval pipeline. The pipeline now supports goal-conditioned retrieval, semantic routing, GNN re-ranking, and narrative context (v0.5.5):

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

### 6. Semantic Routing (v0.5.0)

When `MNEMO_SEMANTIC_ROUTING_ENABLED=true`, an automatic classifier routes the query to the optimal retrieval strategy based on query semantics:

- **head mode** — "What did we discuss yesterday?" (recency-focused)
- **graph-focused** — "What are Alice's core beliefs?" (entity-centric)
- **hybrid** — default fallback

The routing decision is returned in the response as `routing_decision: { selected_mode, confidence, alternatives }`.

### 7. Reranking (RRF, MMR, or GNN)

Results from the parallel searches are merged and reranked using one of three strategies:

- **`rrf` (Reciprocal Rank Fusion)** — Default. Boosts candidates that appear in multiple ranked lists (entity + edge + episode). Effective for most workloads where relevance diversity is less important than consensus.
- **`mmr` (Maximal Marginal Relevance)** — Penalises near-duplicate results. Useful when queries tend to surface many semantically similar facts that would otherwise crowd the context window.
- **`gnn` (Graph Neural Network re-ranking, v0.5.0)** — When `MNEMO_GNN_ENABLED=true`, a lightweight GNN (`mnemo-gnn` crate) operates on the subgraph of candidate results. Multi-head attention learns which graph neighbors matter most for a given query, improving over time via implicit feedback (`POST /api/v1/memory/feedback`). Target: <1ms additional latency.

### 8. Goal-Conditioned Filtering (v0.5.5)

When the request includes a `goal` parameter (e.g. `goal=resolve_ticket`), Mnemo looks up the matching `GoalProfile` and applies goal-specific retrieval weights: entity type preferences, label boosts, recency bias, and max fact limits. The `goal_applied` flag in the response confirms the goal was active.

### 9. Context Assembly

Reranked results are assembled into a token-budgeted string with up to five sections:

1. **Known entities** — Name, type, and summary
2. **Current facts** — Natural language descriptions of relationships (with confidence scores from v0.5.5 decay model)
3. **Relevant conversation history** — Episode previews with timestamps
4. **Narrative summary** (v0.5.5) — Cross-session "story of the user" when `include_narrative=true`
5. **Stale facts needing revalidation** (v0.5.5) — Facts below confidence threshold flagged for attention

Section headers are counted against the `max_tokens` budget. Empty sections are never included.

---

## Storage Architecture

### Storage Traits

The `StateStore` composite trait combines 11 sub-traits, all implemented by `RedisStateStore`:

| Trait | Purpose |
|-------|---------|
| `UserStore` | User CRUD, external ID lookup |
| `SessionStore` | Session lifecycle |
| `EpisodeStore` | Episode CRUD, pending queue, batch create, claim/requeue |
| `EntityStore` | Entity CRUD, name-based dedup |
| `EdgeStore` | Edge CRUD, adjacency queries, conflict detection |
| `AgentStore` | Agent identity, experience events, promotions, COW branching, forking |
| `DigestStore` | Sleep-time memory digest persistence |
| `SpanStore` | LLM call span persistence (7-day TTL) |
| `ClarificationStore` | Self-healing memory clarification requests (v0.5.5) |
| `NarrativeStore` | Cross-session narrative summaries (v0.5.5) |
| `GoalStore` | Goal-conditioned retrieval profiles (v0.5.5) |

Separate traits for vector storage:
- `VectorStore` — Qdrant operations for entity/edge/episode embeddings, payload updates
- `RawVectorStore` — Namespace-based vector storage for external integrations (e.g. AnythingLLM)

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
| `mnemo:agent_branches:{agent_id}` | Sorted Set | COW identity branches (v0.5.0) |
| `mnemo:digest:{user_id}` | JSON | Memory digest per user (v0.5.0) |
| `mnemo:digests` | Sorted Set | All digests index |
| `mnemo:span:{id}` | JSON + EXPIRE | LLM call span (7-day TTL) (v0.5.0) |
| `mnemo:spans` | Sorted Set | Global span index |
| `mnemo:spans_request:{request_id}` | Sorted Set | Spans by request correlation ID |
| `mnemo:spans_user:{user_id}` | Sorted Set | Spans by user |
| `mnemo:clarification:{id}` | JSON | Self-healing clarification request (v0.5.5) |
| `mnemo:user_clarifications:{user_id}` | Sorted Set | User's clarifications |
| `mnemo:narrative:{user_id}` | JSON | Cross-session narrative (v0.5.5) |
| `mnemo:goal_profile:{id}` | JSON | Goal-conditioned retrieval profile (v0.5.5) |
| `mnemo:user_goals:{user_id}` | Sorted Set | User's goal profiles |
| `mnemo:global_goals` | Sorted Set | Global (shared) goal profiles |

Sorted sets are scored by timestamp for time-ordered pagination. Adjacency lists are scored by `valid_at` for temporal ordering.

### Qdrant (Vectors)

Three core collections, each with `user_id` in the payload for tenant filtering:

| Collection | Content | Payload Fields |
|---|---|---|
| `mnemo_entities` | Entity name + type + summary embeddings | `user_id`, `name`, `entity_type` |
| `mnemo_edges` | Fact description embeddings | `user_id`, `label`, `fact` |
| `mnemo_episodes` | Episode content embeddings | `user_id`, `session_id`, `processing_status`, `created_at` |

Additionally, `RawVectorStore` creates dynamically-named collections for external integrations (namespace-based, fully isolated from the above).

All collections use cosine distance. Dimensions match the configured embedding model (default: 384 for `AllMiniLML6V2` local embeddings; 1536 for `text-embedding-3-small` via OpenAI).

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
| LLM model | `MNEMO_LLM_MODEL` | e.g. `claude-haiku-4-20250514` |
| Embedding provider | `MNEMO_EMBEDDING_PROVIDER` | `local` (fastembed), `openai`, `ollama` |
| Embedding base URL | `MNEMO_EMBEDDING_BASE_URL` | OpenAI-compatible endpoint (when not `local`) |
| Embedding model | `MNEMO_EMBEDDING_MODEL` | e.g. `AllMiniLML6V2`, `nomic-embed-text`, `text-embedding-3-small` |
| Embedding dimensions | `MNEMO_EMBEDDING_DIMENSIONS` | integer matching the model (384 for AllMiniLML6V2) |

### Inference Policy

**Online (API-backed):** Use Anthropic with the project API key. Preferred extraction model: `claude-haiku-4-5` (fast, cheap) or `claude-sonnet-4-6` (higher quality).

**Local / offline:** Use **Liquid AI LFM2-24B-A2B** exclusively via Ollama (`MNEMO_LLM_PROVIDER=ollama`, `MNEMO_LLM_MODEL=hf.co/LiquidAI/LFM2-24B-A2B-GGUF`). Do not use other local models for extraction — LFM2-24B is the only validated local model for this workload.

**Embeddings (local, recommended):** Use `MNEMO_EMBEDDING_PROVIDER=local` with `MNEMO_EMBEDDING_MODEL=AllMiniLML6V2` and `MNEMO_EMBEDDING_DIMENSIONS=384`. This uses the built-in fastembed library — no external API needed, no API key, works offline. This is the production default.

**Embeddings (Ollama):** Use `nomic-embed-text` via Ollama. Set `MNEMO_EMBEDDING_DIMENSIONS=768`.

**Embeddings (OpenAI):** `text-embedding-3-small` (1536 dims) via OpenAI for highest quality when API access is available.

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

---

## Version History

### v0.4.0 — Agent Identity Substrate

Added agent-level identity layer with versioned profiles, experience events with time-decay, promotion proposals with approval gating, witness chain audit logs, and webhook delivery system with circuit breaker and retry.

### v0.5.0 — Self-Learning Memory Control Plane (13 features)

| # | Feature | Crate |
|---|---------|-------|
| 1 | GNN-enhanced retrieval re-ranking | `mnemo-gnn` (new) |
| 2 | SONA/EWC++ experience weight consolidation | `mnemo-core`, `mnemo-server` |
| 3 | Temporal tensor compression | `mnemo-retrieval`, `mnemo-server` |
| 4 | Coherence scoring + digest integration | `mnemo-retrieval`, `mnemo-core` |
| 5 | MCP server (Model Context Protocol) | `mnemo-mcp` (new) |
| 6 | Witness chain tamper-proof audit | `mnemo-core`, `mnemo-storage` |
| 7 | Semantic routing for retrieval strategy | `mnemo-retrieval` |
| 8 | Hyperbolic HNSW for entity hierarchy | `mnemo-retrieval` |
| 9 | COW branching for agent identity A/B testing | `mnemo-core`, `mnemo-storage` |
| 10 | DAG workflows for consolidation pipeline | `mnemo-ingest` |
| 11 | Delta consensus (CRDT multi-node sync) | `mnemo-core` |
| 12 | Domain expansion / transfer learning | `mnemo-core`, `mnemo-storage` |
| 13 | Verified/proof-carrying identity updates | `mnemo-core` |

### v0.5.5 — Autonomic Memory (5 features)

| # | Feature | Description |
|---|---------|-------------|
| 1 | Confidence decay + revalidation | Facts decay over time via configurable curves; Fisher importance protects load-bearing facts; stale facts flagged for revalidation |
| 2 | Self-healing memory | Auto-detect low-confidence conflicts, generate targeted clarification questions, reconcile graph state after answers |
| 3 | Cross-session narrative summaries | Evolving "story of the user" with versioned chapters, accessible via API and context retrieval |
| 4 | Goal-conditioned memory | Retrieval strategy conditioned by active objective (e.g. `resolve_ticket`, `plan_trip`); goal profiles with entity/label weights |
| 5 | Counterfactual memory | Simulate retrieval under hypothetical fact changes; diff current vs. hypothetical context for planning agents |
