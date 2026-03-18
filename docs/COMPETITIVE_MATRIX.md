# Competitive Feature Matrix

How Mnemo compares to the three leading AI memory systems. Assessed feature-by-feature against public documentation, GitHub repos, and SDK references as of March 2026. Updated for v0.9.0.

> Legend: :green_circle: Shipped  :yellow_circle: Partial / limited  :red_circle: Not available

## Memory & Retrieval (20 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Semantic vector search | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| Knowledge graph (entity/edge API) | :green_circle: | :green_circle: | :green_circle: | :red_circle: |
| Graph traversal (BFS, shortest path) | :green_circle: | :yellow_circle: | :red_circle: | :red_circle: |
| Community detection | :green_circle: | :green_circle: | :red_circle: | :red_circle: |
| Temporal retrieval (`as_of`, `time_intent`) | :green_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| Fact supersession / invalidation tracking | :green_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| Memory diff (`changes_since`) | :green_circle: | :yellow_circle: | :red_circle: | :red_circle: |
| Contradiction / conflict detection | :green_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| Reranking (RRF, MMR, cross-encoder) | :green_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| Hybrid retrieval (vector + full-text + graph) | :green_circle: | :green_circle: | :yellow_circle: | :yellow_circle: |
| Context assembly with token budgeting | :green_circle: | :green_circle: | :red_circle: | :yellow_circle: |
| Memory contracts (SupportSafe, CurrentStrict, etc.) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| GNN-enhanced re-ranking | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Hyperbolic geometry re-ranking | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Confidence decay + revalidation | :green_circle: | :yellow_circle: | :yellow_circle: | :red_circle: |
| Cross-session narrative summaries | :green_circle: | :green_circle: | :red_circle: | :yellow_circle: |
| Goal-conditioned retrieval | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Counterfactual ("what if") queries | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Custom ontology / entity types | :green_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| Multi-modal (images, documents) | :red_circle: | :yellow_circle: | :green_circle: | :yellow_circle: |

## Agent Architecture (11 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Agent identity core (personality versioning) | :green_circle: | :red_circle: | :red_circle: | :yellow_circle: |
| Experience weighting (EWC++ consolidation) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Agent COW branching (A/B identity testing) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Agent fork (domain expansion) | :green_circle: | :yellow_circle: | :red_circle: | :red_circle: |
| Witness chain audit (tamper-proof) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Multi-agent communication | :red_circle: | :red_circle: | :yellow_circle: | :green_circle: |
| Agent self-modification of memory | :red_circle: | :green_circle: | :green_circle: | :green_circle: |
| Tool use / function calling | :red_circle: | :red_circle: | :green_circle: | :green_circle: |
| Human-in-the-loop approval | :yellow_circle: | :red_circle: | :red_circle: | :green_circle: |
| Shared memory blocks (real-time) | :green_circle: | :green_circle: | :yellow_circle: | :green_circle: |
| Per-agent embedding personalization (TinyLoRA) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |

## Compute & Observability (9 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Sleep-time / offline compute | :green_circle: | :yellow_circle: | :red_circle: | :green_circle: |
| Proactive re-ranking during idle windows | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Memory digest generation | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| LLM call tracing / spans | :green_circle: | :green_circle: | :yellow_circle: | :green_circle: |
| Token usage tracking per span | :green_circle: | :yellow_circle: | :yellow_circle: | :yellow_circle: |
| Time travel debugger (snapshot comparison) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| DAG pipeline metrics | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Operator dashboard | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| OpenTelemetry export | :green_circle: | :green_circle: | :red_circle: | :green_circle: |

## Governance & Compliance (12 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| RBAC / role-based access control | :green_circle: | :green_circle: | :yellow_circle: | :yellow_circle: |
| Data classification labels | :green_circle: | :red_circle: | :green_circle: | :red_circle: |
| Policy-scoped memory views | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Memory guardrails engine | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Multi-agent shared memory regions + ACLs | :green_circle: | :yellow_circle: | :yellow_circle: | :yellow_circle: |
| Agent promotion governance (quorum, cooling) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Per-user retention policies | :green_circle: | :yellow_circle: | :yellow_circle: | :red_circle: |
| Policy preview (dry-run impact analysis) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Governance audit trail | :green_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| SOC 2 Type II certification | :yellow_circle: | :green_circle: | :yellow_circle: | :red_circle: |
| HIPAA compliance (BAA available) | :red_circle: | :green_circle: | :green_circle: | :red_circle: |
| BYOK (customer-managed encryption keys) | :green_circle: | :green_circle: | :green_circle: | :green_circle: |

