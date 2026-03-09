# Mnemo P0 Roadmap

Six capability gaps that determine whether Mnemo wins the memory control plane category or stays a technically impressive niche tool. Ordered by execution priority.

---

## 1. Sleep-Time Compute

### What it is
A background consolidation pass that runs when the system is not actively serving user requests — proactively summarizing, merging, and re-ranking memory without waiting for a query to trigger it. Letta is the only competitor with this. It is a meaningful differentiation claim.

### Current state
`mnemo-ingest` has a continuous background worker (`crates/mnemo-ingest/src/lib.rs`) that polls for pending episodes and summarizes them. This is demand-driven — it only processes what was explicitly enqueued. There is no idle-triggered or scheduled consolidation pass.

### What needs to be built

**P0.1 — Idle-triggered consolidation scheduler**
- Add a configurable idle window (e.g. `MNEMO_SLEEP_IDLE_WINDOW_SECONDS`, default `300`).
- After N seconds with no ingest activity for a user, enqueue a consolidation job for that user.
- Consolidation pass: re-summarize older episodes, merge near-duplicate facts, promote high-signal edges.

**P0.2 — Proactive re-ranking**
- During idle windows, re-score entity/edge relevance using recency decay and access frequency.
- Write updated relevance scores back to Qdrant payload for faster retrieval on next query.

**P0.3 — Sleep-time summary generation**
- For each user with new episodes since last consolidation, generate a "memory digest" — a short summary of what changed.
- Store digest as a first-class memory artifact accessible via `/api/v1/memory/:user/digest`.
- Expose digest in operator dashboard as a "Recent Memory Activity" panel.

**P0.4 — Operator visibility**
- Add `GET /api/v1/ops/sleep_activity` endpoint showing consolidation job history per user.
- Surface last consolidation timestamp and digest in the operator dashboard incident/user lane.

### Success criteria
- Consolidation runs automatically after configurable idle window with zero user action.
- Memory digest is readable via API and visible in operator dashboard.
- No measurable latency regression on query path.
- Falsified by an integration test: write episodes, wait idle window, assert digest exists and entity scores updated.

### Existing foundation
- `crates/mnemo-ingest/src/lib.rs` — background worker, summarize trait, poll loop.
- `crates/mnemo-core/src/traits/llm.rs` — `summarize()` method already on `LlmProvider` trait.
- `crates/mnemo-graph/` — community detection and traversal primitives available.

---

## 2. Knowledge Graph (First-Class, Exposed)

### What it is
A traversable, queryable graph of entities and relationships extracted from memory — not just stored as embedding blobs. Zep and Weaviate have this. We have the plumbing but it is not wired to the public API or operator UX.

### Current state
`mnemo-graph` crate exists with graph traversal and community detection. `mnemo-storage` has full entity/edge CRUD in Redis with adjacency sets. `crates/mnemo-core/src/models/` has `Entity` and `Edge` types. None of this is exposed via public API routes or visible in the operator dashboard.

### What needs to be built

**P0.1 — Graph API routes**
- `GET /api/v1/graph/:user/entities` — list entities for a user with optional type/name filter.
- `GET /api/v1/graph/:user/entities/:id` — get entity with outgoing/incoming edges.
- `GET /api/v1/graph/:user/edges` — list edges with label/time filter.
- `GET /api/v1/graph/:user/neighbors/:entity_id` — 1-hop neighborhood traversal.
- `GET /api/v1/graph/:user/path?from=:id&to=:id` — shortest path between two entities.
- `GET /api/v1/graph/:user/community` — community clusters for the user's graph.

**P0.2 — Graph-aware retrieval**
- Add optional `include_graph` flag to `/api/v1/memory/:user/context`.
- When set, augment semantic retrieval results with graph-neighbor context for matched entities.
- This directly competes with Zep's Graph RAG claim.

**P0.3 — Graph drilldown in operator dashboard**
- Add a "Knowledge Graph" panel to the operator dashboard.
- Render user entity graph using D3 force layout (D3 already present).
- Nodes: entities colored by type. Edges: labeled relationships.
- Click entity -> see linked episodes, edges, and governance events.

**P0.4 — Graph evidence in trace view**
- Include entity/edge matches in `/api/v1/traces/:request_id` response.
- Surface graph nodes in Evidence Constellation alongside episode/webhook/governance nodes.

