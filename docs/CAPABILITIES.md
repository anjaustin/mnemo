# Core Capabilities

Mnemo provides memory infrastructure for production AI agents. This document covers all major capabilities.

## Memory & Retrieval

### Temporal Knowledge Graph

Automatically extracts entities and relationships from conversations and tracks how facts change over time. Every fact has `valid_at` and `invalid_at` timestamps.

### Bi-temporal Retrieval

Answers both "what is true now" and "what was true then" with the `as_of` parameter for point-in-time queries.

### Hybrid Search

Combines semantic vector search (Qdrant), full-text search (RediSearch), and graph traversal for comprehensive retrieval.

### Context Assembly

Token-budgeted context blocks optimized for LLM prompts. Tiered allocation (60%/25%/15%) prioritizes most relevant content.

### Memory Contracts

Predefined retrieval policies:
- `default` - balanced hybrid retrieval
- `support_safe` - conservative, avoids stale facts
- `current_strict` - only currently-valid facts
- `historical_strict` - requires explicit `as_of`

### Reranking

- **RRF** (Reciprocal Rank Fusion) - default, merges ranked lists
- **MMR** (Maximal Marginal Relevance) - diversity-optimized
- **GNN** - Graph Attention Network re-ranking on local subgraphs
- **Hyperbolic** - Poincare ball geometry for hierarchical entities

### Memory Diff

`changes_since` API returns gained/lost facts between two timestamps.

### Conflict Detection

Automatic contradiction detection with severity scoring and resolution queue.

### Confidence Decay

Facts decay in confidence over time unless reinforced. Revalidation triggers for stale but important facts.

### Goal-Conditioned Retrieval

Condition retrieval by active objective (e.g., `goal=resolve_ticket`) with category/label boosts.

### Counterfactual Queries

Simulate retrieval under hypothetical assumptions without modifying state.

## Agent Architecture

### Agent Identity Core

Cryptographic audit chain (SHA-256 Merkle-style witness) for agent personality with versioning, rollback, and promotion workflows.

### Experience Weighting (EWC++)

Elastic Weight Consolidation for agent experiences. High-importance experiences resist temporal decay.

### Agent Branching

Copy-on-write branching for A/B testing agent personality changes. Merge validated changes back to main identity.

### Agent Fork

Create new agents from existing ones with selective experience transfer.

### TinyLoRA Personalization

Per-`(user, agent)` rank-8 LoRA adapters that rotate base embeddings toward observed relevance history. Updated implicitly from retrieval feedback.

### Multi-Agent Shared Memory

Memory regions with per-agent ACLs (`read`/`write`/`manage`), owner-only mutation, and optional expiry.

## Governance & Compliance

### Scoped API Keys (RBAC)

Role-based access control (`read`/`write`/`admin`) with optional user, agent, and classification scoping. Key rotation and revocation via API.

### Data Classification

Four-tier labeling (`public`/`internal`/`confidential`/`restricted`) enforced at retrieval time.

### Memory Views

Named, reusable access policies filtering by classification ceiling, entity types, edge labels, temporal scope, and fact count.

### Memory Guardrails

Composable rule engine with condition predicates (classification, confidence, age, regex, role) and actions (block, redact, reclassify, audit). Dry-run evaluation endpoint.

### Agent Promotion Governance

Proposal-based approval workflows with configurable quorum, cooling periods, auto-reject deadlines, and conflict analysis.

### Retention Policies

Per-user retention defaults with write guards. Policy preview and violation-window query endpoints.

### Audit Trail

Governance audit with per-action timestamps, user IDs, request IDs. SOC 2-ready audit export endpoint.

### BYOK Encryption

AES-256-GCM at-rest encryption for Redis state with customer-managed keys. Key rotation support.

## Webhooks & Events

### Memory Lifecycle Webhooks

Proactive events:
- `head_advanced` - new episode becomes HEAD
- `conflict_detected` - contradiction found
- `fact_added` - new fact extracted
- `fact_superseded` - fact invalidated

### Delivery Infrastructure

- Exponential retry/backoff
- HMAC signature verification
- Circuit breaker + rate limiting
- Dead-letter queue with replay/retry
- Delivery stats and audit

## Compute & Observability

### Sleep-Time Compute

Background consolidation during user idle windows:
- Memory digest generation
- Proactive re-ranking
- Relevance score updates