## Webhooks & Events (6 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Memory lifecycle webhooks | :green_circle: | :green_circle: | :green_circle: | :yellow_circle: |
| Webhook management via API | :green_circle: | :yellow_circle: | :green_circle: | :red_circle: |
| Dead-letter queue + replay + retry | :green_circle: | :yellow_circle: | :red_circle: | :yellow_circle: |
| HMAC signature verification | :green_circle: | :green_circle: | :red_circle: | :red_circle: |
| Circuit breaker + rate limiting | :green_circle: | :yellow_circle: | :yellow_circle: | :red_circle: |
| Webhook delivery stats + audit | :green_circle: | :green_circle: | :red_circle: | :red_circle: |

## SDKs & Integrations (12 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Python SDK | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| TypeScript SDK | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| Go SDK | :red_circle: | :green_circle: | :red_circle: | :red_circle: |
| Async client | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| LangChain adapter | :green_circle: | :green_circle: | :green_circle: | :red_circle: |
| LlamaIndex adapter | :green_circle: | :red_circle: | :green_circle: | :red_circle: |
| Vercel AI SDK adapter | :green_circle: | :red_circle: | :green_circle: | :red_circle: |
| CrewAI / AutoGen adapter | :red_circle: | :green_circle: | :green_circle: | :red_circle: |
| gRPC API | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| MCP server | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| OpenAPI spec | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| CLI tool | :red_circle: | :green_circle: | :yellow_circle: | :green_circle: |

## Deployment & Operations (8 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Self-hosted / open source | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| Docker one-line deploy | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| Cloud IaC templates (AWS, GCP, DO, etc.) | :green_circle: | :red_circle: | :red_circle: | :red_circle: |
| Managed cloud offering | :red_circle: | :green_circle: | :green_circle: | :green_circle: |
| Local embeddings (no API key needed) | :green_circle: | :green_circle: | :green_circle: | :green_circle: |
| Kubernetes / Helm chart | :green_circle: | :yellow_circle: | :yellow_circle: | :green_circle: |
| Pluggable graph backends | :red_circle: | :green_circle: | :green_circle: | :red_circle: |
| Pluggable vector backends | :red_circle: | :yellow_circle: | :green_circle: | :yellow_circle: |

## Scorecard

| | Mnemo | Zep | Mem0 | Letta |
|---|---:|---:|---:|---:|
| :green_circle: Shipped | **64** | **38** | **26** | **22** |
| :yellow_circle: Partial | **2** | **14** | **19** | **11** |
| :red_circle: Not available | **11** | **25** | **32** | **44** |
| **Total features** | **77** | **77** | **77** | **77** |

## Honesty Notes

We take accuracy seriously. Every claim below has a source link or caveat.

### Mnemo Caveats

- **SOC 2**: Technical controls are implemented and mapped to Trust Service Criteria. No formal audit has been conducted yet.
- **Human-in-the-loop**: Agent promotion governance has quorum-based approval. Mnemo does not have per-tool HITL gating like Letta.
- **Reranking caveat**: MMR uses score-proximity approximation rather than full embedding dot-product.
- **Multi-modal**: Not supported. Text-only memory ingestion. This is a genuine gap.
- **Managed cloud**: Not yet available. Mnemo is self-hosted only. This is a genuine gap for teams that prefer managed infrastructure.
- **CORS**: Configurable via `MNEMO_CORS_ALLOWED_ORIGINS` (comma-separated) or `cors_allowed_origins` in TOML. Defaults to `["*"]` for backward compatibility. Set specific origins for production.
- **Auth-exempt routes**: Unauthenticated routes (health, swagger, dashboard) receive a read-only `CallerContext::anonymous()`. Auth-disabled mode still grants admin via `CallerContext::admin_bootstrap()`.
- **OpenAPI paths**: All 142 REST handlers have `#[utoipa::path]` annotations. Some handlers return `Json<serde_json::Value>` with `Object` response schemas for intentionally dynamic responses.
- **BYOK key rotation**: Supports key rotation via `MNEMO_ENCRYPTION_RETIRED_KEYS`. No automatic re-encryption workflow yet.
- **OTLP security**: Supports TLS and auth headers. Defaults to plaintext for local collectors.
- **Helm Qdrant auth**: Supported via `qdrant.apiKey`. Disabled by default.
- **Helm Ingress TLS**: Supports TLS but defaults to disabled.
- **Helm NetworkPolicy**: Optional (`networkPolicy.enabled: true`). Disabled by default.

