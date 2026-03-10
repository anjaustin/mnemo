# Next Moves

Post-P0 work items surfaced by the falsification audit (`docs/P0_VALIDATION.md`),
CI observations, and natural follow-ons. Ordered by priority within each tier.

---

## Tier 0: Ship blockers (do before v0.4.0 tag)

1. ~~**Validate CI green**~~ — **DONE.** All 4 workflows green (quality-gates,
   benchmark-eval, package-ghcr, memory-falsification).
2. ~~**Verify GHCR image exists**~~ — **DONE.** Published at
   `ghcr.io/anjaustin/mnemo/mnemo-server:v0.4.0`.
3. ~~**Tag v0.4.0 release**~~ — **DONE.** Tagged and released at
   https://github.com/anjaustin/mnemo/releases/tag/v0.4.0

## Tier 1: Hardening (deferred MAJOR findings)

### P0-1: Sleep-time compute
4. ~~**Idle-triggered consolidation scheduler**~~ — **DONE + FALSIFIED.**
   Background scheduler in `IngestWorker` triggers digest generation after
   configurable idle window (min 30s, default 300s) per user. Falsification
   fixed: consolidation now runs unconditionally (not only on idle cycles),
   lock contention removed in `record_user_activity`, 24h eviction added for
   unbounded `user_activity` map, minimum 30s idle window enforced, error
   variant fixed for no-entities case.
5. ~~**Persist digests to Redis**~~ — **DONE + FALSIFIED.** `DigestStore` trait
   in `mnemo-core`, implemented by `RedisStateStore`. Write-through from both
   the ingest worker and `POST /api/v1/memory/:user/digest`. Digests loaded
   from Redis on startup to warm the in-memory cache. Key schema:
   `{prefix}digest:{user_id}` (JSON) + `{prefix}digests` (sorted set index).
   Falsification audit found 16 issues (3 CRITICAL, 7 MAJOR, 6 MINOR).
   **Fixed:** cache-Redis split-brain (cache only populated on Redis success),
   atomic save/delete (`redis::pipe().atomic()`), GET read-through fallback
   (cache miss → Redis → cache populate), `digest_generated` flag gated on
   persistence success, POST handler error propagation, `PartialEq` derive,
   integration test covers HTTP GET read-through path.
   **Deferred:** `usize` for counts (matches codebase), no pagination on
   `list_digests` (acceptable scale), last-writer-wins on concurrent POST.
6. ~~**Integration tests for digest endpoints**~~ — **DONE.** 4 storage-level
   tests (`save_and_load`, `overwrite`, `list_all`, `delete`) + 3 API-level
   tests (`GET 404`, `persisted_to_redis_and_served` with read-through
   verification, `POST without LLM`).

### P0-2: Knowledge graph API
7. ~~**Implement `GET /graph/:user/path`**~~ — **DONE.** BFS shortest path
   in `GraphEngine::find_shortest_path()`. Route handler with from/to/max_depth/valid_only
   params. Cross-user entity ownership check. 8 unit tests + 3 integration tests.
8. ~~**Add entity type/name filters**~~ — **DONE.** `?entity_type=...&name=...`
   query params on `graph_list_entities` with case-insensitive matching. Over-fetch
   strategy (4x) when filters active. 2 integration tests.
9. ~~**Add source/target entity filters**~~ — **DONE.** `source_entity_id` and
   `target_entity_id` query params on `graph_list_edges`, passed through to
   `EdgeFilter.matches()`. 1 integration test with both filter types.
10. ~~**Document graph API in API.md**~~ — **DONE.** Full documentation for all
    7 graph endpoints with query params, examples, and response schemas.
11. ~~**Batch edge loading for community detection**~~ — **DONE.** Replaced N
    individual `get_outgoing_edges` calls with single `query_edges` batch query
    in `detect_communities`. Mock store updated with `EdgeFilter.matches()`.
12. ~~**Integration tests for graph API**~~ — **DONE.** 12 integration tests
    covering all 7 graph API endpoints + cross-user auth boundary tests.