### Time Travel Debugger

Compare memory snapshots across time. Returns timeline-level evidence explaining changes.

### LLM Call Tracing

Span-level capture of all LLM calls: prompt, completion, model, token counts, latency, errors. Redis persistence with 7-day TTL.

### OpenTelemetry Export

OTLP trace export with TLS and auth header support.

### Operator Dashboard

Embedded zero-deployment web UI at `/_/` with:
- System health and metrics
- Webhook management and dead-letter recovery
- Knowledge graph explorer (D3 force layout)
- RCA time-travel traces
- Governance policy viewer

### DAG Pipeline Metrics

Per-step latency/throughput metrics for the ingestion pipeline.

## APIs & SDKs

### REST API

142 endpoints with OpenAPI 3.1 spec. Swagger UI at `/swagger-ui/`.

### gRPC API

6 services, 30 RPCs with full data-plane parity. Multiplexed on REST port by default or dedicated port via `MNEMO_GRPC_PORT`.

### MCP Server

Full Model Context Protocol implementation for Claude Code and compatible clients.

**Transports:**
- stdio (default) - for CLI integration
- SSE (optional `--features sse`) - HTTP-based with POST /message, GET /sse endpoints

**13 Tools:**
- `remember` - store memories with temporal tracking
- `recall` - semantic search with hybrid retrieval
- `digest` - get/regenerate user memory summaries
- `scopes` - list data classification scopes
- `graph_query` - knowledge graph operations (neighbors, shortest_path, communities)
- `agent_identity` - get/update agent personality profiles
- `delegate` - create shared memory regions with ACLs
- `revoke` - remove agent access to memory regions
- `experience` - log agent experience events with EWC++ weighting
- `relate` - manage entity relationships (connect/disconnect/list)
- `forget` - request memory deletion with audit trail

**11 Resource Templates:**
- `mnemo://users/{user}/memory/search?query={query}` - semantic search
- `mnemo://users/{user}/digest` - memory digest
- `mnemo://users/{user}/episodes/{episode_id}` - episode details
- `mnemo://agents/{agent_id}/promotions` - pending promotions
- `mnemo://users/{user}/graph/edges` - knowledge graph edges
- `mnemo://users/{user}/graph/communities` - detected communities

**5 Prompt Templates:**
- `memory-context` - load relevant memories for a topic
- `memory-summary` - summarize user memory
- `identity-reflection` - reflect on agent identity and experiences
- `entity-analysis` - analyze entity relationships in knowledge graph
- `remember-conversation` - generate memory-optimized conversation summary

**Resource Subscriptions:**
- `resources/subscribe` / `resources/unsubscribe` for real-time updates
- SSE transport broadcasts `notifications/resources/updated` events

### Python SDK

Zero-dependency sync client (`Mnemo`) and async client (`AsyncMnemo`) with full API coverage, typed results, and request-ID propagation.

**LangChain adapter**: Drop-in `MnemoChatMessageHistory` via `mnemo.ext.langchain`.

**LlamaIndex adapter**: Drop-in `MnemoChatStore` via `mnemo.ext.llamaindex`.

### TypeScript SDK

Fetch-based client with full API parity. Works in Node.js, Deno, Bun, and modern browsers.

**LangChain.js adapter**: Drop-in `MnemoChatMessageHistory` via `mnemo-client/langchain`.

**Vercel AI SDK adapter**: `mnemoRemember`, `mnemoRecall`, `mnemoDigest` tools via `mnemo-client/vercel-ai`.

### Raw Vector API

General-purpose vector database endpoints for external integrations (upsert, similarity search, delete, count, namespace lifecycle).

## Deployment

### Self-Hosted

Single Rust binary with Redis + Qdrant as backing services.

### Docker

One-line quickstart with local embeddings (no API key required).

### Cloud IaC

10 deployment targets all falsified:
- Docker Compose
- AWS CloudFormation
- GCP Terraform
- DigitalOcean Terraform
- Render
- Railway
- Vultr Terraform
- Northflank
- Linode Terraform
- Kubernetes / Helm

### Production Helm Chart

HA-ready with Redis and Qdrant subcharts, security-hardened defaults.

### LLM Providers

Works with Anthropic, OpenAI, Ollama, Liquid AI, or no external LLM.

### Local Embeddings

FastEmbed support for fully offline operation.
