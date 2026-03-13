# Changelog

All notable changes to Mnemo will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **gRPC API** (`mnemo-proto`, `mnemo-server`): Client-facing gRPC endpoint served on the same port as REST via content-type routing. `MemoryService` (GetContext, CreateEpisode, ListEpisodes, DeleteEpisode), `EntityService` (ListEntities, GetEntity), `EdgeService` (QueryEdges, GetEdge). Proto schema at `proto/mnemo/v1/memory.proto`. New `mnemo-proto` crate (10th workspace crate) with `tonic-build` compilation and `FILE_DESCRIPTOR_SET` for server reflection. gRPC health check (`grpc.health.v1.Health/Check`) and reflection (`grpc.reflection.v1.ServerReflection`) services. Same-port multiplexing via `axum::Router::merge`. 11 integration tests. First-in-category: none of Zep, Mem0, or Letta offer gRPC.

### Changed

- Workspace crate count: 9 -> 10 (added `mnemo-proto`).
- `mnemo-server` description updated: "HTTP/REST and gRPC server for Mnemo".
- Total workspace test count: ~1,091 -> ~1,102 (+11 gRPC integration tests).
- `MnemoError` gRPC mapping uses `status_code()` method for correct classification of all error variants.

## [0.6.0] — 2026-03-13

### Added

- **Scoped API keys (RBAC)** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Role-based access control via API key management. `ApiKey` model with `Admin`, `Write`, `Read` roles and optional `ApiKeyScope` (user/agent restrictions). `ApiKeyStore` trait with Redis persistence — keys stored as salted SHA-256 hashes with `mnk_` prefix for identification. CRUD endpoints: `POST /api/v1/keys` (create), `GET /api/v1/keys` (list), `DELETE /api/v1/keys/:id` (revoke), `POST /api/v1/keys/:id/rotate` (rotate with automatic old-key revocation). `CallerContext` extracted by auth middleware for per-request role enforcement. `require_role()` guard on all mutating endpoints. 20 tests + adversarial falsification tests.
- **Data classification labels** (`mnemo-core`, `mnemo-server`): Four-tier classification system — `Public`, `Internal`, `Confidential`, `Restricted` — applied to entities and edges at ingestion time. `Classification` enum with ordinal ordering and `#[serde(default)]` backward compatibility (defaults to `Internal`). Classification filters in context assembly and retrieval. `CallerContext::max_classification()` enforces ceiling based on API key role. 12 tests.
- **Policy-scoped memory views** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Named, reusable access policies that filter memory by classification ceiling, entity type whitelist, edge label blacklist, and temporal scope. `MemoryView` model with `ViewConstraints` enforcement. `ViewStore` trait with Redis persistence (JSON + sorted set + name index). CRUD endpoints at `/api/v1/views`. Views applied at context assembly time via `?view=` query parameter. 14 tests + adversarial falsification tests.
- **Memory guardrails engine** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Rule-based policy engine for memory access control. `GuardrailRule` with composable `GuardrailCondition` predicates (classification thresholds, confidence floors, entity/edge type filters, content regex, caller role checks, age limits) and `GuardrailAction` outcomes (allow, block, redact, reclassify, audit). `GuardrailStore` trait with Redis persistence. CRUD endpoints at `/api/v1/guardrails` plus `POST /api/v1/guardrails/evaluate` for rule evaluation. Priority-ordered evaluation with short-circuit on block. 22 tests + adversarial falsification tests.
- **Agent identity phase B — governance & conflict handling** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Promotion proposals with approval workflows for agent identity changes. `PromotionProposal` with `Pending/Approved/Rejected/Expired` lifecycle, configurable `ApprovalPolicy` (quorum, auto-reject deadline, cooling period, risk-level thresholds). `ConflictAnalysis` engine scores experience evidence for/against proposed changes. Endpoints: `POST /api/v1/agents/:agent_id/promotions` (propose), `POST .../approve` / `POST .../reject`, `GET .../conflicts`, `PUT/GET /api/v1/agents/:agent_id/approval-policy`. 25 tests + adversarial falsification tests.
- **Multi-agent shared memory with ACLs** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Shared memory regions with granular access control. `MemoryRegion` model with owner agent, user scope, classification ceiling, entity/edge type filters. `MemoryRegionAcl` with `Read/Write/Manage` permissions and optional expiry. `RegionStore` trait with Redis persistence — atomic MULTI/EXEC pipelines, user-scoped indices, agent reverse indices, lazy expired ACL cleanup. CRUD endpoints at `/api/v1/regions` with `?user_id=` and `?agent_id=` filters. ACL management at `/api/v1/regions/:region_id/acl`. `validate_agent_id()` rejects path traversal, colons, slashes, unicode, control chars. Full RBAC + ownership enforcement on all endpoints. 36 model tests + 37 integration tests (17 functional + 20 red-team/adversarial).

### Changed

- Workspace version: `0.5.5` -> `0.6.0`.
- `mnemo-core` test count: 364 -> 478 (+114 tests).
- Total workspace test count: ~702 -> ~1,091 (+389 tests).
- `StateStore` composite trait now includes `ApiKeyStore`, `ViewStore`, `GuardrailStore`, `RegionStore`.
- `AppState` struct: added `auth_config` field for API key authentication middleware.
- Auth middleware: `AuthConfig` with key cache (`Arc<RwLock<HashMap<String, CachedKey>>>`), bootstrap key support, `CallerContext` extraction into request extensions.
- `MemoryWebhookEventType` enum expanded with governance events.
- All mutating endpoints now enforce `require_role()` authorization checks.
- `RegionStore::list_regions` accepts `user_id: Option<Uuid>` and `agent_id: Option<&str>` filter parameters.
- Region operations use atomic Redis pipelines (`MULTI/EXEC`) for create, delete, grant, and revoke.
- Expired ACLs lazily cleaned from agent reverse indices during `list_agent_accessible_regions`.

### Security

- Red-team round 1: Fixed 10 of 13 findings — RBAC on all region endpoints, ownership verification on grant/revoke/update, `validate_agent_id()` input sanitization, expired ACL filtering, user existence validation. 16 red-team integration tests.
- Red-team round 2: Resolved remaining 3 accepted risks — atomic Redis pipelines prevent partial writes, user-scoped region index prevents full-scan enumeration, lazy ACL cleanup removes stale index entries. 4 additional integration tests.

## [0.5.5] — 2026-03-11

### Added

