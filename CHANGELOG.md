# Changelog

All notable changes to Mnemo will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Policy preview endpoint (`/api/v1/policies/:user/preview`) for retention/governance impact estimation before apply.
- Policy violation window query endpoint (`/api/v1/policies/:user/violations`) for operator triage over bounded time ranges.
- Time travel summary endpoint (`/api/v1/memory/:user/time_travel/summary`) for lightweight snapshot delta counts and fast RCA-first rendering.
- Operator drill runner script (`tests/operator_p0_drills.sh`) to exercise dead-letter, RCA, and governance workflow suites.

### Changed

- Manual webhook retry response now includes an optional event snapshot envelope for immediate operator confirmation (`/api/v1/memory/webhooks/:id/events/:event_id/retry`).
- Trace lookup endpoint now supports bounded windows and source filters for faster incident-time joins (`/api/v1/traces/:request_id`).

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
- User policy APIs for retention defaults, webhook domain allowlists, and governance audit (`/api/v1/policies/:user`, `/api/v1/policies/:user/audit`).
- Operator endpoints for dashboard and trace explorer (`/api/v1/ops/summary`, `/api/v1/traces/:request_id`).

### Changed

- Webhook delivery now supports dead-letter marking, per-webhook rate limiting, and circuit breaker cooldown behavior.
- Webhook subscriptions and delivery event rows are now persisted to Redis and restored on server startup.
- CI quality gates now include a temporal quality budget check (accuracy, stale rate, p95 latency).
- Policy defaults now auto-apply to memory context/trace requests when callers omit contract or retrieval policy fields.
- Episode write APIs now enforce per-user retention windows (`retention_days_message`, `retention_days_text`, `retention_days_json`).

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