### P0-3: LLM call tracing
13. ~~**Instrument all LLM call sites**~~ — **DONE.** Moved `LlmSpan` to
    `mnemo-core::models::span` (shared type). Added `SpanSink` to `IngestWorker`
    via `.with_span_sink()`. Instrumented 4 ingest worker LLM calls: `extract`,
    `embed_episode`, `session_summarize`, `digest`. Shared ring buffer between
    server routes and ingest worker.
14. ~~**Persist spans to Redis**~~ — **DONE.** `SpanStore` trait added to
    `mnemo-core` with 4 methods (`save_span`, `get_spans_by_request`,
    `get_spans_by_user`, `list_recent_spans`). Implemented by `RedisStateStore`
    with 7-day TTL (`SPAN_TTL_SECS = 604800`). Key schema:
    `{prefix}span:{id}` (JSON + EXPIRE), `{prefix}spans` (global sorted set),
    `{prefix}spans_request:{request_id}` and `{prefix}spans_user:{user_id}`
    (index sorted sets with TTL). Wired into both `IngestWorker.record_span()`
    and server's `record_llm_span()` with best-effort persistence. Query
    handlers (`list_spans_by_request`, `list_spans_by_user`) now read from
    Redis first with in-memory ring buffer fallback. 5 storage-level + 3
    API-level integration tests.
15. ~~**Expose user-lookup in dashboard**~~ — **DONE.** Added User ID input
    field + "Lookup by User" button to LLM Spans dashboard page. Added User column
    to span results table. Refactored span rendering into `renderSpanTable()`.
16. ~~**Integration tests for span endpoints**~~ — **DONE.** 5 storage-level
    tests (`save_and_load_by_request`, `load_by_user`, `list_recent`,
    `no_request_id_not_indexed`, `roundtrip_preserves_fields`) + 3 API-level
    tests (`spans_by_request_from_redis`, `spans_by_user_from_redis`,
    `spans_by_request_empty_returns_empty`). Tests inject spans directly via
    `SpanStore::save_span()` to verify the Redis read-through path.

### P0-5: Python SDK
17. ~~**Type `graph_entity()` return**~~ — **DONE.** Added `GraphEntityDetail`
    and `AdjacencyEdge` models. Both sync and async clients now return typed
    `GraphEntityDetail` with `outgoing_edges`/`incoming_edges` lists.

### P0-5b: LlamaIndex adapter
    ~~**Harden LlamaIndex adapter**~~ — **DONE + FALSIFIED.** Falsification
    found `BaseChatStore.register()` does NOT enable `isinstance()` with
    Pydantic's `ModelMetaclass` — removed; adapter works via duck typing
    (which is how `ChatMemoryBuffer` actually dispatches). Other fixes:
    `delete_message`/`delete_last_message` now use server-side `idx` field
    (not list position), `delete_last_message` single-fetch (no TOCTOU race),
    `_safe_content()` handles `None`/list content, `asyncio.Lock` for async
    dict safety, `except MnemoError` replaces bare `except Exception`,
    `_role_value` normalizes case, `_ensure_uuid` resolves sessions created
    externally, `async_add_message` alias added for LlamaIndex compat.
    59 unit tests (36 sync + 13 async + 10 edge cases).

### P0-6: TypeScript SDK
18. ~~**Feature parity with Python SDK**~~ — **DONE + FALSIFIED.** Added 20+
    methods with full type definitions. Falsification audit found **6 CRITICAL**
    and **13 MAJOR** issues across both TS and Python SDKs — all sharing the
    same root causes:
    - Request payloads sent `from_dt`/`to_dt` but server expects `from`/`to`
      (changesSince, timeTravelTrace, timeTravelSummary) — **FIXED** (both SDKs)
    - Response unwrapping mismatches: getWebhook/getImportJob unwrapped
      `.webhook`/`.job` but server returns bare objects — **FIXED** (TS SDK;
      Python already had defensive fallback)
    - Response field name mismatches in 8 endpoints: getPolicyAudit (`.data`→`.audit`),
      getWebhookAudit (`.events`→`.audit`), replayEvents (`replayed`→`count`,
      `after`→`after_event_id`), retryEvent (`ok`→`queued`), getWebhookStats
      (all field names differ), previewPolicy (`estimated_episodes_affected`→
      `estimated_affected_episodes_total`, `policy`→`current_policy`+`preview_policy`),
      traceLookup (arrays need `matched_` prefix), opsSummary (wrong field name
      `governance_audit_total`→`governance_audit_events_in_window`, missing 5 fields)
      — **ALL FIXED** (both SDKs)
    - listSessions wrong endpoint / createSession wrong field — **FIXED** (TS SDK;
      Python already correct)
    - add() sent `metadata` field server ignores — **FIXED** (removed from TS SDK)
    - AuditRecord field names `action`/`at` vs server's `event_type`/`created_at` — **FIXED**
    - AdjacencyEdge asymmetry documented in Python SDK — **DONE**