### Success criteria
- Graph API routes pass falsification tests covering entity CRUD, edge traversal, and neighbor queries.
- Graph-aware retrieval returns measurably richer context in integration test.
- Operator dashboard renders live user entity graph.
- Feature table entry flips from `⚠️` to `✅`.

### Existing foundation
- `crates/mnemo-graph/` — traversal, community detection, summarization.
- `crates/mnemo-storage/src/redis_store.rs` — full entity/edge CRUD, adjacency sets, name index.
- `crates/mnemo-storage/src/redisearch.rs` — full-text edge search already indexed.
- `crates/mnemo-core/src/models/` — `Entity`, `Edge`, `EdgeFilter` types.
- Dashboard already has D3 v7.

---

## 3. LLM Call Tracing (Span-Level)

### What it is
Capturing the full LLM call span for every extraction, summarization, and context-assembly call: prompt in, completion out, model, token counts (prompt + completion), latency ms, and error if any. We have request IDs and token estimates but not call-level spans. LangSmith wins this category today.

### Current state
- `x-mnemo-request-id` is propagated through routes and stored on audit rows and episodes.
- Token counts are estimated post-hoc via `estimate_tokens()` — not from actual API responses.
- No span capture for individual LLM calls. No prompt or completion logging.

### What needs to be built

**P0.1 — LLM span capture**
- Add `LlmSpan` struct: `span_id`, `request_id`, `call_type` (extract/summarize/embed), `model`, `prompt_tokens`, `completion_tokens`, `total_tokens`, `latency_ms`, `error`, `created_at`.
- Capture span on every `extract()`, `summarize()`, and `chat_completion()` call in `mnemo-llm`.
- Store spans in a Redis sorted set keyed by request_id and by user.

**P0.2 — Span API**
- `GET /api/v1/spans/:request_id` — all LLM spans for a request.
- `GET /api/v1/spans/user/:user_id?from=&to=&limit=` — span history for a user.
- Include span summary (total tokens, total latency, call count) alongside trace results in `/api/v1/traces/:request_id`.

**P0.3 — Operator dashboard integration**
- Surface LLM span summary in trace drilldown: total LLM calls, total tokens, total latency, any errors.
- Add span detail collapsible section showing per-call breakdown.
- Include span nodes in Evidence Constellation graph.

**P0.4 — Token cost estimation**
- Add configurable cost-per-token map (model -> $/1k tokens).
- Include estimated cost in span summary and in operator dashboard.
- Expose cost aggregate in `/api/v1/ops/summary`.

**P0.5 — OTel export (stretch)**
- Export spans in OpenTelemetry format to a configurable OTLP endpoint.
- This makes Mnemo pluggable into Jaeger, Grafana, Honeycomb, and Datadog pipelines.

### Success criteria
- Every LLM call produces a stored span with real token counts from API response.
- `GET /api/v1/spans/:request_id` returns all spans for a given request.
- Operator dashboard shows token cost and latency breakdown per request.
- Feature table entry flips from `⚠️` to `✅`.

### Existing foundation
- `crates/mnemo-llm/src/openai_compat.rs` — `chat_completion()`, `summarize()`, `extract()` all call the LLM API and already parse responses.
- `crates/mnemo-llm/src/anthropic.rs` — same.
- `x-mnemo-request-id` propagation already in place.
- Redis storage patterns already established.

---

## 4. SOC 2 Compliance Posture

### What it is
SOC 2 Type II certification covering Security, Availability, and Confidentiality. This is the enterprise procurement gate. Mem0, Zep, Pinecone, Weaviate, and LangSmith all have it. We do not.

### Current state
We have more compliance infrastructure than we are giving ourselves credit for:
- `GET /api/v1/audit/export` — unified governance + webhook audit log with SIEM-ready output, explicitly designed for auditors (`docs/API.md:68`).
- Governance audit rows with per-action timestamps, user IDs, request IDs, and change details.
- Webhook delivery audit with signed payloads and replay capability.
- Per-user memory policies with retention write guards.
- `x-mnemo-request-id` propagation for full request traceability.

What is missing is the posture documentation, controls mapping, and the operational requirements that underpin certification.

### What needs to be built

**P0.1 — Controls documentation**
- Write `docs/SECURITY_CONTROLS.md` mapping Mnemo features to SOC 2 Trust Service Criteria:
  - CC6 (Logical and Physical Access): API key auth, per-user scoping, policy enforcement.
  - CC7 (System Operations): audit export, webhook audit, governance audit, request tracing.
  - CC9 (Risk Mitigation): retention guards, policy violation tracking, dead-letter recovery.
  - A1 (Availability): webhook circuit breaker, health endpoint, Redis/Qdrant redundancy paths.
  - C1 (Confidentiality): per-user data isolation, namespace separation, policy-governed access.