- **Confidence decay + revalidation** (`mnemo-core`, `mnemo-server`): Facts and entity edges decay in confidence over time unless reinforced by new evidence. `effective_edge_confidence()` applies `confidence * corroboration_boost * decay_factor * importance_protection`. `compute_edge_fisher_importance()` scores edges using corroboration rarity and connectivity. `GET /api/v1/memory/:user/stale` returns facts below revalidation threshold, ranked by importance. `POST /api/v1/memory/:user/revalidate` resets decay clock. `revalidation_needed` webhook event. 17 tests + 12 adversarial falsification tests.
- **Self-healing memory** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Auto-detect low-confidence conflicts and generate targeted clarification questions. `ClarificationRequest` type with `Pending/Resolved/Expired/Dismissed` status lifecycle. `ClarificationStore` trait with Redis persistence (JSON + sorted set by severity). `POST /api/v1/memory/:user/clarifications` runs conflict radar and generates questions. `POST .../resolve` applies answer with optional winning edge invalidation. `POST .../dismiss` for irrelevant conflicts. `clarification_generated` and `clarification_resolved` webhook events. 14 tests + 10 adversarial falsification tests.
- **Cross-session narrative summaries** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Evolving "story of the user" narratives that update after each session. `UserNarrative` type with versioned chapters (`NarrativeChapter` with period, summary, key_changes). `NarrativeStore` trait with Redis persistence. `GET /api/v1/memory/:user/narrative` returns current narrative. `POST /api/v1/memory/:user/narrative/refresh` generates/updates via LLM with incremental or full rebuild. `include_narrative: true` in context requests prepends narrative as preamble. `narrative_refreshed` webhook event. 18 tests + 13 adversarial falsification tests.
- **Goal-conditioned memory** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Condition retrieval by active objective, not only semantic similarity. `GoalProfile` type with entity category boosts, edge label boosts, temporal bias, boost/suppress keywords. `GoalStore` trait with Redis persistence (JSON + sorted set + name index). CRUD endpoints at `/api/v1/memory/:user/goals`. `goal` parameter in context requests triggers relevance re-scoring via `compute_relevance_adjustment()`. `goal_applied` in context response. 18 tests + 12 adversarial falsification tests.
- **Counterfactual memory** (`mnemo-core`, `mnemo-server`): Simulate retrieval under hypothetical assumptions without modifying state. `POST /api/v1/memory/:user/counterfactual` accepts `hypotheticals` array of fact overrides. `apply_hypotheticals()` engine replaces matching facts (case-insensitive entity+label) and injects novel hypotheticals. `CounterfactualDiff` tracks overridden facts, injected count, and novel additions. Read-only COW simulation. 12 tests + 11 adversarial falsification tests.

### Changed

- `mnemo-core` test count: 227 -> 364 (+137 tests).
- Total workspace test count: ~553 -> ~702 (+149 tests).
- `FactSummary` now derives `PartialEq` (needed for counterfactual diff tracking).
- `StateStore` composite trait now includes `ClarificationStore`, `NarrativeStore`, `GoalStore`.
- `MemoryWebhookEventType` enum expanded: `RevalidationNeeded`, `ClarificationGenerated`, `ClarificationResolved`, `NarrativeRefreshed`.
- `MemoryContextRequest` extended with `include_narrative`, `goal` fields.
- `MemoryContextResponse` extended with `narrative`, `goal_applied` fields.

## [0.5.0] — 2026-03-11

### Added

- **GNN-enhanced retrieval re-ranking** (`mnemo-gnn`): New crate implementing a 3-layer graph neural network for entity re-ranking. Takes entity candidates with graph topology signals (edge count, depth, connectivity) and produces attention-weighted relevance scores. Integrated into the retrieval pipeline for context assembly. 14 unit tests + falsification.
- **SONA/EWC++ experience weight consolidation** (`mnemo-core`, `mnemo-server`): Elastic Weight Consolidation for agent experience events. `compute_fisher_importance()` scores each experience using novelty (inverse category saturation), corroboration (confidence-weight alignment), and weight magnitude. `effective_experience_weight_ewc()` applies the consolidation formula: `weight * confidence * decay * (1 + fisher_importance * EWC_LAMBDA)`. High-importance experiences resist temporal decay — synaptic consolidation for AI agents. 18 tests + falsification.
- **Temporal tensor compression** (`mnemo-retrieval`): Tiered quantization for aging episode embeddings — f32 (hot), f16 (warm), int8 (cool), binary (cold). Configurable age thresholds, error-bounded lossy compression, compression statistics tracking via `GET /api/v1/ops/compression`. 18 tests + falsification.
- **Coherence scoring endpoint** (`mnemo-retrieval`, `mnemo-server`): `POST /api/v1/memory/:user/coherence` computes a multi-signal coherence score for a user's knowledge graph — entity frequency variance, edge consistency, temporal coverage, structural connectedness. Returns a 0.0-1.0 score with per-signal breakdown. 25 tests + 8 adversarial falsification tests.
- **MCP server** (`mnemo-mcp`): New crate implementing Model Context Protocol over stdio transport. 7 tools (`mnemo_remember`, `mnemo_recall`, `mnemo_graph_query`, `mnemo_agent_identity`, `mnemo_digest`, `mnemo_coherence`, `mnemo_health`), 2 resource templates (`mnemo://users/{user}/memory`, `mnemo://agents/{agent_id}/identity`). JSON-RPC 2.0 compliant, path traversal protection, Claude Code integration config. 38 tests + 30 adversarial falsification tests.
- **Witness chain tamper-proof audit** (`mnemo-core`, `mnemo-server`): SHA-256 hash-chained audit trail for all agent identity mutations. Every `AgentIdentityAuditEvent` stores `prev_hash` and `event_hash` forming an append-only chain. `verify_audit_chain()` detects tampering, gaps, and forks. `GET /api/v1/agents/:agent_id/identity/audit/verify` endpoint. 17 tests + 6 adversarial falsification tests.
- **Semantic routing** (`mnemo-retrieval`, `mnemo-server`): Keyword-based query classifier that routes retrieval queries to optimal strategies — `Head`, `Hybrid`, `Historical`, `GraphFocused`, `EpisodeRecall`. Applied in 3 context endpoints (`/context`, `/causal_recall`, agent context). `RoutingDecision` included in `ContextBlock` for observability. 21 tests + 10 adversarial falsification tests.
- **Hyperbolic HNSW** (`mnemo-retrieval`): Poincare ball geometry for hierarchical entity re-ranking. `poincare_distance()`, `mobius_addition()`, `exp_map()`, `log_map()` operations. `HyperbolicHnswIndex` with insertion, k-nearest-neighbor search, and layer probability. Entities closer to the origin (more general) get boosted during retrieval. 29 tests + 11 adversarial falsification tests.
- **COW branching for agent identity** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): Copy-on-write branching for A/B testing agent personality changes. 6 endpoints: create/list/get/update/merge/delete branches. Branch agents use `{agent_id}:branch:{branch_name}` format. Merge brings validated changes back to main identity. 9 model tests + 8 adversarial falsification tests.
- **DAG workflows** (`mnemo-ingest`, `mnemo-server`): Typed pipeline formalization with 7 steps (ingest, extract, embed, index, enrich, summarize, consolidate). Per-step latency/throughput metrics, dead-letter queue with configurable capacity and exponential backoff. `GET /api/v1/ops/pipeline` endpoint. 24 tests + 10 adversarial falsification tests.
- **Delta consensus** (`mnemo-core`, `mnemo-server`): CRDT primitives for multi-node sync — `GCounter`, `LWWRegister`, `ORSet`, `LWWMap`. `VectorClock` for causal ordering, `HybridLogicalClock` for wall-clock + logical time, `MerkleDigest` for state fingerprinting, `DeltaEnvelope` for replication. `GET /api/v1/ops/sync` endpoint. `AgentIdentity` included as a `DeltaResourceType`. 54 tests + 12 adversarial falsification tests.
- **Domain expansion / agent fork** (`mnemo-core`, `mnemo-storage`, `mnemo-server`): `POST /api/v1/agents/:agent_id/fork` creates a new agent from an existing one with selective experience transfer. `ExperienceFilter` controls which events transfer (by category, min confidence, min weight, max count). `ForkLineage` preserves provenance. Identity core can be overridden at fork time. 14 model tests + 12 adversarial falsification tests.
- **Verified/proof-carrying identity updates** (`mnemo-core`, `mnemo-server`): `POST /api/v1/agents/:agent_id/identity/verified` accepts Merkle proof-carrying writes. `AllowlistMerkleTree` built from canonical identity allowlist (6 keys). Proposer generates `AllowlistMembershipProof` per key; server verifies proof against canonical root + forbidden-substring deep scan before accepting write. Proof stored in response for auditability. 21 tests + 12 adversarial falsification tests.