### Zep Notes

- **Self-hosted**: [Graphiti](https://github.com/getzep/graphiti) is fully open-source (Apache 2.0) with Neo4j, FalkorDB, Kuzu, and Neptune backends.
- **Graph traversal**: BFS shipped (`bfs_origin_node_uuids`), plus node-distance reranker. No shortest-path API.
- **Reranking**: Five rerankers: RRF, MMR, cross-encoder, node_distance, episode_mentions.
- **Community detection**: Leiden algorithm with incremental updates.
- **Temporal**: Full bi-temporal model (`created_at`, `valid_at`, `invalid_at`, `expired_at`).
- **Memory diff**: No first-class `changes_since` endpoint, but datetime filters achieve equivalent results.
- **Contradiction detection**: Automatic fact invalidation on conflicting ingestion.
- **Tracing**: Full OpenTelemetry support in Graphiti.
- **Sleep-time**: Graph construction and community building are async/offline.
- **Agent self-modification**: Full CRUD API for graph nodes/edges/episodes.
- **Confidence decay**: `recency_weight` with configurable half-lives. No formal revalidation workflow.
- **Narrative summaries**: Entity and community summaries evolve across sessions.
- **Local embeddings**: Graphiti supports Ollama.
- **Webhook management**: Dashboard UI only, not API.
- **Shared memory**: Group graphs for cross-user sharing. No per-graph agent ACLs.
- **Vector backends**: Pluggable embedders but storage tied to graph backend's native index.

### Mem0 Notes

- **Graph**: Full entity/edge extraction via LLM into Neo4j/Memgraph/Neptune/Kuzu. Not a traversable graph API.
- **Contradiction**: Built into `add()` pipeline via LLM. Internal process, not user-facing API.
- **Reranking**: Cross-encoder with 5 providers. No RRF or MMR fusion.
- **Temporal**: Custom timestamps, time-based filters. No `as_of` point-in-time query.
- **Data classification**: Custom categories (default 15 or user-defined).
- **RBAC**: Org/project/workspace scoping. No documented role definitions.
- **HIPAA**: Compliant with BAA available.
- **SOC 2**: Type I completed. Type II in progress.
- **Shared memory**: OpenMemory + scoping enables cross-agent sharing. No explicit ACLs.
- **Webhooks**: SDK-managed CRUD. No HMAC signing or delivery stats.
- **Multi-modal**: Images (JPG, PNG), documents (MDX, TXT), and PDFs supported.
- **CLI**: OpenMemory has a CLI (`npx @openmemory/install`).

### Letta Notes

- **Sleep-time**: Background "sleep-time agents" process conversation history. Research paper published (arXiv:2504.13171).
- **Context assembly**: Character-based limits, not token-based budgeting.
- **Hybrid retrieval**: Separate vector and full-text search tools. Not unified.
- **Multi-agent communication**: Three built-in tools for messaging.
- **Self-modification**: Core architecture with `memory_insert`, `memory_replace`, `memory_rethink` tools.
- **HITL**: Per-tool and per-agent approval configuration.
- **RBAC**: Enterprise-only. Not in OSS.
- **Identity**: Memory blocks as mutable persona. No versioning/rollback/audit.
- **Tracing**: OTel collector configs available. Runs & Steps model.
- **Narrative summaries**: Sleep-time agents generate "learned context" summaries.
- **Multi-modal**: Document processing via code interpreter. No native image embeddings.
- **Webhooks**: Step-complete webhook only. No memory lifecycle events.
- **CLI**: `@letta-ai/letta-code` npm CLI.
- **BYOK**: "Connect your own LLM API keys" listed on pricing.
- **Local embeddings**: Docker deployment supports Ollama.
- **Vector backends**: pgvector only.
