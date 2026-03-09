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
7. **Implement `GET /graph/:user/path`** — Shortest path endpoint from the
   P0 roadmap spec. Requires BFS/Dijkstra in `GraphEngine`.
8. **Add entity type/name filters** — `?entity_type=...&name=...` query
   params on `graph_list_entities`.
9. **Add source/target entity filters** — Re-add `source_entity_id` and
   `target_entity_id` to `graph_list_edges`.
10. **Document graph API in API.md** — Five new routes are undocumented.
11. **Batch edge loading for community detection** — Replace N individual
    `get_outgoing_edges` calls with a bulk query.
12. **Integration tests for graph API** — All 5 routes, including auth
    boundary tests.

### P0-3: LLM call tracing
13. **Instrument all LLM call sites** — Currently only `extract_memory` and
    `refresh_memory_digest` record spans. The `remember_memory` handler's
    internal extraction path and any webhook-triggered LLM calls should also
    record spans.
14. **Persist spans to Redis** — In-memory ring buffer lost on restart.
15. **Expose user-lookup in dashboard** — The `/spans/user/:user_id` endpoint
    has no corresponding UI input field.
16. **Integration tests for span endpoints**.

### P0-5: Python SDK
17. **Type `graph_entity()` return** — Currently returns raw `dict`; should
    return a typed model.

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
18. **Feature parity with Python SDK** — Missing: time travel, conflict radar,
    causal recall, governance, full webhook management, operator summary.
19. **Build-validate locally** — `npm` is not available on this machine; the
    TS SDK was written but not compiled. CI should catch type errors.

## Tier 2: Polish

20. **Clean up redundant `use UserStore` imports** — Five warnings in graph
    handlers (already imported at module level).
21. **Align default limits** — Server defaults to 20 for graph edges, SDKs
    default to 100. Pick one and align.
22. **Add `encodeURIComponent` to `spansByUser` userId** in TS SDK.
23. **Proactive re-ranking** — P0.2 from the sleep-time spec (re-score
    entity/edge relevance during idle windows).

## Tier 3: Future capabilities

24. **Redis-backed span persistence with TTL** — Replace in-memory VecDeque
    with Redis sorted set, auto-expire after 7 days.
25. **Webhook TLS enforcement for updates** — Currently only checked at
    registration; should also check on `PATCH` webhook updates.
26. **Structured LLM output for digest** — Replace prompt parsing with
    tool/function calling to get reliable JSON output for topics.
27. **Token counting in spans** — `summarize()` and `extract()` don't
    currently return token counts. Wrap the LLM calls to capture usage.
