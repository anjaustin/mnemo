# Mnemo Phase 1.5 — Production Hardening PRD

**Version:** 0.1.5-PRD
**Date:** 2026-03-01
**Predecessor:** Phase 1 (v0.1.0) — Core Engine (complete)

## Status Snapshot (2026-03-02)

Phase 1.5 is functionally complete, with follow-on scope added beyond the original PRD.

| Item | Status | Notes |
|---|---|---|
| 3.1 Compilation fixes | Complete | Workspace compiles and runs in CI.
| 3.2 Anthropic provider | Complete | Native provider implemented.
| 3.3 Auth middleware | Complete | API key middleware in place and tested.
| 3.4 Retry/backoff | Complete | Retry behavior implemented in ingestion path.
| 3.5 Full-text search | Complete | RediSearch path integrated in retrieval.
| 3.6 Integration/e2e tests | Complete | Integration suite plus memory falsification tests in CI.
| 3.7 Benchmarks | Partially complete | Repro harness + CI benchmark workflow shipped; benchmark docs are now in `docs/EVALUATION.md` and `docs/COMPETITIVE.md` with run evidence.

Additional scope shipped after this PRD:

- High-level memory API (`/api/v1/memory`, `/api/v1/memory/:user/context`)
- Temporal scoring controls (`time_intent`, `as_of`, `temporal_weight`)
- Thread HEAD mode and diagnostics (`mode=head|hybrid|historical`, `head` metadata)
- Python SDK and quickstart workflow

---

## 1. Motivation

Phase 1 delivered a complete skeleton: domain models, storage backends, LLM providers, ingestion pipeline, retrieval engine, REST API, and documentation. But it doesn't compile against real crate APIs yet, has zero integration tests, no authentication, a broken Anthropic provider, unused retry logic, and no full-text search.

A seasoned developer evaluating Mnemo today would star the repo and come back in 6 weeks. Phase 1.5 closes that gap. The goal is: **a developer can clone, run `docker compose up`, hit the API, and see real entities extracted from real messages in under 10 minutes.**

---

## 2. Success Criteria

A PR reviewer can verify each of these:

| # | Criterion | Verification |
|---|-----------|--------------|
| 1 | `cargo build --workspace` succeeds with zero warnings | CI green |
| 2 | `cargo test --workspace` passes all unit + integration tests | CI green |
| 3 | End-to-end smoke test passes (create user → add episode → get context with extracted entities) | `tests/e2e.sh` exits 0 |
| 4 | Anthropic Claude works as LLM provider with real API key | Manual test documented |
| 5 | API key auth rejects unauthenticated requests when enabled | Integration test |
| 6 | Failed episodes are retried up to `max_retries` with exponential backoff | Unit test |
| 7 | Full-text search returns results for exact keyword queries | Integration test |
| 8 | Benchmarks page exists with reproducible latency numbers | `docs/BENCHMARKS.md` |

---

## 3. Work Items

### 3.1 — Compilation Fixes

**Problem:** The codebase was written without `cargo check` available. Several crate API usages (Qdrant client, Redis, Axum) are based on documentation review, not tested compilation. The code will not compile as-is.

**Scope:**
- Fix all compilation errors across 7 crates
- Resolve dependency version mismatches (Qdrant SDK API surface, Redis async API, Axum 0.8 route macros)
- Verify `edition = "2021"` and `thiserror = "1"` work everywhere (already fixed in falsification pass)
- Generate and commit `Cargo.lock`

**Acceptance:** `cargo build --workspace` and `cargo clippy --workspace -- -D warnings` both succeed.

**Estimated effort:** 1–2 days. Most issues will be type mismatches and API changes in the Qdrant/Redis crates.

---

### 3.2 — Anthropic Native Provider

**Problem:** The `OpenAiCompatibleProvider` claims Anthropic support but sends the OpenAI chat completions request format. Anthropic's Messages API differs in three ways:
1. System prompt is a top-level `system` parameter, not a message with `role: "system"`
2. The response structure uses `content` blocks, not `choices[0].message.content`
3. The endpoint is `/v1/messages`, not `/v1/chat/completions`

Sending the current request format to `api.anthropic.com` will return a 400 error.