### Changed

- `mnemo-core` test count: 62 -> 227 (+165 tests).
- Total workspace test count: ~355 -> ~553 (+198 tests).
- `AgentStore` trait: 14 -> 21 methods (added `fork_agent` + 6 branch methods).
- `AppState` struct: added `compression_config`, `compression_stats`, `hyperbolic_config`, `pipeline_metrics`, `sync_status` fields.
- `docs/API.md`: 13 new endpoint sections documenting all v0.5.0 features.

## [0.4.0] — 2026-03-10

### Added

- **Proactive `fact_added` / `fact_superseded` webhook events** (`mnemo-ingest`, `mnemo-core`, `mnemo-server`): fact mutation events now fire proactively from the ingestion pipeline via a `tokio::mpsc` channel, rather than only reactively when a client calls `changes_since`. All four webhook event types (`head_advanced`, `conflict_detected`, `fact_added`, `fact_superseded`) are now emitted as mutations occur.
- **Falsification audit** (`mnemo-llm`, `mnemo-retrieval`, `mnemo-server`): 17-target comprehensive feature-matrix verification. 6 new tests added, 2 bugs fixed (GraphTraversal source tracking, SDK method gaps), 1 doc corrected. All 17 targets at PASS.
- **Documentation overhaul**: 8 undocumented API endpoints added to `docs/API.md` (operator incidents, evidence export bundles, LLM span tracing, memory digest). 5 missing environment variables added to README configuration table. `.env.example` expanded from 7 to 35+ variables with section grouping. Dead config removed from `default.toml`. SDK README corrections.
- **Real token counting** in LLM spans — capture actual usage from provider responses.
- **Structured JSON output** for digest generation with fallback parser.
- **`PATCH /api/v1/memory/webhooks/:id`** with TLS enforcement.
- **LLM span persistence** to Redis.
- **Batch community detection**, typed Python SDK, TypeScript SDK.
- **Graph API hardening**, `u32` overflow prevention in `query_edges`.

### Fixed

- **Documentation audit** — SDK README corrections (Python `ChangesSinceResult` field names, TypeScript `graphShortestPath` added). `CONTRIBUTING.md` updated to reflect shipped features (TypeScript SDK, progressive summarization).
- **Falsification audit fixes** — GraphTraversal source tracking bug (count-based instead of relevance-based), 5 missing TypeScript SDK methods + types.

## [0.3.7] — 2026-03-06

### Added

- **Ollama native embed keep-warm** (`mnemo-llm`, `mnemo-server`): Ollama's `/v1/embeddings` (OpenAI-compat) silently ignores `keep_alive`; switched to native `/api/embed` endpoint with `keep_alive: -1` so the model never unloads between requests. Background 180-second keep-warm task in `main.rs` pings the embedder as belt-and-suspenders against OS-level eviction. Eval result with fix: 3/3 quality gates pass — 95% factual recall, 100% temporal query return rate, p95 latency 1575ms.
- **`AsyncMnemo` full parity** (`sdk/python`): 18 methods were present in the sync `Mnemo` client but absent from `AsyncMnemo`. All 18 are now implemented: `context_head`, `time_travel_trace`, `time_travel_summary`, `preview_policy`, `get_policy_audit`, `get_policy_violations`, `create_webhook`, `get_webhook`, `delete_webhook`, `get_webhook_events`, `get_dead_letter_events`, `replay_events`, `retry_event`, `get_webhook_stats`, `get_webhook_audit`, `trace_lookup`, `import_chat_history`, `get_import_job`. Backed by 18 new falsification tests (all pass; total async test count raised from 22 to 40).
- **Progressive session summarization** (`mnemo-ingest`, `mnemo-core`, `mnemo-server`): after every N completed episodes (default N=10, configurable via `MNEMO_SESSION_SUMMARY_THRESHOLD` / `[extraction] session_summary_threshold`), the ingest worker calls `LlmProvider::summarize()` and writes the result back to the session via `update_session`. Non-fatal — LLM failures are logged and ingest continues. `UpdateSessionRequest` gains `summary: Option<String>` and `summary_tokens: Option<u32>` fields; `Session::apply_update()` updated accordingly.
- **LongMemEval benchmark** (`eval/longmem_eval.py`): Mnemo-native evaluation harness covering the 5 LongMemEval task types — single-hop (4 cases), multi-hop (3), temporal (3), preference-tracking (3), and absent-information detection (3). Gate thresholds: single-hop ≥ 80%, multi-hop ≥ 70%, temporal ≥ 75%, preference ≥ 80%, absent ≥ 90%. Integrated into `.github/workflows/benchmark-eval.yml`; results published to job summary. Accepts `--cases-file` for additional JSON case packs and `--gate-only` for CI exit-code gating.