**P0.2 — Encryption at rest**
- Document and enforce encryption-at-rest requirements for Redis and Qdrant deployments.
- Add deployment guide section for Redis with TLS + encrypted AOF/RDB.
- Add Qdrant deployment guide section for encrypted storage volumes.
- Add `MNEMO_REQUIRE_TLS=true` server config flag that rejects non-TLS upstream connections.

**P0.3 — Audit log hardening**
- Make audit export tamper-evident: add HMAC signature to each audit batch response.
- Add `GET /api/v1/audit/export/stream` for continuous SIEM ingestion.
- Add audit log retention policy enforcement: configurable minimum retention window.

**P0.4 — Access control hardening**
- Promote API key auth from optional to the default-on path (currently `WARN: API key auth DISABLED` in prod).
- Add scoped API keys: read-only, write, admin.
- Add key rotation endpoint.

**P0.5 — Compliance posture page**
- Public `SECURITY.md` and `docs/COMPLIANCE.md` suitable for sharing with enterprise security teams.
- Maps each SOC 2 criterion to the specific Mnemo API, config, and deployment controls that satisfy it.

### Success criteria
- `docs/SECURITY_CONTROLS.md` covers all five SOC 2 Trust Service Criteria with concrete feature/control mappings.
- `MNEMO_REQUIRE_TLS=true` enforced end-to-end.
- Audit export is HMAC-signed and streamable.
- API key auth is on by default with scoped key support.
- Ready to engage a SOC 2 auditor.

### Existing foundation
- `GET /api/v1/audit/export` already in production.
- Governance + webhook audit rows already timestamped and user-scoped.
- Per-user policy enforcement already in place.
- `x-mnemo-request-id` traceability already in place.

---

## 5. One-Line Install

### What it is
A developer should be able to go from zero to a running Mnemo instance in under two minutes, with a single command. Every competitor has this. We do not. This is the first impression gate for developer adoption.

### Current state
Getting Mnemo running requires: clone the repo, install Rust, install Redis Stack, install Qdrant, configure environment variables, and `cargo run`. That is six steps with three external dependencies before you can call the API.

### What needs to be built

**P0.1 — Docker Compose quick-start**
- `docker-compose.yml` at repo root that brings up Mnemo + Redis Stack + Qdrant with sane defaults.
- Single command: `docker compose up`.
- Includes a `mnemo-server` image built from the repo `Dockerfile`.
- Pre-configured with local embeddings (no API key needed for basic use).

**P0.2 — Published Docker image**
- Publish `ghcr.io/anjaustin/mnemo:latest` and `ghcr.io/anjaustin/mnemo:v{version}` via existing `package-ghcr.yml` workflow (already present, needs validation).
- Image should be self-contained: includes fastembed model cache warm-up on first start.

**P0.3 — curl quick-start**
- Add a `scripts/quickstart.sh` that: pulls the Docker Compose file, starts services, waits for health, and runs a sample `POST /api/v1/memory` + `POST /api/v1/memory/:user/context` round-trip.
- Output should be friendly and show the memory round-trip result in the terminal.

**P0.4 — README install section**
- Rewrite the README install section to lead with `docker compose up`.
- Include a copy-pasteable three-command sequence: pull, up, curl.
- Link to Python SDK and TypeScript SDK install as next step.

**P0.5 — Homebrew formula (stretch)**
- Publish a Homebrew tap for `mnemo-server` binary install on macOS/Linux.
- `brew install anjaustin/mnemo/mnemo-server`.

### Success criteria
- `docker compose up` from a clean machine starts a working Mnemo instance in under 2 minutes.
- A developer can run a memory round-trip with zero Rust toolchain knowledge.
- README leads with the one-command install.
- Falsified by a CI job that runs the Docker Compose quick-start and exercises the API.

### Existing foundation
- `deploy/` directory already has deployment config.
- `package-ghcr.yml` workflow exists for building and pushing Docker images.
- `tests/dashboard_smoke.sh` can be adapted as a quick-start validation script.

---

## 6. SDKs (Python + TypeScript)

### What it is
Published, installable, documented SDKs that let developers integrate Mnemo in one line. Every competitor has this. We have a Python SDK in `sdk/python/` but it is not published to PyPI. We have no TypeScript SDK.

