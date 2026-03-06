# QA/QC Falsification PRD

**Status:** P0 active  
**Version:** 0.3.3  
**Created:** 2026-03-05  
**Purpose:** Systematic, adversarial verification of every feature, endpoint, claim, and artifact in the Mnemo codebase. Nothing is trusted until it is tested. If a claim cannot be falsified, it is not a claim — it is hope.

> "Honor the skeptics so that we honor ourselves."

---

## Principles

1. **Every shipped feature gets a gate.** No feature is "done" until an adversarial test exists that would catch its regression.
2. **Claims are liabilities.** Every assertion in README, CHANGELOG, PRDs, and docs is a claim that can be false or stale. Each must be verified or corrected.
3. **Gaps are ranked by blast radius.** Untested code that touches data paths (storage, retrieval, ingestion) is more dangerous than untested code that touches cosmetics.
4. **Cross-document consistency is a gate.** If two documents disagree, one is wrong. Find it.
5. **Tests must be runnable.** A test that requires manual infrastructure setup or secret keys and cannot run in CI is a liability, not an asset.

---

## Table of Contents

1. [Domain 1: Core API Surface](#domain-1-core-api-surface)
2. [Domain 2: Memory Write Path](#domain-2-memory-write-path)
3. [Domain 3: Memory Read/Recall Path](#domain-3-memory-readrecall-path)
4. [Domain 4: Temporal Reasoning](#domain-4-temporal-reasoning)
5. [Domain 5: Webhook System](#domain-5-webhook-system)
6. [Domain 6: Governance & Policies](#domain-6-governance--policies)
7. [Domain 7: Agent Identity Substrate](#domain-7-agent-identity-substrate)
8. [Domain 8: Chat History Import](#domain-8-chat-history-import)
9. [Domain 9: Raw Vector API](#domain-9-raw-vector-api)
10. [Domain 10: Session Messages API](#domain-10-session-messages-api)
11. [Domain 11: Operator Endpoints](#domain-11-operator-endpoints)
12. [Domain 12: Graph Engine](#domain-12-graph-engine)
13. [Domain 13: LLM Providers](#domain-13-llm-providers)
14. [Domain 14: Storage Layer](#domain-14-storage-layer)
15. [Domain 15: Retrieval Engine](#domain-15-retrieval-engine)
16. [Domain 16: Python SDK](#domain-16-python-sdk)
17. [Domain 17: Framework Adapters](#domain-17-framework-adapters)
18. [Domain 18: AnythingLLM Integration](#domain-18-anythingllm-integration)
19. [Domain 19: Configuration & Startup](#domain-19-configuration--startup)
20. [Domain 20: Auth Middleware](#domain-20-auth-middleware)
21. [Domain 21: CI/CD Pipelines](#domain-21-cicd-pipelines)
22. [Domain 22: Docker & Packaging](#domain-22-docker--packaging)
23. [Domain 23: Deployment Artifacts](#domain-23-deployment-artifacts)
24. [Domain 24: Documentation Consistency](#domain-24-documentation-consistency)
25. [Domain 25: Security & Credential Hygiene](#domain-25-security--credential-hygiene)
26. [Appendix A: Known Inconsistencies](#appendix-a-known-inconsistencies)
27. [Appendix B: Coverage Gap Matrix](#appendix-b-coverage-gap-matrix)

---

## Domain 1: Core API Surface

**Scope:** All 66 route bindings across 54 unique paths. Every endpoint must respond correctly to valid input, reject invalid input with proper error codes, and include `x-mnemo-request-id` in responses.

**Current coverage:** 56/66 routes tested in `memory_api.rs`. Remaining routes tested via E2E scripts or Python tests.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| API-01 | Every endpoint returns `x-mnemo-request-id` header | Automated | Iterate all 54 paths, assert header present on every response (2xx and 4xx) | ❌ Not tested systematically |
| API-02 | Client-provided `x-mnemo-request-id` is echoed back | Automated | Send custom request ID, assert same ID in response header | ✅ Tested in `test_request_id_header_is_set_and_propagated` |
| API-03 | `/health` returns `{"status":"ok","version":"0.3.3"}` | Automated | `GET /health`, assert status and version match `Cargo.toml` | ✅ Tested in E2E smoke |
| API-04 | `/healthz` returns same response as `/health` | Automated | `GET /healthz`, compare to `/health` response | ❌ Not explicitly tested |
| API-05 | Unknown routes return 404, not 500 | Automated | `GET /api/v1/nonexistent`, assert 404 | ❌ Not tested |
| API-06 | All error responses follow `{"error":{"code":"...","message":"..."}}` format | Automated | Trigger 400, 401, 404, 409, 429, 500 and validate JSON shape | ❌ Not tested systematically |
| API-07 | `GET /metrics` returns `text/plain` Prometheus format | Automated | Assert content type and at least one `http_requests_total` line | ✅ Tested in `test_metrics_endpoint_exposes_prometheus_text` |
| API-08 | All list endpoints support cursor-based pagination (`limit`, `after`) | Automated | Create >20 entities, paginate with `limit=5`, verify cursor behavior | ⚠️ Partially tested (some endpoints) |
| API-09 | All list endpoints return newest-first ordering | Automated | Insert items with known timestamps, verify descending order | ⚠️ Partially tested |

---

## Domain 2: Memory Write Path

**Scope:** `POST /api/v1/memory`, `POST /api/v1/sessions/:id/episodes`, `POST /api/v1/sessions/:id/episodes/batch`. The complete lifecycle from client write to episode persisted in Redis.

**Current coverage:** Good — tested in `memory_api.rs` and E2E scripts.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| MW-01 | `POST /api/v1/memory` auto-creates user and session | Automated | Write with new user name, assert `user_id` and `session_id` returned as UUIDs | ✅ Tested |
| MW-02 | `POST /api/v1/memory` with existing user reuses user | Automated | Write twice with same user, assert same `user_id` | ✅ Tested |
| MW-03 | `session` field is a name string, not UUID | Automated | Write with `session: "my-session"`, assert `session_id` is a UUID (not the string) | ✅ Tested |
| MW-04 | `text` is the correct field name (not `content`) | Automated | Write with `content` field, assert 400 or silent failure detection | ⚠️ Implicitly tested |
| MW-05 | `user` is the correct field name (not `user_id`) | Automated | Write with `user_id` field, assert 400 or silent failure detection | ⚠️ Implicitly tested |
| MW-06 | Episode is persisted to Redis immediately (sync) | Automated | Write, immediately `GET /api/v1/sessions/:id/episodes`, assert count > 0 | ✅ Tested |
| MW-07 | Batch episode endpoint creates multiple episodes | Automated | `POST .../episodes/batch` with 3 episodes, list, assert count == 3 | ❌ Not tested |
| MW-08 | Empty `text` field is rejected with 400 | Automated | Write with `text: ""`, assert 400 | ❌ Not tested |
| MW-09 | Missing `user` field is rejected with 400 | Automated | Write with no `user`, assert 400 | ❌ Not tested |
| MW-10 | Retention policy blocks stale episode write | Automated | Set retention, write with old timestamp, assert rejection | ✅ Tested in `test_policy_retention_blocks_stale_episode_write` |

---

## Domain 3: Memory Read/Recall Path

**Scope:** `POST /api/v1/memory/:user/context`, `POST /api/v1/users/:user_id/context`. The complete retrieval pipeline from query to assembled context.

**Current coverage:** Good for happy paths. Gaps in edge cases.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| MR-01 | Context includes recently written text (immediate recall fallback) | Automated | Write, immediately query context, assert written text appears | ✅ Tested |
| MR-02 | `:user` can be UUID, external_id, or name | Automated | Create user with external_id, query context with all three identifiers | ⚠️ UUID and name tested, external_id not explicitly |
| MR-03 | Non-existent user returns 404 | Automated | Query context for fake user, assert 404 | ❌ Not tested |
| MR-04 | `contract` parameter affects retrieval behavior | Automated | Query with `historical_strict` (requires `as_of`), verify 400 without it | ✅ Tested |
| MR-05 | `retrieval_policy` parameter affects response diagnostics | Automated | Query with each policy, assert `retrieval_policy_diagnostics` present | ✅ Tested |
| MR-06 | `mode=head` returns head diagnostics | Automated | Query with `mode=head`, assert `head` key in response | ✅ Tested |
| MR-07 | `mode=head` with no sessions returns empty head | Automated | Query new user with `mode=head`, assert empty head | ✅ Tested |
| MR-08 | `filters` parameter restricts results | Automated | Write episodes with different metadata, filter, assert correct subset | ✅ Tested |
| MR-09 | `max_tokens` budget is respected | Automated | Set low max_tokens, assert context length within budget | ⚠️ Implicitly tested |
| MR-10 | Empty context returns gracefully (not error) | Automated | Query user with no episodes, assert 200 with empty context | ⚠️ Tested indirectly |

---

## Domain 4: Temporal Reasoning

**Scope:** `changes_since`, `time_travel/trace`, `time_travel/summary`, `conflict_radar`, `causal_recall`. The temporal intelligence layer.

**Current coverage:** Good — dedicated tests for each endpoint.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| TR-01 | `changes_since` reports episodes added in window | Automated | Write episodes, query window, assert correct count | ✅ Tested |
| TR-02 | `changes_since` rejects invalid window (to <= from) | Automated | Send `to` before `from`, assert 400 | ✅ Tested |
| TR-03 | `changes_since` includes `request_id` when source writes had it | Automated | Write with custom request ID, verify it appears in changes | ✅ Tested |
| TR-04 | `time_travel/trace` reports fact shifts between timestamps | Automated | Write contradicting facts, trace, assert shifts | ✅ Tested |
| TR-05 | `time_travel/trace` rejects invalid window | Automated | Send `to` before `from`, assert 400 | ✅ Tested |
| TR-06 | `time_travel/summary` returns delta counts | Automated | Write facts, query summary, assert non-zero counts | ✅ Tested |
| TR-07 | `conflict_radar` detects active contradictions | Automated | Write contradicting facts, assert conflicts found | ✅ Tested |
| TR-08 | `causal_recall` returns fact lineage | Automated | Write facts, causal recall, assert non-empty chains | ✅ Tested |
| TR-09 | `causal_recall` rejects empty query | Automated | Send empty query, assert 400 | ✅ Tested |
| TR-10 | Temporal intent auto-resolution (`current` vs `historical`) | Automated | Query with "currently" vs "back in 2024", assert different `time_intent` in diagnostics | ✅ Tested |
| TR-11 | `temporal_weight` parameter influences scoring | Automated | Same query with different weights, assert rank order change | ⚠️ Tested indirectly |
| TR-12 | `as_of` parameter selects point-in-time facts | Automated | Write fact, supersede it, query with `as_of` before supersession | ⚠️ Tested via contract tests |
| TR-13 | Eval harness: accuracy >= 95%, stale <= 5%, p95 <= 300ms | Automated (CI) | `python3 eval/temporal_eval.py`, assert quality budget | ✅ Enforced in CI |

---

## Domain 5: Webhook System

**Scope:** Registration, HMAC signing, delivery, retry/backoff, dead-letter, replay, audit, rate limiting, circuit breaker, persistence.

**Current coverage:** Strong — 10+ dedicated tests. Gap: persistence with `persistence_enabled: true`.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| WH-01 | Webhook registration succeeds and returns ID | Automated | Register webhook, assert webhook ID returned | ✅ Tested |
| WH-02 | HMAC signature is correct (`t=<ts>,v1=<hex>` over `<ts>.<body>`) | Automated | Register with signing_secret, capture outbound, verify HMAC | ✅ Tested |
| WH-03 | `head_advanced` event fires after memory write | Automated | Register webhook, write memory, assert event captured | ✅ Tested |
| WH-04 | `conflict_detected` event fires on contradiction | Automated | Register webhook, write contradicting facts, assert event | ✅ Tested |
| WH-05 | Retry with exponential backoff on non-2xx | Automated | Point webhook at failing endpoint, verify retry count and timing | ✅ Tested |
| WH-06 | Dead-letter after `max_attempts` exhausted | Automated | Fail all retries, verify `dead_letter=true` | ✅ Tested |
| WH-07 | Dead-letter listing endpoint works | Automated | Create dead-letter events, list, assert present | ✅ Tested |
| WH-08 | Manual retry succeeds after sink recovers | Automated | Fail, fix sink, retry, assert delivered | ✅ Tested |
| WH-09 | Replay cursor pagination (chronological, sparse IDs) | Automated | Create events, replay with `after_event_id`, verify ordering | ✅ Tested |
| WH-10 | Audit log records registration, retry, dead-letter transitions | Automated | Perform lifecycle, query audit, assert entries | ✅ Tested |
| WH-11 | Stats endpoint returns delivery metrics | Automated | Generate events, query stats, assert counts | ✅ Tested |
| WH-12 | Domain allowlist blocks disallowed webhook target | Automated | Set policy allowlist, register webhook outside domain, assert rejection | ✅ Tested |
| WH-13 | Rate limiting protects downstream | Automated | Send burst exceeding `rate_limit_per_minute`, assert throttled | ✅ Tested (`wh13_rate_limiting_throttles_excess_deliveries`) |
| WH-14 | Circuit breaker opens after threshold failures | Automated | Fail `circuit_breaker_threshold` times, assert circuit open | ✅ Tested (`wh14_circuit_breaker_opens_after_threshold_failures`) |
| WH-15 | **Persistence: subscriptions/events survive restart** | Automated | Register webhook, create events, restart server, verify state intact | ❌ Not tested (all tests run with `persistence_enabled: false`) |
| WH-16 | `force=true` on retry re-delivers already-delivered events | Automated | Deliver event, retry with `force=true`, assert re-delivered | ⚠️ Tested via API, delivery not verified |

---

## Domain 6: Governance & Policies

**Scope:** `GET/PUT /api/v1/policies/:user`, preview, audit, violations. Retention enforcement, domain allowlists, default contract/policy fallback.

**Current coverage:** Strong — dedicated 4x4 contract/policy matrix test.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| GOV-01 | Policy CRUD works (get, upsert) | Automated | Create policy, get, verify fields match | ✅ Tested |
| GOV-02 | Policy preview estimates impact without applying | Automated | Preview retention change, verify estimate without mutation | ✅ Tested |
| GOV-03 | Policy audit records all policy changes | Automated | Change policy, query audit, assert entry | ✅ Tested |
| GOV-04 | Violations endpoint filters by time window | Automated | Create violations, query with window, assert correct subset | ✅ Tested |
| GOV-05 | Violations endpoint rejects invalid window | Automated | Send `to` before `from`, assert 400 | ✅ Tested |
| GOV-06 | Default memory contract applies when request omits it | Automated | Set policy with default contract, query without contract, assert policy default used | ✅ Tested |
| GOV-07 | Default retrieval policy applies when request omits it | Automated | Set policy with default retrieval_policy, query without it, assert default used | ✅ Tested |
| GOV-08 | Retention enforcement blocks stale episode writes | Automated | Set retention_days, write with timestamp older than retention, assert rejection | ✅ Tested |
| GOV-09 | `entity_deleted` and `edge_deleted` emit governance audit events | Automated | Delete entity, delete edge, query audit, assert events | ⚠️ Claimed in CHANGELOG, not explicitly verified |
| GOV-10 | Session deletion emits governance audit | Automated | Delete session, query audit, assert `session_deleted` event | ✅ Tested |

---

## Domain 7: Agent Identity Substrate

**Scope:** Identity CRUD, versioning, rollback, experience events, promotion workflow, contamination guards, identity-aware context.

**Current coverage:** Strong — dedicated adversarial tests.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| AIS-01 | First `GET /agents/:id/identity` auto-creates default profile | Automated | GET new agent, assert profile returned | ✅ Tested |
| AIS-02 | `PUT /agents/:id/identity` updates core | Automated | Update fields, GET, verify changed | ✅ Tested |
| AIS-03 | Version history is append-only | Automated | Update 3 times, list versions, assert 3+ entries | ✅ Tested |
| AIS-04 | Rollback restores prior version | Automated | Update, rollback, GET, verify old values | ✅ Tested |
| AIS-05 | Rollback preserves audit trail | Automated | Rollback, query audit, assert `rollback` event | ✅ Tested |
| AIS-06 | Contamination guard: user memory cannot write to identity_core | Automated | Attempt to inject user fact into core, assert rejection | ✅ Tested |
| AIS-07 | Drift resistance: repeated adversarial mutations blocked | Automated | Send repeated adversarial updates, assert protected fields unchanged | ✅ Tested |
| AIS-08 | Promotion requires >= 3 source_event_ids | Automated | Create proposal with < 3 events, assert 400 | ✅ Tested |
| AIS-09 | Approve promotion applies candidate_core | Automated | Create and approve proposal, verify core updated | ✅ Tested |
| AIS-10 | Reject promotion leaves core unchanged | Automated | Create and reject proposal, verify core unchanged | ✅ Tested |
| AIS-11 | Identity-aware context includes diagnostics | Automated | Add experience, query context, assert `identity_version`, `experience_events_used` in response | ✅ Tested |
| AIS-12 | `models/agent.rs` has unit tests | Automated | Write unit tests for Agent model construction, serialization, validation | ❌ No unit tests |

---

## Domain 8: Chat History Import

**Scope:** `POST /api/v1/import/chat-history`, `GET /api/v1/import/jobs/:job_id`. NDJSON, ChatGPT export, Gemini export. Dry run, idempotency.

**Current coverage:** Strong — 7 dedicated tests.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| IMP-01 | NDJSON import creates episodes | Automated | Import, list episodes, assert count matches | ✅ Tested |
| IMP-02 | ChatGPT export import works | Automated | Import ChatGPT format, verify episodes | ✅ Tested |
| IMP-03 | Gemini export import works | Automated | Import Gemini format, verify episodes | ✅ Tested |
| IMP-04 | Dry run validates without writing | Automated | Import with `dry_run: true`, list episodes, assert 0 | ✅ Tested |
| IMP-05 | Idempotency key prevents duplicate replay | Automated | Import twice with same key, assert same job_id, no new episodes | ✅ Tested |
| IMP-06 | Malformed rows are rejected with error details | Automated | Import with invalid JSON, assert errors in job status | ✅ Tested |
| IMP-07 | Mixed timestamp quality handled gracefully | Automated | Import with some rows missing `created_at`, assert success | ✅ Tested |
| IMP-08 | Import stress: >8000 messages completes without failure | Manual | Run `eval/import_stress.py`, assert 0 failures | ✅ Tested (eval harness) |
| IMP-09 | Duplicate user+idempotency_key returns HTTP 200 (not 202) | Automated | Import, re-import with same key, assert 200 | ⚠️ Claimed in API.md, not explicitly tested |

---

## Domain 9: Raw Vector API

**Scope:** 6 endpoints for upsert, query, delete, count, namespace lifecycle. Used by AnythingLLM integration.

**Current coverage:** Tested via Python test (`integrations/anythingllm/test_api.py`, 39 assertions).

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| VEC-01 | Upsert creates namespace automatically | Automated | Upsert to new namespace, count, assert > 0 | ✅ Tested |
| VEC-02 | Upsert with existing ID overwrites (idempotent) | Automated | Upsert same ID twice, count, assert count unchanged | ✅ Tested |
| VEC-03 | Query returns semantically similar results | Automated | Upsert vectors, query with similar vector, assert results | ✅ Tested |
| VEC-04 | Query non-existent namespace returns empty (not error) | Automated | Query fake namespace, assert empty results array | ✅ Tested |
| VEC-05 | Delete IDs removes vectors | Automated | Upsert, delete by ID, count, assert decreased | ✅ Tested |
| VEC-06 | Delete non-existent IDs is no-op | Automated | Delete fake IDs, assert no error | ✅ Tested |
| VEC-07 | Delete namespace removes all vectors | Automated | Upsert, delete namespace, count, assert 0 | ✅ Tested |
| VEC-08 | Delete non-existent namespace is no-op | Automated | Delete fake namespace, assert no error | ✅ Tested |
| VEC-09 | Count returns 0 for non-existent namespace | Automated | Count fake namespace, assert 0 | ✅ Tested |
| VEC-10 | Exists returns true/false correctly | Automated | Check non-existent (false), create, check (true) | ✅ Tested |
| VEC-11 | Namespaces isolated from Mnemo internal collections | Automated | Upsert to raw namespace, verify no contamination of entity/edge/episode collections | ⚠️ Claimed, not explicitly tested |
| VEC-12 | Raw Vector API tests run from Rust (not just Python) | Automated | Write Rust integration tests for all 6 endpoints | ❌ No Rust tests |

---

## Domain 10: Session Messages API

**Scope:** `GET/DELETE /api/v1/sessions/:id/messages`, `DELETE .../messages/:idx`. Adapter primitive for LangChain/LlamaIndex.

**Current coverage:** Python test (`tests/test_messages_api.py`, 12 scenarios). No Rust tests.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| MSG-01 | GET messages returns chronological order | Automated | Add episodes, list messages, assert ascending `created_at` | ✅ Tested (Python) |
| MSG-02 | DELETE by index removes correct message | Automated | Add 3 messages, delete index 1, verify middle message gone | ✅ Tested (Python) |
| MSG-03 | DELETE by out-of-bounds index returns 400 | Automated | Delete index 999, assert 400 | ✅ Tested (Python) |
| MSG-04 | DELETE all messages clears without deleting session | Automated | Clear messages, get session, assert session exists but messages empty | ✅ Tested (Python) |
| MSG-05 | Pagination with `limit` works | Automated | Add 5 messages, list with `limit=2`, assert 2 returned | ✅ Tested (Python) |
| MSG-06 | **Error code for out-of-bounds: 400 or 404?** | Audit | API.md says 400 (`validation_error`), SDK PRD says 404. Determine which is correct. | ❌ Inconsistency — must resolve |
| MSG-07 | Session Messages API has Rust integration tests | Automated | Write Rust tests in `memory_api.rs` for all 3 endpoints | ❌ No Rust tests |

---

## Domain 11: Operator Endpoints

**Scope:** `/api/v1/ops/summary`, `/api/v1/traces/:request_id`. Operator drill automation.

**Current coverage:** Good — dedicated tests.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| OPS-01 | Ops summary returns operator counters | Automated | Generate activity, query summary, assert non-zero counters | ✅ Tested |
| OPS-02 | `window_seconds` parameter bounds respected (max 86400) | Automated | Send `window_seconds=100000`, assert 400 or clamped | ❌ Not tested |
| OPS-03 | Trace lookup joins episode/webhook/governance records | Automated | Generate cross-pipeline activity, trace by request_id, assert joined records | ✅ Tested |
| OPS-04 | Trace lookup supports source filters | Automated | Trace with `include_episodes=true, include_webhook_events=false`, assert only episodes returned | ✅ Tested |
| OPS-05 | Trace lookup rejects invalid window | Automated | Send `to` before `from`, assert 400 | ✅ Tested |
| OPS-06 | Operator drill runner passes all 3 drills | Automated | `bash tests/operator_p0_drills.sh`, assert exit code 0 | ✅ Tested (CI) |

---

## Domain 12: Graph Engine

**Scope:** `mnemo-graph` crate. BFS traversal, community detection, summarization. `/api/v1/entities/:id/subgraph`.

**Current coverage:** **ZERO tests.** This is the highest-severity gap.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| GR-01 | BFS traversal returns correct subgraph at depth=1 | Automated | Create entities with known edges, traverse from seed, assert correct neighbors | ❌ No tests |
| GR-02 | BFS traversal respects `max_nodes` limit | Automated | Create large graph, traverse with `max_nodes=5`, assert <= 5 nodes | ❌ No tests |
| GR-03 | BFS traversal respects `depth` parameter | Automated | Create chain of 5 entities, traverse with depth=2, assert only 2-hop neighbors | ❌ No tests |
| GR-04 | Graph-traversed results receive 0.8x relevance discount | Automated | Verify scoring in retrieval pipeline, assert discount applied | ❌ No tests |
| GR-05 | Community detection produces non-trivial partitions | Automated | Create graph with 2 clusters, run community detection, assert 2 communities | ❌ No tests |
| GR-06 | `/api/v1/entities/:id/subgraph` endpoint returns valid response | Automated | Create entity with edges, GET subgraph, assert JSON structure | ❌ No tests (only tested indirectly via E2E) |
| GR-07 | Subgraph endpoint handles non-existent entity | Automated | GET subgraph for fake entity, assert 404 | ❌ No tests |

**Priority:** CRITICAL. Graph engine is in the hot path for context assembly. Must be falsified.

---

## Domain 13: LLM Providers

**Scope:** `mnemo-llm` crate. OpenAI-compatible, Anthropic, Ollama, Liquid AI providers. Prompt construction, response parsing, error handling.

**Current coverage:** **ZERO tests.** Second highest-severity gap.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| LLM-01 | OpenAI-compatible provider constructs valid prompts | Automated | Call extraction with known input, assert prompt structure | ❌ No tests |
| LLM-02 | Anthropic provider constructs valid prompts | Automated | Call extraction with known input, assert prompt structure | ❌ No tests |
| LLM-03 | Provider handles malformed LLM response gracefully | Automated | Mock LLM returning invalid JSON, assert error not panic | ❌ No tests |
| LLM-04 | Provider handles rate limit (429) with `retry_after_ms` | Automated | Mock 429 response, assert rate limit error propagated | ❌ No tests |
| LLM-05 | Provider handles token limit exceeded | Automated | Send oversized input, assert graceful error | ❌ No tests |
| LLM-06 | `provider: none` skips extraction without error | Automated | Configure `none` provider, write memory, assert episode stored but no entities/edges | ⚠️ Tested indirectly in E2E smoke |
| LLM-07 | Embedding dimension mismatch detected and reported | Automated | Configure 768-dim embedder against 1536-dim collection, assert clear error | ❌ No tests |

**Priority:** HIGH. LLM providers are in the write-path critical chain. Failures here corrupt graph state silently.

---

## Domain 14: Storage Layer

**Scope:** `mnemo-storage` crate. `RedisStateStore`, `QdrantVectorStore`, `RediSearch`.

**Current coverage:** 6 integration tests for Redis, 2 unit tests for RediSearch. **Zero dedicated Qdrant tests.**

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| ST-01 | User CRUD lifecycle | Automated | Create, read, update, delete user in Redis | ✅ Tested |
| ST-02 | Session CRUD lifecycle | Automated | Create, read, update, delete session | ✅ Tested |
| ST-03 | Episode pending queue with atomic claiming | Automated | Push to queue, claim with `ZREM`, verify no double-process | ✅ Tested |
| ST-04 | Entity dedup by name (case-insensitive) | Automated | Create `Acme` and `acme`, verify single entity | ✅ Tested |
| ST-05 | Entity dedup cross-user isolation | Automated | Create `Acme` for user A and B, verify separate entities | ✅ Tested |
| ST-06 | Edge conflict detection | Automated | Create conflicting edges, assert detection | ✅ Tested |
| ST-07 | Episode requeue after failure | Automated | Fail episode, requeue, assert back in pending set | ✅ Tested |
| ST-08 | **Qdrant upsert + search roundtrip** | Automated | Upsert vector, search by similar vector, assert hit | ❌ No dedicated test |
| ST-09 | **Qdrant tenant isolation (`user_id` filter)** | Automated | Upsert for user A and B, search as A, assert only A's results | ❌ No dedicated test |
| ST-10 | **Qdrant `delete_user_vectors` removes all collections** | Automated | Upsert across all 3 collections, delete, verify empty | ❌ No dedicated test |
| ST-11 | **Fulltext search returns relevant results** | Automated | Store episodes with known text, fulltext search, assert matches | ❌ No dedicated test |
| ST-12 | Session listing with `since` filter | Automated | Create sessions at different times, list with `since`, verify filter | ❌ No dedicated test |
| ST-13 | RediSearch query escaping handles special characters | Automated | Search with `@#$%` in query, assert no crash | ✅ Tested (unit) |
| ST-14 | Redis connection failure produces clear error (not panic) | Automated | Start with invalid `MNEMO_REDIS_URL`, assert error message | ❌ No test |
| ST-15 | Qdrant connection failure produces clear error | Automated | Start with invalid `MNEMO_QDRANT_URL`, assert error message | ❌ No test |

**Priority:** HIGH for Qdrant gaps. Qdrant is in the read-path critical chain.

---

## Domain 15: Retrieval Engine

**Scope:** `mnemo-retrieval` crate. RRF/MMR reranking, temporal scoring, metadata prefilter.

**Current coverage:** 6 unit tests. Tested indirectly via `memory_api.rs`.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| RET-01 | RRF merge produces correct composite scores | Automated | Merge known score lists, assert expected ranking | ✅ Tested |
| RET-02 | RRF with overlapping sources boosts rank | Automated | Submit overlapping results, assert score increase | ✅ Tested |
| RET-03 | Temporal intent resolves from query text | Automated | Test "currently" -> current, "back in 2024" -> historical | ✅ Tested |
| RET-04 | Temporal scoring prefers current facts for current intent | Automated | Score current vs superseded facts, assert current ranked higher | ✅ Tested |
| RET-05 | Diagnostics emitted with temporal scoring | Automated | Run scoring, assert diagnostics present | ✅ Tested |
| RET-06 | Metadata prefilter restricts candidate set | Automated | Query with metadata filters, assert reduced candidates in diagnostics | ✅ Tested (via memory_api) |
| RET-07 | `metadata_relax_if_empty` expands search when filters match nothing | Automated | Filter on non-existent value, assert relaxation in diagnostics | ✅ Tested |
| RET-08 | MMR reranker produces diverse results | Automated | Feed similar results, assert MMR increases diversity | ⚠️ **MMR not implemented** — only RRF exists. 5 RRF diversity tests added. Config mentions MMR as future option but no code branch exists. |
| RET-09 | Token budget is respected in context assembly | Automated | Set `max_tokens=50`, assert assembled context within budget | ⚠️ Tested via unit test, not integration |

---

## Domain 16: Python SDK

**Scope:** `mnemo-client` package. Sync client (`Mnemo`), async client (`AsyncMnemo`), transport, models, errors.

**Current coverage:** `test_sdk.py` covers sync client and structural checks. **Async client untested.**

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| SDK-01 | `pip install` works from git URL | Manual | `pip install git+...#subdirectory=sdk/python`, assert no errors | ✅ Tested |
| SDK-02 | `from mnemo import Mnemo` works | Automated | Import, assert no error | ✅ Tested |
| SDK-03 | `Mnemo.health()` returns status and version | Automated | Call health, assert fields | ✅ Tested |
| SDK-04 | `Mnemo.add()` and `Mnemo.context()` roundtrip | Automated | Add memory, recall, assert text present | ✅ Tested |
| SDK-05 | `Mnemo.get_messages()`, `delete_message()`, `clear_messages()` | Automated | Full message lifecycle test | ✅ Tested |
| SDK-06 | Error hierarchy: `MnemoHttpError`, `MnemoNotFoundError`, `MnemoValidationError` | Automated | Trigger 400 and 404, assert correct exception types | ✅ Tested |
| SDK-07 | All 27 methods exist on `Mnemo` class | Automated | Assert method existence via `hasattr` | ⚠️ Partially tested via package export check |
| SDK-08 | **`AsyncMnemo` roundtrip works** | Automated | Run async health/add/context, assert correct results | ❌ Not tested |
| SDK-09 | **`AsyncMnemo` all 27 methods exist** | Automated | Assert method existence | ❌ Not tested |
| SDK-10 | **`AsyncMnemo` context manager works** | Automated | `async with AsyncMnemo(...) as client:` pattern works | ❌ Not tested |
| SDK-11 | Transport retries on transient failure | Automated | Mock server returning 503 then 200, assert retry succeeds | ❌ Not tested |
| SDK-12 | `x-mnemo-request-id` propagated in SDK requests | Automated | Set custom request ID on client, verify in response | ⚠️ Claimed, not explicitly tested |
| SDK-13 | SDK version matches `Cargo.toml` version | Automated | Read both files, assert match | ❌ Not tested |

---

## Domain 17: Framework Adapters

**Scope:** `mnemo.ext.langchain.MnemoChatMessageHistory`, `mnemo.ext.llamaindex.MnemoChatStore`.

**Current coverage:** Structural tests always run. Functional tests require optional deps (skipped if not installed).

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| LC-01 | `MnemoChatMessageHistory` inherits `BaseChatMessageHistory` | Automated | Import, assert `issubclass` | ✅ Tested (structural) |
| LC-02 | `add_user_message` + `add_ai_message` + `messages` property | Automated | Add messages, read, assert correct types and content | ✅ Tested (functional, when langchain installed) |
| LC-03 | `clear()` removes all messages | Automated | Add messages, clear, assert empty | ✅ Tested (functional) |
| LC-04 | Session name auto-mapped to session UUID | Automated | Use named session, verify UUID resolution | ✅ Tested (functional) |
| LI-01 | `MnemoChatStore` inherits `BaseChatStore` | Automated | Import, assert `issubclass` | ✅ Tested (structural) |
| LI-02 | All 7 abstract methods implemented | Automated | Call each method, assert no `NotImplementedError` | ✅ Tested (functional, when llamaindex installed) |
| LI-03 | `get_keys()` returns list of session keys | Automated | Add messages to multiple sessions, get_keys, assert all present | ⚠️ Tested (functional) |
| LI-04 | Async variants work (`aget_messages`, `aadd_messages`, `aclear`) | Automated | Call async methods, assert correct behavior | ❌ Not tested |

---

## Domain 18: AnythingLLM Integration

**Scope:** `integrations/anythingllm/index.js`. VectorDatabase subclass for AnythingLLM.

**Current coverage:** Python API test (39 assertions) + Node.js provider test (230 lines).

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| ALM-01 | Provider `connect()` and `heartbeat()` work | Automated | Instantiate provider, call heartbeat, assert true | ✅ Tested (JS) |
| ALM-02 | `addDocumentToNamespace` stores vectors | Automated | Add document, query, assert found | ✅ Tested (JS) |
| ALM-03 | `performSimilaritySearch` returns relevant results | Automated | Add vectors, search, assert results | ✅ Tested (JS) |
| ALM-04 | `deleteDocumentFromNamespace` removes specific docs | Automated | Add, delete, verify removed | ✅ Tested (JS) |
| ALM-05 | `deleteVectorsInNamespace` clears entire namespace | Automated | Add, delete all, count, assert 0 | ✅ Tested (JS) |
| ALM-06 | Provider works when copied into actual AnythingLLM installation | Manual | Follow README instructions, verify in AnythingLLM UI | ❌ Not tested |

---

## Domain 19: Configuration & Startup

**Scope:** `config/default.toml`, environment variable overrides, server startup behavior.

**Current coverage:** **ZERO tests.** Config parsing is entirely untested.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| CFG-01 | `config/default.toml` parses without error | Automated | Load config, assert all sections present | ❌ No tests |
| CFG-02 | Environment variable overrides work (`MNEMO_SERVER_PORT`, etc.) | Automated | Set env vars, start server, verify behavior | ❌ No tests |
| CFG-03 | All env vars in README table are actually read by the server | Automated | For each documented env var, set it, start server, verify effect | ❌ No tests |
| CFG-04 | Invalid config values produce clear error messages | Automated | Set `MNEMO_SERVER_PORT=abc`, start server, assert error | ❌ No tests |
| CFG-05 | Missing required config with no default produces clear error | Automated | Unset `MNEMO_REDIS_URL`, `MNEMO_QDRANT_URL`, verify error | ❌ No tests |
| CFG-06 | `MNEMO_AUTH_ENABLED=true` without `MNEMO_AUTH_API_KEYS` produces error | Automated | Enable auth, provide no keys, verify behavior | ❌ No tests |

**Priority:** MEDIUM. Misconfiguration is the #1 cause of deployment failures.

---

## Domain 20: Auth Middleware

**Scope:** API key authentication via Bearer token or `X-API-Key` header.

**Current coverage:** 6 unit tests in `middleware/auth.rs`.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| AUTH-01 | Auth disabled: all requests pass | Automated | Disable auth, request without key, assert 200 | ✅ Tested |
| AUTH-02 | Auth enabled: missing key returns 401 | Automated | Enable auth, request without key, assert 401 | ✅ Tested |
| AUTH-03 | Auth enabled: wrong key returns 401 | Automated | Enable auth, send wrong key, assert 401 | ⚠️ Not explicitly tested (only missing key tested) |
| AUTH-04 | `/health` and `/metrics` bypass auth | Automated | Enable auth, request health without key, assert 200 | ✅ Tested |
| AUTH-05 | Bearer Authorization header accepted | Automated | Send `Authorization: Bearer <key>`, assert 200 | ✅ Tested |
| AUTH-06 | `X-API-Key` header accepted | Automated | Send `X-API-Key: <key>`, assert 200 | ✅ Tested |
| AUTH-07 | Auth integration test with real server | Automated | Start server with `MNEMO_AUTH_ENABLED=true`, test from client | ❌ No integration test |

---

## Domain 21: CI/CD Pipelines

**Scope:** 6 GitHub Actions workflows. Must all function correctly.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| CI-01 | `quality-gates.yml` runs all documented checks | Audit | Read workflow, compare to README quality gate list, assert match | ⚠️ Must verify |
| CI-02 | `memory-falsification.yml` runs all 56 integration tests | Audit | Read workflow, verify `cargo test -p mnemo-server --test memory_api` | ✅ Verified |
| CI-03 | `benchmark-eval.yml` enforces quality budget | Audit | Read workflow, verify accuracy >= 95%, stale <= 5%, p95 <= 300ms | ✅ Verified |
| CI-04 | `nightly-soak.yml` runs 3x flake detection | Audit | Read workflow, verify 3 repetitions | ✅ Verified |
| CI-05 | `package-ghcr.yml` builds and pushes Docker image | Audit | Check GHCR for published images | ⚠️ Must verify |
| CI-06 | `release.yml` creates GitHub Release with artifacts | Audit | Check GitHub Releases for correct artifacts | ⚠️ Must verify |
| CI-07 | All workflows succeed on current `main` branch | Automated | Check GitHub Actions status | ⚠️ Must verify |

---

## Domain 22: Docker & Packaging

**Scope:** Dockerfile, docker-compose files, GHCR images.

**Current coverage:** No automated tests for container behavior.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| DK-01 | `docker build .` succeeds | Automated | Build image, assert exit code 0 | ✅ Script (`tests/docker_build_test.sh`) |
| DK-02 | Built image size < 50MB | Automated | Build image, check size | ✅ Script (`tests/docker_build_test.sh`) |
| DK-03 | Container starts and responds to `/health` | Automated | `docker run`, curl health, assert ok | ✅ Script (`tests/docker_build_test.sh`) |
| DK-04 | `docker-compose.yml` starts full stack | Automated | `docker compose up -d`, wait, health check | ⚠️ Used in dev, not automated |
| DK-05 | `docker-compose.test.yml` starts with tmpfs volumes | Automated | `docker compose -f docker-compose.test.yml up`, verify ephemeral | ⚠️ Used in CI |
| DK-06 | Production compose uses named volumes for persistence | Audit | Read `docker-compose.prod.yml`, verify named volumes | ❌ Not audited |
| DK-07 | Managed compose connects to external Redis/Qdrant | Audit | Read `docker-compose.managed.yml`, verify no local Redis/Qdrant services | ❌ Not audited |

---

## Domain 23: Deployment Artifacts

**Scope:** All 10 deployment targets. File existence, structural correctness, guide accuracy.

**Current coverage:** All 10 targets were live-tested and falsified. Artifacts exist but structural correctness is not continuously verified.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| DEP-01 | All 86 deployment files exist | Automated | Glob all paths, assert existence | ⚠️ Spot-checked, not automated |
| DEP-02 | CloudFormation template validates | Automated | YAML parse + structural check (Resources, Parameters) | ✅ Script (`tests/deploy_artifact_validation.sh`) |
| DEP-03 | Terraform configs validate | Automated | `terraform validate` for all 4 targets | ✅ Script (`tests/deploy_artifact_validation.sh`) — all 4 pass |
| DEP-04 | Render blueprint validates | Automated | YAML parse + services key check | ✅ Script (`tests/deploy_artifact_validation.sh`) |
| DEP-05 | Railway config validates | Automated | JSON parse + structural check | ✅ Script (`tests/deploy_artifact_validation.sh`) |
| DEP-06 | Northflank stack validates | Automated | JSON parse + structural check | ✅ Script (`tests/deploy_artifact_validation.sh`) |
| DEP-07 | All DEPLOY.md files reference correct version (0.3.3) | Automated | Grep all DEPLOY.md for version, assert current | ❌ Not tested |
| DEP-08 | **Elestio artifacts cleanup** | Audit | CHANGELOG mentions `deploy/elestio/`, but T8 was changed to Vultr. Does `deploy/elestio/` still exist? If so, remove or document. | ❌ Must check |
| DEP-09 | **GCP Terraform state files not committed** | Audit | Check if `terraform.tfstate`, `.tfstate.backup`, `terraform.tfvars` are committed | ❌ Must check — flagged as risk |

---

## Domain 24: Documentation Consistency

**Scope:** Cross-document claims must agree. Stale claims must be corrected.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| DOC-01 | API.md health response version matches Cargo.toml | Audit | API.md shows `0.3.1`, Cargo.toml is `0.3.3` | ❌ STALE — must fix |
| DOC-02 | SDK PRD status matches README status | Audit | SDK PRD says "P0 active", README says SDK shipped | ❌ STALE — must fix |
| DOC-03 | Phase 2 PRD M4 status matches README | Audit | PRD says "in_progress", README checkmarks it complete | ❌ STALE — must fix |
| DOC-04 | DELETE messages/:idx error code consistent | Audit | API.md says 400, SDK PRD says 404 | ❌ INCONSISTENT — must resolve |
| DOC-05 | AnythingLLM star count current | Audit | README claims 55.5k stars | ⚠️ Point-in-time claim |
| DOC-06 | All README links resolve | Automated | Extract all links, verify 200/301 | ❌ Not tested |
| DOC-07 | All `docs/` files referenced in README table | Audit | Cross-check docs/ listing against README documentation table | ❌ Not checked |
| DOC-08 | CHANGELOG [Unreleased] section matches actual uncommitted changes | Audit | Compare CHANGELOG with git log | ❌ Not checked |

---

## Domain 25: Security & Credential Hygiene

**Scope:** No secrets committed. No credentials in tracked files.

### Falsification Gates

| ID | Gate | Method | How to Falsify | Status |
|----|------|--------|----------------|--------|
| SEC-01 | `.keys/` directory is gitignored | Automated | `git ls-files .keys/`, assert empty | ✅ Tested (`tests/credential_scan.sh`) |
| SEC-02 | Sensitive file patterns gitignored | Automated | `git ls-files *.pem credentials.json terraform.tfstate`, assert empty | ✅ Tested (`tests/credential_scan.sh`) |
| SEC-03 | Terraform state files not tracked | Automated | `git ls-files **/terraform.tfstate`, assert empty | ✅ Tested (`tests/credential_scan.sh`) |
| SEC-04 | No API keys, tokens, or passwords in tracked files | Automated | Scan tracked files for patterns: `sk-`, `rnd_`, `dop_v1_`, `AKIA`, etc. | ✅ Tested (`tests/credential_scan.sh`) |
| SEC-05 | `.env.example` contains no real values | Automated | Read all `.env.example` files, verify placeholder values only | ✅ Tested (`tests/credential_scan.sh`) |

---

## Appendix A: Known Inconsistencies

These were identified during the survey and must be resolved as part of this QA/QC pass.

| ID | Issue | Location | Resolution |
|----|-------|----------|------------|
| INC-01 | API.md health response shows version `0.3.1` but current is `0.3.3` | `docs/API.md` | ✅ FIXED — updated to `0.3.3` |
| INC-02 | SDK PRD status says "P0 active" but SDK is shipped | `docs/SDK_INTEGRATIONS_PRD.md` | ✅ FIXED — updated to "complete" |
| INC-03 | Phase 2 PRD M4 says "in_progress" but README marks complete | `docs/PHASE_2_PRD.md` | ✅ FIXED — updated M4 to "complete" |
| INC-04 | DELETE messages/:idx error code: API.md says 400, SDK PRD says 404 | `docs/API.md`, `docs/SDK_INTEGRATIONS_PRD.md` | ✅ FIXED — server returns 400 (`MnemoError::Validation`). SDK PRD corrected to match. |
| INC-05 | CHANGELOG mentions `deploy/elestio/` but T8 is Vultr | `CHANGELOG.md` | ✅ FIXED — updated to Vultr references |
| INC-06 | GCP Terraform state files appear to be committed | `deploy/gcp/terraform/` | ✅ FALSE ALARM — only `terraform.tfvars` tracked (project ID only, no secrets). State files are gitignored. |
| INC-07 | `zep_api.key` may be tracked | Repo root | ✅ FALSE ALARM — not tracked (`git ls-files` returns empty) |

---

## Appendix B: Coverage Gap Matrix

Severity: CRITICAL (data path, no tests), HIGH (data path, partial tests), MEDIUM (operational, no tests), LOW (cosmetic/docs).

| Domain | Current Tests | Gap Severity | Missing |
|--------|--------------|--------------|---------|
| Graph Engine (mnemo-graph) | 0 | CRITICAL | All: BFS, community detection, subgraph endpoint |
| LLM Providers (mnemo-llm) | 0 | HIGH | Prompt construction, response parsing, error handling |
| Qdrant Store | 0 dedicated | HIGH | Upsert/search roundtrip, tenant isolation, GDPR delete |
| Async SDK Client | 0 | HIGH | Full `AsyncMnemo` coverage |
| Webhook Persistence | 0 | HIGH | State survival across restart |
| Config Parsing | 0 | MEDIUM | TOML loading, env var overrides, validation |
| Docker Container | 0 | MEDIUM | Build, startup, healthcheck, size |
| Session Messages (Rust) | 0 | MEDIUM | All 3 endpoints in Rust integration tests |
| Raw Vector (Rust) | 0 | MEDIUM | All 6 endpoints in Rust integration tests |
| Auth Integration | 0 | MEDIUM | End-to-end auth with real server |
| Rate Limiting | 0 | LOW | Webhook rate limiting behavior |
| Circuit Breaker | 0 | LOW | Webhook circuit breaker behavior |

---

## Execution Plan

### Phase 1: Stop the Bleeding (Critical + High)
1. Write `mnemo-graph` unit tests (GR-01 through GR-07)
2. Write `mnemo-llm` unit tests with mock HTTP (LLM-01 through LLM-07)
3. Write dedicated Qdrant store integration tests (ST-08 through ST-10)
4. Write `AsyncMnemo` test coverage (SDK-08 through SDK-10)
5. Write webhook persistence test (WH-15)
6. Fix all known inconsistencies (INC-01 through INC-07)

### Phase 2: Harden (Medium)
7. Write config parsing tests (CFG-01 through CFG-06)
8. Write Session Messages Rust tests (MSG-07)
9. Write Raw Vector Rust tests (VEC-12)
10. Write Docker build/startup tests (DK-01 through DK-03)
11. Write auth integration test (AUTH-07)
12. Add request-id systematic test (API-01)
13. Audit and fix documentation consistency (DOC-01 through DOC-08)

### Phase 3: Polish (Low)
14. Write rate limiting test (WH-13)
15. Write circuit breaker test (WH-14)
16. Write MMR reranker test (RET-08)
17. Credential scan (SEC-01 through SEC-05)
18. Deployment artifact validation in CI (DEP-02 through DEP-06)

---

## Success Criteria

This PRD is complete when:

1. **Zero CRITICAL gaps** remain in the coverage matrix.
2. **Zero HIGH gaps** remain.
3. **All known inconsistencies** (Appendix A) are resolved.
4. **All security flags** (Domain 25) are verified clean.
5. **Every feature claimed as "shipped" in README** has at least one adversarial test that would catch its regression.

Until then, we are not done.