### Fixed

- **33 documentation gaps** across P0/P1/P2 priority levels:
  - `ChangesSinceResult` and `TimeTravelTraceResult` in `_models.py` had wrong field names that didn't match the actual server response shape; body key mappings corrected in both `client.py` and `async_client.py`.
  - `POST /api/v1/memory/extract` and `GET /api/v1/audit/export` were entirely absent from `docs/API.md`; full request/response documentation added for both.
  - `docs/ARCHITECTURE.md`: 7-step retrieval pipeline documented; 11 Redis key patterns added; Qdrant payload indexes section added; Agent Identity Substrate section added; Webhook Delivery Architecture section added; Kubernetes note corrected.
  - `sdk/python/README.md`: `context()` mode values corrected (`"head"`, `"hybrid"`, `"historical"` — not `"semantic"` / `"temporal"`).
  - `README.md`: version number updated to `0.3.6`; reranker TOML config block added to env var table.
  - `crates/mnemo-core/src/models/agent.rs`: full `///` doc comments on all 11 structs/enums and their fields.
  - `docs/TESTING.md`: `eval_recall_quality.py` row added; section 3.5 with run instructions added; integration/unit test counts corrected (91 integration / 24 unit).
  - `CONTRIBUTING.md`: "Areas Where Help Needed" updated — shipped items removed, done annotations added.
  - `FALSIFICATION.md`: resolved annotations added for issues 1, 3, 6, 15, 21, 22.

## [0.3.6] — 2026-03-06

### Added

- **MMR reranker** (`mnemo-retrieval`): implemented `mmr_merge()` — Maximal Marginal Relevance selection that iteratively picks the next result maximising `λ·relevance − (1−λ)·sim_to_selected` (λ=0.7). Reduces near-duplicate results when queries return highly similar items.
- **`RerankerConfig` enum** in `config.rs`: `reranker = "rrf" | "mmr"` in `[retrieval]` TOML section is now parsed and respected. Previously the key was silently ignored. `RerankerMode` in `state.rs` propagates the setting to every `get_context()` call.
- **`Reranker` enum** in `mnemo-retrieval`: `Rrf` | `Mmr`. `get_context()` now takes an explicit `reranker: Reranker` argument. All 8 call sites in `routes.rs` pass `reranker_for_state(&state)`.
- **5 new MMR falsification tests** in `mnemo-retrieval`: `ret_mmr_selects_highest_relevance_first_with_no_prior_selections`, `ret_mmr_penalises_near_duplicate_after_first_selection`, `ret_mmr_empty_input_returns_empty`, `ret_mmr_preserves_all_unique_items`, `ret_mmr_vs_rrf_scores_differ_on_duplicate_heavy_input`.
- **Qdrant payload indexes** (`mnemo-storage`): `ensure_collection()` now creates `CreateFieldIndexCollectionBuilder` indexes on `user_id` (Keyword), `session_id` (Keyword), `processing_status` (Keyword), and `created_at` (Float) after collection creation. Eliminates brute-force payload scans on filtered searches. Non-fatal on failure (logs warn, continues).
- **SDK exponential backoff** (`mnemo-client`): sync `SyncTransport` and async `AsyncMnemo._req()` now use exponential backoff with full jitter (`base * 2^attempt * U(0,1)`) instead of linear delay.
- **Async SDK 429 retry** (`mnemo-client`): `AsyncMnemo._req()` now retries HTTP 429 responses (previously re-raised immediately, unlike the sync client). `MnemoRateLimitError` is raised only after all retries are exhausted.
- **4 new SDK retry falsification tests**: `test_async_429_is_retried_when_max_retries_gt_0`, `test_async_429_raises_after_exhausting_retries`, `test_async_5xx_is_retried_when_max_retries_gt_0`, `test_sync_exponential_backoff`.
- **`tests/eval_recall_quality.py`**: recall quality evaluation harness. 40-fact gold dataset across 4 synthetic users + 3 temporal correctness cases. Three quality gates: factual recall >= 85%, temporal query return rate >= 90%, p95 latency <= 500ms. Run with `python tests/eval_recall_quality.py [--server URL]`.

## [0.3.5] — 2026-03-06

### Added

- **O(1) trace index** (`GET /api/v1/traces/:request_id`): writes a `rid_episodes:{request_id}` sorted set in Redis at episode-create time. Trace lookup now resolves episodes via an indexed `ZRANGEBYSCORE` instead of scanning all users. Constant-time regardless of corpus size.
- **`POST /api/v1/memory/extract`**: synchronous LLM extraction preview endpoint. Returns entities and relationships extracted from arbitrary text. Degrades gracefully (returns `ok=true`, empty extraction, `note: "no_llm: ..."`) when no LLM is configured. Route registered before `/:user/context` to prevent Axum path-matching collision.
- **`LlmHandle` enum** in `state.rs`: dyn-compatible wrapper over `Arc<AnthropicProvider>` and `Arc<OpenAiCompatibleProvider>`. Exposes `extract()`, `provider_name()`, and `model_name()`. Avoids the dyn-incompatibility of `LlmProvider` trait (async fn). Stored as `AppState.llm: Option<LlmHandle>`.
- **`GET /api/v1/audit/export`**: SOC 2 audit log export endpoint. Merges governance and webhook audit events into a unified `AuditExportRecord` array sorted newest-first. Query params: `from`, `to`, `limit` (default 1000, max 10000), `include_governance` (default true), `include_webhook` (default true), `user` (optional filter). Returns 400 when `to < from`.
- 11 new integration tests covering all three items (index correctness, user filter scoping, empty-index case, extract with/without LLM, empty-text 400, audit export empty/governance/time-filter/include-flags/invalid-window).

### Added