**Scope:**
- Create `AnthropicProvider` as a new struct in `mnemo-llm` (not a variant of `OpenAiCompatibleProvider`)
- Implement the Anthropic Messages API format:
  - `POST https://api.anthropic.com/v1/messages`
  - Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`
  - Body: `{ model, system, messages: [{role: "user", content}], max_tokens, temperature }`
  - Response: `{ content: [{type: "text", text: "..."}] }`
- Wire the provider selection in config: `provider = "anthropic"` → `AnthropicProvider`
- Keep `OpenAiCompatibleProvider` for OpenAI, Ollama, Liquid, vLLM (it's correct for those)

**Request format:**
```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 2048,
  "system": "You are an entity extraction engine...",
  "messages": [
    {"role": "user", "content": "Extract entities from: ..."}
  ],
  "temperature": 0.0
}
```

**Response format:**
```json
{
  "content": [
    {"type": "text", "text": "{\"entities\": [...], \"relationships\": [...]}"}
  ]
}
```

**Acceptance:** With `MNEMO_LLM_PROVIDER=anthropic` and a valid API key, entity extraction succeeds end-to-end. Integration test included.

**Estimated effort:** 0.5 days.

---

### 3.3 — API Key Authentication Middleware

**Problem:** Zero authentication exists. Any network-reachable client can read, write, and delete all user data.

**Scope:**
- Axum middleware layer that checks `Authorization: Bearer <key>` or `X-API-Key: <key>` headers
- Keys loaded from config (`auth.api_keys`) and environment (`MNEMO_AUTH_API_KEYS`, comma-separated)
- `auth.enabled = false` (default) skips the middleware entirely (development mode)
- `auth.enabled = true` requires a valid key on every request except `GET /health`
- Invalid/missing key returns `401 Unauthorized` with `{"error": {"code": "unauthorized", "message": "Invalid or missing API key"}}`

**Design:**
```rust
// In mnemo-server/src/middleware/auth.rs
pub struct AuthLayer {
    enabled: bool,
    valid_keys: HashSet<String>,
}

impl<S> tower::Layer<S> for AuthLayer {
    // Extract key from Authorization or X-API-Key header
    // Compare against valid_keys set
    // Pass through if disabled or key matches
    // Return 401 if enabled and key invalid
}
```

**Not in scope (Phase 2+):**
- JWT tokens
- Per-user API keys with scoped permissions
- Rate limiting per key
- Key rotation

**Acceptance:** Integration test creates a request without a key → 401. Request with valid key → 200. Auth disabled → all requests pass.

**Estimated effort:** 0.5 days.

---

### 3.4 — Ingestion Retry with Exponential Backoff

**Problem:** The ingestion worker has `max_retries: 3` in its config but never uses it. A single LLM timeout marks the episode as permanently `failed`.

**Scope:**
- Track retry count on the `Episode` model: add `retry_count: u32` field
- On processing failure:
  - If `retry_count < max_retries`: increment `retry_count`, set status back to `pending`, re-add to the pending sorted set with a delayed score (current time + backoff)
  - If `retry_count >= max_retries`: mark as `failed` (permanent)
- Backoff formula: `base_delay_ms * 2^retry_count` (500ms, 1s, 2s for 3 retries)
- The pending sorted set naturally handles delay: episodes scored in the future won't be picked up by `ZRANGE` until their score time arrives

**Episode model change:**
```rust
// In episode.rs
pub struct Episode {
    // ... existing fields ...
    
    /// Number of times this episode has been retried after failure.
    #[serde(default)]
    pub retry_count: u32,
}
```

**Worker change:**
```rust
// In poll_and_process, on failure:
if episode.retry_count < self.config.max_retries {
    ep.retry_count += 1;
    ep.processing_status = ProcessingStatus::Pending;
    ep.processing_error = Some(e.to_string());
    self.state_store.update_episode(&ep).await?;
    // Re-add to pending with delayed score
    let delay_ms = 500 * 2u64.pow(ep.retry_count);
    let future_score = (Utc::now().timestamp_millis() + delay_ms as i64) as f64;
    // ZADD pending_episodes future_score episode_id
} else {
    ep.mark_failed(e.to_string());
    self.state_store.update_episode(&ep).await?;
}
```

**Acceptance:** Unit test: episode fails 3 times → retried 3 times with increasing delays → 4th failure marks as `failed` permanently. Retry count visible in episode GET response.

**Estimated effort:** 0.5 days.

---

### 3.5 — Full-Text Search via RediSearch

**Problem:** Retrieval only uses semantic search (Qdrant) + graph traversal. Exact keyword matches (names, IDs, product codes, quoted phrases) are missed because embedding models map them to semantic neighborhoods, not exact matches.

**Prerequisite:** The Docker Compose already uses `redis/redis-stack`, which includes RediSearch. No new infrastructure required.

**Scope:**
- Create RediSearch indexes for entities, edges, and episodes
- Add `FullTextStore` trait to `mnemo-core`
- Implement in `mnemo-storage` using RediSearch `FT.CREATE`, `FT.SEARCH`
- Integrate into `RetrievalEngine` as a parallel search path alongside semantic search
- Implement Reciprocal Rank Fusion (RRF) to merge semantic + full-text results

**Index schema:**
```
FT.CREATE mnemo:idx:entities ON JSON
  PREFIX 1 mnemo:entity:
  SCHEMA
    $.name AS name TEXT WEIGHT 2.0
    $.summary AS summary TEXT
    $.user_id AS user_id TAG
    $.entity_type AS entity_type TAG

