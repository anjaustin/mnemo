# Next Moves

Post-P0 work items surfaced by the falsification audit (`docs/P0_VALIDATION.md`),
CI observations, and natural follow-ons. Ordered by priority within each tier.

---

## Tier 0: Ship blockers (do before v0.4.0 tag)

1. **Validate CI green** — `quality-gates` workflow failed on the last push.
   Investigate and fix before tagging.
2. **Verify GHCR image exists** — `package-ghcr` workflow was in-progress.
   Confirm the image is published and pullable.
3. **Tag v0.4.0 release** — Once CI is green, tag and create GitHub release
   with changelog covering all 7 P0s.

## Tier 1: Hardening (deferred MAJOR findings)

### P0-1: Sleep-time compute
4. ~~**Idle-triggered consolidation scheduler**~~ — **DONE.** Background
   scheduler in `IngestWorker` triggers digest generation after configurable
   idle window (`MNEMO_SLEEP_IDLE_WINDOW_SECONDS`, default 300s) per user.
   Supports local LLM providers (Ollama, Liquid AI) for zero cloud cost.
5. **Persist digests to Redis** — Currently in-memory only; lost on restart.
6. **Integration tests for digest endpoints** — GET (404 + success) and POST.

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
    ~~**Harden LlamaIndex adapter**~~ — **DONE.** `MnemoChatStore` now
    registers as `BaseChatStore` virtual subclass (isinstance passes),
    `get_keys()` queries server-side session list via new
    `client.list_sessions()` with graceful fallback, `_user_uuid` resolved
    on first write. 36 unit tests in `test_llamaindex_adapter.py` all pass.

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