- **Python SDK — full production release** (`sdk/python`).
  - **Full API coverage**: `Mnemo` sync client and `AsyncMnemo` async client cover all memory, governance, webhook, operator, import, and session-message endpoints. Zero runtime dependencies for the sync client (stdlib only).
  - **Async client** (`AsyncMnemo`) via `aiohttp` (optional extra: `pip install mnemo-client[async]`). Mirrors the sync interface exactly with `async def` methods and `async with` context manager support.
  - **LangChain adapter** (`mnemo.ext.langchain.MnemoChatMessageHistory`): drop-in `BaseChatMessageHistory` implementation. Lazy-imports `langchain-core` so the core SDK stays zero-dependency. Handles session name→UUID caching, multimodal content flattening, and all message role mappings (`HumanMessage`, `AIMessage`, `SystemMessage`, `ToolMessage`, `ChatMessage`). Supports sync and async variants (`aget_messages`, `aadd_messages`, `aclear`).
  - **LlamaIndex adapter** (`mnemo.ext.llamaindex.MnemoChatStore`): drop-in `BaseChatStore` implementation. All 7 abstract methods implemented (`set_messages`, `get_messages`, `add_message`, `delete_messages`, `delete_message`, `delete_last_message`, `get_keys`). Async variants for all methods. Session key→UUID caching. Graceful out-of-bounds handling on `delete_message`.
  - **Request-ID propagation**: every SDK call accepts and returns `x-mnemo-request-id` for end-to-end distributed tracing through `trace_lookup()`.
  - **Typed result models**: all methods return dataclasses (`RememberResult`, `ContextResult`, `MessagesResult`, `PolicyResult`, `WebhookResult`, `WebhookStats`, `WebhookEvent`, `TimeTravelTraceResult`, `TimeTravelSummaryResult`, `ChangesSinceResult`, `ConflictRadarResult`, `CausalRecallResult`, `OpsSummaryResult`, `TraceLookupResult`, `ImportJobResult`, `AuditRecord`, `Message`, `DeleteResult`, `ReplayResult`, `RetryResult`, `HealthResult`).
  - **Error hierarchy**: `MnemoError` → `MnemoHttpError`, `MnemoRateLimitError`, `MnemoNotFoundError`, `MnemoValidationError`, `MnemoConnectionError`, `MnemoTimeoutError`. Rate-limit errors carry `retry_after_ms` from server headers.
  - **Session message endpoints** on server: `GET /api/v1/sessions/:id/messages` (chronological, paginated), `DELETE /api/v1/sessions/:id/messages` (clear all), `DELETE /api/v1/sessions/:id/messages/:idx` (delete by ordinal index). Required by framework adapters.
  - **Docker test infrastructure**: `sdk/python/docker-compose.test.yml` starts Redis Stack + Qdrant + built Mnemo server on offset ports (8181/6380/6335-6336) to avoid collision with the dev stack.
  - **`sdk/python/Makefile`**: `make test` builds the server image, starts the stack, polls for health, runs `pytest`, and tears down. `make test-local` runs the suite against an already-running server on :8080.
  - **`sdk/python/tests/conftest.py`**: session-scoped `mnemo_client` and `async_mnemo_client` fixtures with server readiness polling.
  - **88-gate falsification suite** (`tests/test_sdk.py`): covers health, add/context, session messages (get/delete-by-index/clear), LangChain structural + functional, LlamaIndex structural + functional (all 7 methods), error handling, and package exports. All 88 gates pass.
  - **18-gate async falsification suite** (`tests/test_async_client.py`): mocked aiohttp transport covers health, add, context, get/clear/delete messages, all HTTP error codes, API key headers, request-ID forwarding, context manager, manual close, changes_since, conflict_radar, causal_recall.

- **Operator Dashboard Phase C**: face-melting polish pass on the embedded dashboard.
  - **Design system upgrade**: full CSS token system (`--bg`, `--bg-surface`, `--bg-card`, semantic color tokens with `-dim` variants, layout and font vars, motion vars).
  - **Animated transitions**: `page-in` fade+slide on every page navigation, `pulse-green` glow on healthy status dot, `shimmer` skeleton loaders while data is fetching, `toast-in`/`toast-out` for toast notification lifecycle.
  - **Toast notification system**: replaces all `alert()` calls — `toast.success/error/info/warn(title, msg)` renders pill notifications in `#toast-container` with auto-dismiss (4s default, 8s for errors) and a close button.
  - **Skeleton loaders**: `mnemo.loading()` now renders animated shimmer blocks instead of plain "Loading…" text.
  - **Badge component**: `badge(label, type)` utility returns pill-shaped `<span class="badge badge-{type}">` elements (green/yellow/red/blue/gray) — replaces raw `status-ok`/`status-error` spans.
  - **SVG horizontal timeline** on RCA page: `buildTimelineSvg(events)` renders an inline `<svg>` with a horizontal axis, colored event dots, alternating above/below labels, and start/end timestamps — replaces the plain table.
  - **D3.js v7 force-directed graph** on Explorer page: replaces the 120-iteration canvas simulation. Features drag nodes, scroll-to-zoom, pan, click to select with node detail panel, hover tooltip, and fit/zoom-in/zoom-out toolbar buttons. Falls back to `runFallbackSimulation()` if CDN is unavailable.
  - **Sidebar SVG icons**: each nav link has an inline `.nav-icon` SVG.
  - **Confirmation modal improvements**: `#modal-title` element, `title` parameter on `confirmAction()`, backdrop-click to dismiss.
  - **Lazy page initialization**: `_pageInits` map — pages (webhooks, rca, governance, traces, explorer) initialize only on first visit. Home always inits at boot.
  - **`fmtDateAgo(iso)`** utility: relative time strings ("5m ago", "2h ago", "3d ago") used throughout tables and audit logs.
  - **`stat-grid` / `stat-row`** layout for key-value displays in webhook detail, governance policy, and node detail panel.
  - **`table-wrap`** overflow container on all tables for responsive horizontal scrolling.
  - **Selected row highlight** on webhook grid (persists across grid reloads).
  - **Better empty states**: descriptive context-specific messages on all empty panels.
  - **`panel`** wrapper class on secondary content sections.
  - **`btn-group`** for grouped button layouts.
  - **`btn-xs`** and `btn-ghost` button variants.
  - **`activity-item`** divs with type-colored left border on Home page recent activity feed.

### Fixed

- `#node-detail-panel` and `#graph-tooltip` now have `class="hidden"` in initial HTML (were visible on load).

- **Operator Dashboard Phase A**: embedded zero-deployment web UI served at `/_/` via `rust-embed`.
  - Dark-themed SPA with 6 pages: Home, Webhooks, RCA, Governance, Traces, Explorer.
  - Home page live-polls `/health`, `/api/v1/ops/summary`, and `/api/v1/memory/webhooks` to display system status, metric cards, and webhook grid.
  - Static assets (HTML, CSS, JS) compiled into the server binary — no separate web server or build step needed.