FT.CREATE mnemo:idx:edges ON JSON
  PREFIX 1 mnemo:edge:
  SCHEMA
    $.fact AS fact TEXT
    $.label AS label TAG
    $.user_id AS user_id TAG

FT.CREATE mnemo:idx:episodes ON JSON
  PREFIX 1 mnemo:episode:
  SCHEMA
    $.content AS content TEXT
    $.user_id AS user_id TAG
    $.session_id AS session_id TAG
```

**FullTextStore trait:**
```rust
pub trait FullTextStore: Send + Sync {
    async fn search_entities_ft(
        &self, user_id: Uuid, query: &str, limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;
    
    async fn search_edges_ft(
        &self, user_id: Uuid, query: &str, limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;
    
    async fn search_episodes_ft(
        &self, user_id: Uuid, query: &str, limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    async fn ensure_indexes(&self) -> StorageResult<()>;
}
```

**Reciprocal Rank Fusion:**
```
RRF_score(item) = Σ  1 / (k + rank_in_source)
                  for each source that returned the item

k = 60 (standard constant)
```

Merge results from semantic search, full-text search, and graph traversal. Deduplicate by ID. Sort by RRF score.

**Acceptance:** Integration test: add episode with "invoice #INV-2024-0847", search for "INV-2024-0847" → returns the episode. Semantic search alone would miss this. Full-text search catches it.

**Estimated effort:** 1.5 days.

---

### 3.6 — Integration Tests

**Problem:** Only `mnemo-core` has unit tests (pure domain logic). No code that touches Redis, Qdrant, or an LLM has been tested.

**Scope:**
- Docker Compose test profile (`docker-compose.test.yml`) with Redis + Qdrant
- Integration test suite in `tests/integration/` covering:

**Storage tests (`tests/integration/storage.rs`):**
- User CRUD lifecycle
- Session CRUD lifecycle
- Episode creation + pending queue behavior
- Episode atomic claim (two concurrent claims, only one succeeds)
- Entity create + find_by_name dedup
- Entity alias index
- Edge CRUD + adjacency list queries
- Edge conflict detection + invalidation
- Pagination (cursor-based, correct ordering)
- Delete cascading (user delete cleans up vectors)

**Ingestion tests (`tests/integration/ingest.rs`):**
- Full pipeline: add episode → wait → verify entities + edges created
- Duplicate entity dedup across multiple episodes
- Contradiction detection: two conflicting facts → old edge invalidated
- Retry behavior: mock LLM fails once, succeeds on retry

**Retrieval tests (`tests/integration/retrieval.rs`):**
- Semantic search returns relevant entities
- Context assembly respects token budget
- Temporal filter excludes invalidated edges
- Graph traversal includes connected facts
- Full-text search finds exact matches (after 3.5 lands)

**End-to-end smoke test (`tests/e2e.sh`):**
```bash
#!/bin/bash
# Starts stack, runs full workflow via curl, verifies responses
# Exit 0 = pass, Exit 1 = fail
# Used in CI and as the "10-minute developer experience"
```

**Acceptance:** `docker compose -f docker-compose.test.yml up -d && cargo test --test integration` passes. `./tests/e2e.sh` passes.

**Estimated effort:** 2 days.

---

### 3.7 — Benchmarks

**Problem:** The original PRD claims <50ms P95 retrieval and ≥95% on DMR. No numbers have been measured.

**Scope:**
- Benchmarking harness in `benches/` using Criterion
- Latency benchmarks:
  - Episode ingestion (API call → stored in Redis): target <5ms P95
  - Context retrieval (API call → context string returned): target <50ms P95
  - Semantic search (Qdrant round-trip): target <30ms P95
  - Entity extraction (LLM round-trip): measure, don't target (LLM-dependent)
- Throughput benchmark:
  - Concurrent episode ingestion: target 1000 episodes/sec sustained
  - Concurrent context retrieval: target 500 req/sec
- Memory benchmark:
  - Base memory footprint of server process
  - Memory per 1000 users with 100 episodes each
- Publish results in `docs/BENCHMARKS.md` with hardware spec, methodology, and reproduction instructions

**Not in scope:** DMR and LongMemEval accuracy benchmarks (these require significant dataset preparation — deferred to Phase 2).

**Acceptance:** `docs/BENCHMARKS.md` exists with real numbers from a reproducible test. Numbers are measured, not estimated.

**Estimated effort:** 1 day.

---

## 4. Dependency Graph

Work items have this dependency order:

```
3.1 Compilation Fixes
 ├──▶ 3.2 Anthropic Provider
 ├──▶ 3.3 Auth Middleware
 ├──▶ 3.4 Retry with Backoff
 ├──▶ 3.5 Full-Text Search (RediSearch)
 │
 └──▶ 3.6 Integration Tests (depends on all above)
      └──▶ 3.7 Benchmarks (depends on working, tested code)
```

3.1 must land first. 3.2–3.5 can be parallelized. 3.6 runs after all code changes. 3.7 runs last.

---

## 5. Files Created / Modified

| Work Item | New Files | Modified Files |
|-----------|-----------|----------------|
| 3.1 Compilation | `Cargo.lock` | All `*.rs` files (as needed for compile fixes) |
| 3.2 Anthropic | `crates/mnemo-llm/src/anthropic.rs` | `crates/mnemo-llm/src/lib.rs`, `crates/mnemo-server/src/main.rs` (provider selection) |
| 3.3 Auth | `crates/mnemo-server/src/middleware/auth.rs`, `crates/mnemo-server/src/middleware/mod.rs` | `crates/mnemo-server/src/main.rs`, `crates/mnemo-server/src/config.rs`, `crates/mnemo-server/src/lib.rs` |
| 3.4 Retry | — | `crates/mnemo-core/src/models/episode.rs`, `crates/mnemo-ingest/src/lib.rs`, `crates/mnemo-storage/src/redis_store.rs` |
| 3.5 Full-Text | `crates/mnemo-core/src/traits/fulltext.rs`, `crates/mnemo-storage/src/redisearch.rs` | `crates/mnemo-core/src/traits/mod.rs`, `crates/mnemo-storage/src/lib.rs`, `crates/mnemo-retrieval/src/lib.rs` |
| 3.6 Integration | `tests/integration/storage.rs`, `tests/integration/ingest.rs`, `tests/integration/retrieval.rs`, `tests/e2e.sh`, `docker-compose.test.yml` | `Cargo.toml` (workspace test config) |
| 3.7 Benchmarks | `benches/latency.rs`, `benches/throughput.rs`, `docs/BENCHMARKS.md` | `Cargo.toml` (bench config) |

---

## 6. Estimated Total Effort

| Item | Estimate |
|------|----------|
| 3.1 Compilation Fixes | 1–2 days |
| 3.2 Anthropic Provider | 0.5 days |
| 3.3 Auth Middleware | 0.5 days |
| 3.4 Retry with Backoff | 0.5 days |
| 3.5 Full-Text Search | 1.5 days |
| 3.6 Integration Tests | 2 days |
| 3.7 Benchmarks | 1 day |
| **Total** | **7–8 days** |

Sequential execution: ~8 days. With parallel work on 3.2–3.5 after 3.1 lands: **~5 days.**

---

## 7. What Phase 1.5 Does NOT Include

These are explicitly deferred to Phase 2:

- **SDKs** (Python, TypeScript) — the REST API is stable enough for curl/httpx
- **gRPC API** — REST is sufficient for now
- **Progressive summarization** — improves context quality but isn't a blocker
- **Document ingestion** (PDF, Markdown) — valuable but not core
- **MCP server** — ecosystem play, not production hardening
- **Admin dashboard** — nice-to-have, not need-to-have
- **Helm chart** — Docker Compose is sufficient for early adopters
- **DMR/LongMemEval accuracy benchmarks** — require significant dataset work
- **JWT auth / per-user keys / RBAC** — simple API key auth is enough for v0.1.5

---

## 8. Post-Phase 1.5 State

After this work ships, a developer evaluating Mnemo will find:

1. **It compiles and runs.** `cargo build` succeeds. `docker compose up` starts the full stack.
2. **It's tested.** Integration tests cover the full pipeline. An e2e smoke test proves the happy path.
3. **It's secure enough.** API key auth blocks unauthorized access.
4. **The Anthropic claim is real.** `MNEMO_LLM_PROVIDER=anthropic` actually works.
5. **Failures are handled.** Transient LLM errors don't permanently lose episodes.
6. **Search actually works.** Both semantic and keyword queries return results.
7. **Performance is measured.** Real latency numbers, not aspirational targets.

This is the difference between "I'll star it and come back" and "I'll run it in staging this week."
