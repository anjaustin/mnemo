# Changelog

All notable changes to Mnemo will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- QA/QC Phase 1: 59 new tests across 5 domains (mnemo-graph, mnemo-llm, Qdrant, AsyncMnemo SDK, webhook persistence).
- QA/QC Phase 2: 44 additional tests (config parsing, session messages, raw vectors, auth integration, request-id, API consistency).
- `docs/QA_QC_FALSIFICATION_PRD.md` — comprehensive QA/QC falsification PRD (25 domains, ~170 gates).
- `tests/docker_build_test.sh` — Docker build and startup falsification script (DK-01 through DK-03).

### Fixed

- Qdrant `ensure_collection` TOCTOU race condition — swallow "already exists" errors on concurrent creation.
- Qdrant client `skip_compatibility_check()` — prevent version mismatch errors with older Qdrant servers.
- Security: added `*.pem`, `credentials.json`, `terraform.tfstate` to `.gitignore` (SEC-02).
- Doc inconsistencies: API.md version, SDK PRD status, Phase 2 PRD M4 status, CHANGELOG Vultr references.

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