- **Operator Dashboard Phase B**: feature mapping — all 5 operator pages fully functional.
  - **Webhooks page**: grid with parallel stats fetching, clickable drill-down to detail view (target, circuit status, dead-letter queue with per-event retry, audit log, replay-all, delete with confirmation modal).
  - **RCA page**: time-travel trace submission with snapshot comparison (FROM/TO cards), gained/lost facts, timeline table, retrieval policy diagnostics, summary.
  - **Governance page**: policy viewer with inline edit form, save/preview impact, violations panel (last 24h), audit trail.
  - **Traces page**: request ID lookup rendering matched episodes, webhook events/audit, governance audit.
  - **Knowledge Graph Explorer**: username-to-UUID resolution, entity list, force-directed graph rendering on `<canvas>` (repulsion, spring attraction, color-coded by entity type, dashed red for invalidated edges, labels, legend).
  - Cross-cutting: `confirmAction()` modal, 30s API timeout with `AbortController`, `escapeHtml` XSS protection on all innerHTML, date formatters, status badges, truncated IDs.
- `GET /api/v1/memory/webhooks` — list all registered webhook subscriptions (sorted newest-first, `signing_secret` excluded).
- `tests/dashboard_smoke.sh` — 12-gate dashboard and list-webhooks smoke test script.
- `tests/phase_b_screenshots.py` — Playwright screenshot validation script for all dashboard pages.
- Integration tests: `test_list_memory_webhooks_returns_all_registered` (list endpoint), `test_dashboard_serves_index_and_static_assets` (embedded asset serving + SPA routing).
- Dependencies: `rust-embed v8`, `mime_guess v2`.

### Fixed

- Dashboard `mnemo.api()` no longer sends `Content-Type: application/json` on GET requests (prevented body-less requests from completing on some endpoints).
- Dashboard Explorer page resolves username to UUID via `/api/v1/users/external/:external_id` before calling entities endpoint (was passing raw username as UUID path param).
- Dashboard API calls now abort after 30 seconds via `AbortController` (prevents browser from hanging indefinitely on slow endpoints).

## [0.3.4] — 2026-03-05

### Added

- QA/QC Phase 1: 59 new tests across 5 domains (mnemo-graph, mnemo-llm, Qdrant, AsyncMnemo SDK, webhook persistence).
- QA/QC Phase 2: 44 additional tests (config parsing, session messages, raw vectors, auth integration, request-id, API consistency).
- QA/QC Phase 3: 6 additional tests (webhook rate limiting WH-13, circuit breaker WH-14, RRF reranker diversity RET-08).
- `docs/QA_QC_FALSIFICATION_PRD.md` — comprehensive QA/QC falsification PRD (25 domains, ~170 gates, 3-phase execution plan).
- `tests/docker_build_test.sh` — Docker build and startup falsification script (DK-01 through DK-03).
- `tests/credential_scan.sh` — credential hygiene scanning script (SEC-01 through SEC-05, 5 PASS).
- `tests/deploy_artifact_validation.sh` — deployment artifact structural validation (DEP-02 through DEP-06, 36 PASS).
- `sdk/python/tests/test_async_client.py` — 18 async SDK unit tests with aioresponses (SDK-08 through SDK-10).
- 109 new tests total across all 3 phases, bringing project total to ~226 tests.

### Fixed

- Qdrant `ensure_collection` TOCTOU race condition — swallow "already exists" errors on concurrent creation.
- Qdrant client `skip_compatibility_check()` — prevent version mismatch errors with older Qdrant servers.
- Security: added `*.pem`, `credentials.json`, `terraform.tfstate` to `.gitignore` (SEC-02).
- Doc inconsistencies: API.md version, SDK PRD status, Phase 2 PRD M4 status, CHANGELOG Vultr references.
- Documentation audit: 25 gaps identified and fixed across README.md, API.md, TESTING.md, CHANGELOG.md, and QA_QC_FALSIFICATION_PRD.md.
- README.md: added 5 missing env vars (`MNEMO_SERVER_HOST`, `MNEMO_LLM_BASE_URL`, `MNEMO_EMBEDDING_MODEL`, `MNEMO_EMBEDDING_BASE_URL`, `MNEMO_EMBEDDING_DIMENSIONS`).
- README.md: corrected integration test count (56 → 78) and added QA/QC Falsification section to Project Status.
- QA_QC_FALSIFICATION_PRD.md: corrected route counts (54/66 → 57/72), updated all resolved gate statuses, fixed Coverage Gap Matrix, marked all 3 phases complete in Execution Plan.
- TESTING.md: added test count summary table and QA/QC test sections with run commands.

## [0.3.3] — 2026-03-05

### Added

- Production deployment artifacts for T5–T10 (DigitalOcean, Render, Railway, Vultr, Northflank, Linode) — all falsified end-to-end.
- `deploy/digitalocean/terraform/` — Droplet + Firewall Terraform, Ubuntu 24.04, Docker Compose via user-data. Same startup script pattern as T4 GCP.
- `deploy/render/render.yaml` — Render Blueprint: mnemo web service + managed Redis + Qdrant web service with persistent disk.
- `deploy/render/DEPLOY.md` — Blueprint and manual deploy instructions; cost notes (Redis Stack module caveat documented).
- `deploy/railway/railway.json` + `DEPLOY.md` — Railway template manifest and deploy guide; private networking wiring documented.
- `deploy/vultr/terraform/` + `DEPLOY.md` — Vultr Terraform IaC (vc2-2c-4gb, Ubuntu 24.04, Docker Compose via user-data startup script).
- `deploy/northflank/stack.json` + `DEPLOY.md` — Northflank stack definition (3 services, persistent volumes); CLI and dashboard deploy paths.
- `deploy/linode/terraform/` — Linode instance + Firewall Terraform, `startup.sh.tpl`, variables, outputs. Ubuntu 24.04, Docker Compose stack.
- `deploy/linode/DEPLOY.md` — full guide; cost callout (~$18/month, lowest of IaaS targets).

### Discovered (T10 Linode falsification)

- Linode Ubuntu 24.04 has Docker pre-installed via snap or not at all — startup script uses official Docker apt repo for deterministic install.
- Existing services on host (Qdrant on 6333/6334, n8n on 5678) require Mnemo's internal Qdrant to bind host port 6335→6334 to avoid conflict; compose-internal networking uses `qdrant:6334` unaffected.
- `sudo` requires `-S` flag and password piped via stdin in non-interactive SSH sessions.

## [0.3.2] — 2026-03-05

### Added

