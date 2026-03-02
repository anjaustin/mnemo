# Changelog

All notable changes to Mnemo will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