### Current state
- `sdk/python/mnemo/` exists with sync client, async client, LangChain extension, LlamaIndex extension, and tests.
- Package name: `mnemo-client`, version `0.3.7`.
- Not published to PyPI. No `pip install mnemo-client` works today.
- No TypeScript/JavaScript SDK exists anywhere in the repo.

### What needs to be built

**P0.1 — Publish Python SDK to PyPI**
- Add GitHub Actions workflow: on tag `sdk/python/v*`, build and publish `mnemo-client` to PyPI.
- Add `__version__` to `mnemo/__init__.py`.
- Write a proper `sdk/python/README.md` with install + quickstart.
- Ensure LangChain and LlamaIndex integrations are documented with code examples.
- `pip install mnemo-client` must work.

**P0.2 — Python SDK completeness**
- Audit coverage against all public API endpoints.
- Add missing methods: graph API, spans API, sleep digest, audit export, evidence export.
- Add typed response models for all endpoints using `dataclasses` or `pydantic` (optional dep).
- Add `mnemo.memory()`, `mnemo.context()`, `mnemo.rca()` convenience wrappers.

**P0.3 — TypeScript SDK**
- Create `sdk/typescript/` with a `MnemoClient` class.
- Implement: `memory.add()`, `memory.context()`, `memory.rca()`, `sessions.*`, `policies.*`, `traces.*`.
- Zero runtime dependencies (use native `fetch`).
- Publish to npm as `@mnemo/client`.
- Add LangChain.js integration as optional export.

**P0.4 — SDK CI**
- Add Python SDK test job to `quality-gates.yml`: spin up Mnemo, run `sdk/python/tests/`.
- Add TypeScript SDK test job: spin up Mnemo, run `sdk/typescript/tests/`.
- Both jobs must pass before merge.

**P0.5 — Framework integration guides**
- `docs/integrations/langchain.md` — Python and JS.
- `docs/integrations/llamaindex.md`.
- `docs/integrations/openai-agents.md`.
- `docs/integrations/vercel-ai-sdk.md` (TypeScript).

### Success criteria
- `pip install mnemo-client` installs the published package.
- `npm install @mnemo/client` installs the published package.
- Both SDKs cover memory, context, sessions, traces, and graph endpoints.
- SDK CI jobs run on every PR.
- LangChain and LlamaIndex integrations are documented with working code examples.

### Existing foundation
- `sdk/python/mnemo/` — sync client, async client, LangChain ext, LlamaIndex ext, tests.
- `sdk/python/pyproject.toml` — package configured, just not published.
- No TypeScript SDK exists yet.

---

## Priority Order Summary

| # | Initiative | Competitive impact | Build effort | Existing foundation |
|---|---|---|---|---|
| 1 | Sleep-time compute | High — only Letta has it | Medium | Strong (ingest worker, summarize trait) |
| 2 | Knowledge graph (exposed) | High — Zep/Weaviate advantage closes | Medium | Strong (mnemo-graph, entity/edge storage) |
| 3 | LLM call tracing | High — LangSmith gap partially closes | Medium | Moderate (request IDs, token estimates) |
| 4 | SOC 2 compliance posture | Critical — enterprise gate | Medium-High | Moderate (audit export, governance, policy) |
| 5 | One-line install | Critical — developer adoption gate | Low-Medium | Moderate (deploy/, package-ghcr.yml) |
| 6 | SDKs (Python + TypeScript) | Critical — ecosystem gate | Medium | Strong for Python, zero for TypeScript |

## Feature Table Impact

After all six are shipped, the feature table changes:

| Feature | Before | After |
|---|---|---|
| Sleep-time / offline compute | `❌` | `✅` |
| Knowledge graph / entity relations | `⚠️` | `✅` |
| LLM call tracing / span visibility | `⚠️` | `✅` |
| SOC 2 / HIPAA compliance | `❌` | `✅` (SOC 2) / `⚠️` (HIPAA) |
| One-line / quick install | `❌` | `✅` |
| Python SDK | `❌` | `✅` |
| JS/TS SDK | `❌` | `✅` |
| LlamaIndex adapter | `❌` | `✅` (BaseChatStore, server-side keys, 36 tests) |
| Retrieval benchmarks published | `❌` | `⚠️` (partial, from graph/span data) |

Completing this roadmap closes the five most critical `❌` gaps and flips three `⚠️` entries to `✅`. The remaining moat gaps after that are HIPAA certification and benchmark publications — both achievable in a follow-on sprint.