- Production deployment artifacts for T1–T4 (Docker, Bare Metal, AWS CloudFormation, GCP Terraform) — all falsified end-to-end.
- `deploy/docker/docker-compose.prod.yml` — production-ready Compose file using GHCR images, named volumes, healthchecks, and resource limits.
- `deploy/docker/docker-compose.managed.yml` — managed-services variant (external Redis + Qdrant); only `mnemo-server` runs locally.
- `deploy/docker/.env.example` — all required and optional env vars with inline comments.
- `deploy/docker/DEPLOY.md` — quick-start guide and managed-services walkthrough.
- `deploy/bare-metal/mnemo.service` — systemd unit with `Restart=always`, `EnvironmentFile=`, and `LimitNOFILE`.
- `deploy/bare-metal/nginx.conf` — reverse proxy reference with timeout config tuned for long-running context requests.
- `deploy/bare-metal/update.sh` — binary swap and `systemctl restart` script for in-place upgrades.
- `deploy/bare-metal/DEPLOY.md` — step-by-step guide: binary download, systemd, nginx, TLS.
- `deploy/aws/cloudformation/mnemo_cfn.yaml` — hardened CloudFormation template: EC2 t3.medium, EBS gp3 volume (inline `BlockDeviceMappings`, no race condition), Security Group, UserData with AL2023 compatibility fixes, AOF-enabled Redis, and `cfn-signal` with 20-minute timeout. All 5 falsification gates passed.
- `deploy/aws/cloudformation/DEPLOY.md` — console + CLI deploy instructions, parameter table, cost estimate (~$32/month), SSH access and teardown.
- `deploy/gcp/terraform/main.tf` — GCP Compute Engine e2-medium, Debian 12, persistent pd-ssd data disk (attached at boot), Docker Compose stack via startup script.
- `deploy/gcp/terraform/variables.tf`, `outputs.tf`, `terraform.tfvars` — full variable/output surface.
- `deploy/gcp/DEPLOY.md` — `gcloud auth`, `terraform init/plan/apply`, verify, destroy. All 5 falsification gates passed.
- `docs/PRD_DEPLOY.md` — Deployment PRD covering T1–T10 targets, resource floors, rollout phasing, and falsification gate contract.

### Fixed

- Root `.gitignore` now excludes `.keys/` (cloud credential directories).
- `deploy/gcp/terraform/.gitignore` excludes `.terraform/`, `terraform.tfstate*`, and plan files.

### Discovered (deployment falsification)

- **CloudFormation `!Sub` + bash heredocs**: `Fn::Sub` list form required to prevent `${VAR:-default}` bash default syntax from being processed as CloudFormation substitutions.
- **EBS attach race condition**: Separate `AWS::EC2::Volume` + `VolumeAttachment` resources race against UserData. Fix: inline `BlockDeviceMappings` on the instance.
- **AL2023 `curl` conflict**: `dnf install curl` conflicts with pre-installed `curl-minimal`. Dropped `curl` from install list.
- **Redis AOF**: `--save 60 1` alone loses data on restarts within 60s. Fix: `--appendonly yes` alongside RDB.
- **GCP SSH**: `gcloud compute ssh` uses a non-standard port internally; direct `ssh -p 22` with the generated key works reliably.
- **GHCR versioned tags**: Only `latest` is currently published. Deployment templates default to `latest`.

## [0.3.1] — 2026-03-04

### Added

- User policy APIs for retention defaults, webhook domain allowlists, and governance audit (`/api/v1/policies/:user`, `/api/v1/policies/:user/audit`).
- Operator endpoints for dashboard and trace explorer (`/api/v1/ops/summary`, `/api/v1/traces/:request_id`).
- Policy preview endpoint (`/api/v1/policies/:user/preview`) for retention/governance impact estimation before apply.
- Policy violation window query endpoint (`/api/v1/policies/:user/violations`) for operator triage over bounded time ranges.
- Time travel summary endpoint (`/api/v1/memory/:user/time_travel/summary`) for lightweight snapshot delta counts and fast RCA-first rendering.
- Operator drill runner script (`tests/operator_p0_drills.sh`) to exercise dead-letter, RCA, and governance workflow suites.
- Operator UX PRD and execution backlog (`docs/OPERATOR_UX_PRD.md`, `docs/OPERATOR_UX_EXECUTION_BACKLOG.md`).
- Read-path retention enforcement on context, `changes_since`, and `time_travel/trace` responses, filtering episodes past per-user retention windows.
- Replay cursor pagination falsification test covering chronological ordering, sparse IDs, unknown cursor reset, filter interactions, and limit clamping.
- Contract/retrieval policy combination consistency test: exhaustive 4×4 matrix (16 cases) verifying `retrieval_policy_diagnostics` resolution across all `MemoryContract` × `AdaptiveRetrievalPolicy` pairs.
- SDK Integrations PRD (`docs/SDK_INTEGRATIONS_PRD.md`) — Python SDK rebuild, LangChain `MnemoChatMessageHistory`, LlamaIndex `MnemoChatStore`, Docker-based falsification.
- Operator Dashboard PRD (`docs/OPERATOR_DASHBOARD_PRD.md`) — embedded zero-deployment dashboard with dead-letter recovery, RCA canvas, governance center, and graph explorer.
- Raw Vector API (`/api/v1/vectors/:namespace/*`) — 6 endpoints exposing Mnemo as a pluggable vector database for external systems like AnythingLLM. Supports upsert, similarity search, delete, count, namespace lifecycle, and automatic dimension detection.
- `RawVectorStore` trait in `mnemo-core` for namespace-based raw vector operations, isolated from internal entity/edge/episode collections.
- AnythingLLM vector DB provider (`integrations/anythingllm/`) — drop-in Node.js adapter implementing AnythingLLM's `VectorDatabase` base class for seamless integration.
- Raw Vector API falsification test suite (39 assertions covering upsert, search, delete, idempotency, batch operations, validation, and namespace lifecycle).
- Session Messages API — 3 new endpoints enabling framework adapters: `GET /api/v1/sessions/:id/messages` (chronological message list with pagination), `DELETE /api/v1/sessions/:id/messages` (clear session), `DELETE /api/v1/sessions/:id/messages/:idx` (delete by ordinal index). Falsified with 31 assertions.
- `delete_episode(id)` and `delete_session_episodes(session_id)` methods added to `EpisodeStore` trait and implemented in `RedisStateStore`.
- Python SDK full rebuild (`sdk/python/mnemo-client 0.3.1`):
  - `Mnemo` sync client with complete API coverage (27 methods: memory, governance, webhooks, operator, import, session messages, health).
  - `AsyncMnemo` async client (aiohttp-backed) mirroring all sync methods.
  - Typed exception hierarchy (`_errors.py`): `MnemoError`, `MnemoConnectionError`, `MnemoTimeoutError`, `MnemoHttpError`, `MnemoRateLimitError`, `MnemoNotFoundError`, `MnemoValidationError`.
  - Typed result dataclasses (`_models.py`) for all 18 response types.
  - `SyncTransport` with retry logic, `x-mnemo-request-id` propagation, and typed error mapping.
  - `context()` now includes `contract`, `retrieval_policy`, `filters`, `time_intent`, `as_of`, `temporal_weight` parameters.
  - `mnemo.ext.langchain.MnemoChatMessageHistory` — drop-in LangChain `BaseChatMessageHistory` adapter.
  - `mnemo.ext.llamaindex.MnemoChatStore` — drop-in LlamaIndex `BaseChatStore` adapter (all 7 abstract methods).
  - Optional extras: `[async]` (aiohttp), `[langchain]` (langchain-core), `[llamaindex]` (llama-index-core), `[all]`.
  - SDK falsification test suite (`sdk/python/tests/test_sdk.py`) — 65 assertions, all passing against live server.