19. **Build-validate locally** — `npm`/`node` not available on this machine;
    the TS SDK was written but not compiled. CI should catch type errors.

### Backend hardening
28. ~~**Clamp `query_edges` HTTP endpoint limit**~~ — **DONE.** The `query_edges`
    route handler passed user-supplied `EdgeFilter.limit` unclamped to the
    storage layer (DoS vector). Added `.clamp(1, 1000)` consistent with all
    other paginated endpoints.

## Tier 2: Polish

20. ~~**Clean up redundant `use UserStore` imports**~~ — **DONE.** Already
    resolved; clippy shows zero warnings.
21. ~~**Align default limits**~~ — **DONE.** Both Python and TS SDK graph
    entity/edge methods aligned to server default of 20.
22. ~~**Add `encodeURIComponent` to `spansByUser` userId**~~ — **DONE.**
23. ~~**Proactive re-ranking**~~ — **DONE.** P0.2 sleep-time re-scoring
    implemented. During idle windows, `IngestWorker.proactive_rerank()` computes
    composite relevance scores for entities (mention_count + recency decay +
    edge density) and edges (confidence + corroboration_count + recency decay),
    then writes scores to Qdrant payloads via `VectorStore.set_entity_payload()`
    / `set_edge_payload()` (payload-only update, no embedding re-upload).
    Tracked by `rerank_generated` set (cleared on user activity). Integration
    tests: `test_proactive_rerank_writes_relevance_scores` (verifies payload
    fields and score ranges) and `test_proactive_rerank_idempotent_per_idle_window`
    (verifies no duplicate reranking in same idle window).

## Tier 3: Future capabilities

24. ~~**Redis-backed span persistence with TTL**~~ — **DONE.** (See item 14 above.)
25. ~~**Webhook TLS enforcement for updates**~~ — **DONE.** Added
    `PATCH /api/v1/memory/webhooks/:id` endpoint with full TLS enforcement,
    domain allowlist policy check, and audit trail. Previously only POST
    (register) existed — now updates apply the same three validation gates:
    `is_http_url()`, `require_tls` HTTPS enforcement, `is_target_url_allowed()`
    domain allowlist. SDK support added: TS `updateWebhook()`, Python sync
    `update_webhook()`, Python async `update_webhook()`. 5 integration tests:
    field update, 404, TLS rejection, domain allowlist, audit trail.
26. ~~**Structured LLM output for digest**~~ — **DONE.** Digest prompt now
    requests JSON `{"summary": "...", "topics": [...]}` instead of fragile
    `TOPICS:` line parsing. New `parse_digest_response()` function (pub in
    `mnemo-ingest`) tries JSON parse first (handles markdown fences), falls
    back to legacy `TOPICS:` format for backward compat. Shared between
    ingest worker and HTTP handler. 8 unit tests cover JSON, fenced JSON,
    legacy format, plain text, topic cap, empty summary fallback.
27. ~~**Token counting in spans**~~ DONE — Added `TokenUsage` struct and
    `extract_with_usage()` / `summarize_with_usage()` methods to the
    `LlmProvider` trait (with default impls that delegate with zero usage).
    Both `OpenAiCompatibleProvider` and `AnthropicProvider` override these
    to parse real `usage` fields from API responses (OpenAI returns
    `prompt_tokens`/`completion_tokens`/`total_tokens`; Anthropic returns
    `input_tokens`/`output_tokens` — total computed). Updated all callers:
    ingest worker (`process_episode`, `generate_digest`, session
    summarization) and the server digest HTTP handler now capture and
    record real token counts in `LlmSpan`. `LlmHandle` in `state.rs`
    exposes `summarize_with_usage()` for the server route layer.