### Changed

- Policy defaults now auto-apply to memory context/trace requests when callers omit contract or retrieval policy fields.
- Episode write APIs now enforce per-user retention windows (`retention_days_message`, `retention_days_text`, `retention_days_json`).
- Manual webhook retry response now includes an optional event snapshot envelope for immediate operator confirmation (`/api/v1/memory/webhooks/:id/events/:event_id/retry`).
- Trace lookup endpoint now supports bounded windows and source filters for faster incident-time joins (`/api/v1/traces/:request_id`).

### Fixed

- `delete_entity` and `delete_edge` handlers now emit governance audit events (`entity_deleted`, `edge_deleted`), closing a gap where destructive entity/edge operations were untracked.

## [0.3.0] — 2026-03-04

### Added

- Time Travel Trace API (`/api/v1/memory/:user/time_travel/trace`) for windowed memory snapshot diffing and timeline-level change evidence.
- Webhook operational endpoints: dead-letter event listing and delivery stats (`/api/v1/memory/webhooks/:id/events/dead-letter`, `/api/v1/memory/webhooks/:id/stats`).
- Webhook replay, manual retry, and audit endpoints (`/api/v1/memory/webhooks/:id/events/replay`, `/api/v1/memory/webhooks/:id/events/:event_id/retry`, `/api/v1/memory/webhooks/:id/audit`).
- P0 Ops Control Plane PRD (`docs/P0_OPS_CONTROL_PLANE_PRD.md`) with scope, rollout, and falsification gates.
- Prometheus-compatible metrics endpoint (`/metrics`) for HTTP/webhook delivery telemetry.
- Request correlation propagation with `x-mnemo-request-id` response header support.
- Webhook event and audit records now retain originating request IDs for end-to-end trace joins.
- Episode writes now persist request IDs into metadata, enabling trace joins in `changes_since`, `time_travel/trace`, ingest logs, and webhook delivery.

### Changed

- Webhook delivery now supports dead-letter marking, per-webhook rate limiting, and circuit breaker cooldown behavior.
- Webhook subscriptions and delivery event rows are now persisted to Redis and restored on server startup.
- CI quality gates now include a temporal quality budget check (accuracy, stale rate, p95 latency).

## [0.2.0] — 2026-03-03

### Added

- Release automation workflow for version tags (`.github/workflows/release.yml`).
- GHCR package publication workflow (`.github/workflows/package-ghcr.yml`).
- Memory webhook API (`/api/v1/memory/webhooks`) with retained delivery event telemetry.
- Outbound webhook delivery pipeline with exponential retry/backoff and optional HMAC signatures (`x-mnemo-signature`).

### Changed

- Workspace repository metadata now points to the canonical repository URL.
- Added `.dockerignore` to reduce container build context and improve image build consistency.
- README and evaluation/testing docs now reflect current quick-win memory APIs and latest falsification/benchmark snapshots.
- README release/package section now documents current GitHub Release artifacts and GHCR pull/tag strategy.

## [0.1.0] — 2026-03-01

### Added

**Core**
- Domain models: User, Session, Episode, Entity, Edge, ContextBlock
- Bi-temporal edge model with `valid_at`/`invalid_at` lifecycle
- Custom `EntityType` serde with flexible parsing (known types + custom strings)
- Unified `MnemoError` type with HTTP status codes and error code strings
- Storage traits: `UserStore`, `SessionStore`, `EpisodeStore`, `EntityStore`, `EdgeStore`, `VectorStore`
- Composite `StateStore` trait (Redis side) separate from `VectorStore` (Qdrant side)
- LLM traits: `LlmProvider` (extraction, summarization, contradiction detection) and `EmbeddingProvider`
- Token-budgeted context assembly with section header accounting

**Storage**
- `RedisStateStore`: Full implementation of all state storage traits
- Redis key schema with sorted sets for pagination, adjacency lists for graph traversal
- Atomic episode claiming via `ZREM` for safe concurrent processing
- Entity name index for O(1) deduplication lookups
- `QdrantVectorStore`: Entity, edge, and episode embedding storage
- Cosine similarity search with tenant isolation via `user_id` filter
- GDPR-compliant `delete_user_vectors` across all collections

**LLM**
- `OpenAiCompatibleProvider`: Works with OpenAI, Anthropic, Ollama, Liquid AI, vLLM
- Structured entity/relationship extraction with JSON parsing (handles markdown fences)
- Rate limit detection with `retry_after_ms` propagation
- `OpenAiCompatibleEmbedder`: Batch embedding generation

**Ingestion**
- Background worker with configurable poll interval, batch size, and concurrency
- Pipeline: claim → extract → deduplicate entities → invalidate conflicting edges → embed
- Automatic entity deduplication against existing graph
- Automatic contradiction detection and edge invalidation

**Retrieval**
- Hybrid search: semantic (Qdrant) + graph traversal
- Temporal filtering (point-in-time queries)
- Relevance-sorted results across entities, facts, and episodes
- Token-budgeted context string assembly

**Graph**
- BFS traversal with configurable depth and node limit
- Label propagation community detection
- Temporal awareness (valid edges only)

**Server**
- 25 REST API endpoints (users, sessions, episodes, entities, edges, context, graph)
- TOML configuration with environment variable overrides
- Health check endpoint
- CORS support
- Structured error responses with consistent error codes
- Cursor-based pagination on all list endpoints

**Infrastructure**
- 7-crate Rust workspace with clean dependency graph
- Docker Compose (Redis Stack + Qdrant + Mnemo)
- Multi-stage Dockerfile (builder + minimal runtime)
- Release profile: LTO, single codegen unit, stripped binary
- Apache 2.0 license
