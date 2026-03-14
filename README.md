# Mnemo

| CI | Falsification | Benchmarks | Packages | Release |
| --- | --- | --- | --- | --- |
| [![quality-gates](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml) | [![memory-falsification](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml) | [![benchmark-eval](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml) | [![package-ghcr](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml) | [![release](https://github.com/anjaustin/mnemo/actions/workflows/release.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/release.yml) |

| Version | Release Date | License | Stars | Downloads |
| --- | --- | --- | --- | --- |
| [![version](https://img.shields.io/github/v/tag/anjaustin/mnemo?sort=semver&label=version)](https://github.com/anjaustin/mnemo/releases) | [![release-date](https://img.shields.io/github/release-date/anjaustin/mnemo)](https://github.com/anjaustin/mnemo/releases) | [![license-apache](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE) | [![stars](https://img.shields.io/github/stars/anjaustin/mnemo?style=flat&label=stars&color=blue)](https://github.com/anjaustin/mnemo/stargazers) | [![downloads](https://img.shields.io/github/downloads/anjaustin/mnemo/total?label=downloads&color=blue)](https://github.com/anjaustin/mnemo/releases) |

| Latest Release | Release Artifacts | GHCR Package |
| --- | --- | --- |
| [![latest-release](https://img.shields.io/github/v/release/anjaustin/mnemo?display_name=tag&sort=semver)](https://github.com/anjaustin/mnemo/releases/latest) | [![release-assets](https://img.shields.io/badge/assets-linux--amd64%20%7C%20tar.gz%20%7C%20sha256-2da44e)](https://github.com/anjaustin/mnemo/releases/latest) | [![ghcr-package](https://img.shields.io/badge/ghcr-mnemo--server-1f6feb)](https://github.com/anjaustin/mnemo/pkgs/container/mnemo%2Fmnemo-server) |

![Mnemosyne](img/mnemosyne.gif)

**Memory infrastructure for production AI agents.**

Mnemo is a free, open-source, self-hosted memory and context engine for agent systems. It is built in Rust, uses Redis and Qdrant, and focuses on temporal correctness, fast recall, and operational simplicity.

## Who Mnemo is for

- Teams shipping assistants or autonomous agents that need memory with auditability and temporal truth.
- Builders who want self-hosted control, not a managed black box.
- Engineering orgs that care about hard quality gates and reproducible evaluation.

## Deploy Mnemo

All 10 targets fully falsified (5-gate test: health, write, context, list-episodes, delete). Production Helm chart for Kubernetes. Deployment guides in `deploy/` and `docs/DEPLOY.md`.

| Docker | AWS | GCP | DigitalOcean | Render |
|:------:|:---:|:---:|:------------:|:------:|
| [![Deploy with Docker][docker-btn]][docker-deploy] | [![Deploy on AWS][aws-btn]][aws-deploy] | [![Deploy on GCP][gcp-btn]][gcp-deploy] | [![Deploy on DigitalOcean][do-btn]][do-deploy] | [![Deploy on Render][render-btn]][render-deploy] |

| Railway | Vultr | Northflank | Linode | Kubernetes |
|:-------:|:-----:|:----------:|:------:|:----------:|
| [![Deploy on Railway][railway-btn]][railway-deploy] | [![Deploy on Vultr][vultr-btn]][vultr-deploy] | [![Deploy on Northflank][northflank-btn]][northflank-deploy] | [![Deploy on Linode][linode-btn]][linode-deploy] | [![Deploy on Kubernetes][k8s-btn]][k8s-deploy] |

[docker-btn]: ./img/deploy/docker.svg
[docker-deploy]: deploy/docker/DEPLOY.md
[aws-btn]: ./img/deploy/aws.svg
[aws-deploy]: deploy/aws/cloudformation/DEPLOY.md
[gcp-btn]: https://deploy.cloud.run/button.svg
[gcp-deploy]: deploy/gcp/DEPLOY.md
[do-btn]: https://www.deploytodo.com/do-btn-blue.svg
[do-deploy]: deploy/digitalocean/DEPLOY.md
[render-btn]: https://render.com/images/deploy-to-render-button.svg
[render-deploy]: deploy/render/DEPLOY.md
[railway-btn]: https://railway.app/button.svg
[railway-deploy]: deploy/railway/DEPLOY.md
[vultr-btn]: ./img/deploy/vultr.svg
[vultr-deploy]: deploy/vultr/DEPLOY.md
[northflank-btn]: https://assets.northflank.com/deploy_to_northflank_smm_36700fb050.svg
[northflank-deploy]: deploy/northflank/DEPLOY.md
[linode-btn]: ./img/deploy/linode.svg
[linode-deploy]: deploy/linode/DEPLOY.md
[k8s-btn]: ./img/deploy/kubernetes.svg
[k8s-deploy]: docs/DEPLOY.md

[or set up a production Mnemo instance without Docker →](deploy/bare-metal/DEPLOY.md) | [Kubernetes / Helm →](docs/DEPLOY.md)

## Why teams choose Mnemo

- **Temporal memory, not static notes**: facts can be superseded while preserving history for point-in-time recall (`docs/TEMPORAL_VECTORIZATION.md`).
- **Fast context assembly**: hybrid retrieval and pre-assembled context blocks optimized for LLM prompts, with both REST and gRPC APIs on the same port (`docs/ARCHITECTURE.md`).
- **Enterprise access control**: RBAC with scoped API keys, data classification labels, policy-scoped memory views, a guardrails engine, multi-agent shared memory regions with ACLs, and agent promotion governance with approval workflows.
- **Agent identity controls**: identity core, experience weighting, versioning, audit, rollback, and promotion flow (`docs/AGENT_IDENTITY_SUBSTRATE.md`).
- **Proof over claims**: benchmark harness plus falsification and CI gates are first-class (`docs/EVALUATION.md`, `docs/COMPETITIVE.md`, `.github/workflows/quality-gates.yml`).

## Competitive Feature Matrix

How Mnemo compares to the three leading AI memory systems. Assessed feature-by-feature against public documentation, GitHub repos, and SDK references as of March 2026. Updated for v0.7.0 (OpenTelemetry, BYOK encryption, OpenAPI spec, Helm chart, red-team hardening).

> Legend: :green_circle: Shipped  :yellow_circle: Partial / limited  :red_circle: Not available

### Memory & Retrieval (20 features)

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

### Agent Architecture (10 features)

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

### Compute & Observability (9 features)

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

### Governance & Compliance (12 features)

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

### Webhooks & Events (6 features)

| Feature | Mnemo | Zep | Mem0 | Letta |
|---|:---:|:---:|:---:|:---:|
| Memory lifecycle webhooks | :green_circle: | :green_circle: | :green_circle: | :yellow_circle: |
| Webhook management via API | :green_circle: | :yellow_circle: | :green_circle: | :red_circle: |
| Dead-letter queue + replay + retry | :green_circle: | :yellow_circle: | :red_circle: | :yellow_circle: |
| HMAC signature verification | :green_circle: | :green_circle: | :red_circle: | :red_circle: |
| Circuit breaker + rate limiting | :green_circle: | :yellow_circle: | :yellow_circle: | :red_circle: |
| Webhook delivery stats + audit | :green_circle: | :green_circle: | :red_circle: | :red_circle: |

### SDKs & Integrations (12 features)

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

### Deployment & Operations (8 features)

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

### Scorecard

| | Mnemo | Zep | Mem0 | Letta |
|---|---:|---:|---:|---:|
| :green_circle: Shipped | **64** | **38** | **26** | **22** |
| :yellow_circle: Partial | **2** | **14** | **19** | **11** |
| :red_circle: Not available | **11** | **25** | **32** | **44** |
| **Total features** | **77** | **77** | **77** | **77** |

### Honesty notes

We take accuracy seriously. Every claim below has a source link or caveat.

- **Mnemo SOC 2**: Technical controls are implemented and mapped to Trust Service Criteria. No formal audit has been conducted yet.
- **Mnemo human-in-the-loop**: Agent promotion governance has quorum-based approval. Mnemo does not have per-tool HITL gating like Letta.
- **Mnemo reranking caveat**: MMR uses score-proximity approximation rather than full embedding dot-product.
- **Mnemo multi-modal**: Not supported. Text-only memory ingestion. This is a genuine gap.
- **Mnemo managed cloud**: Not yet available. Mnemo is self-hosted only. This is a genuine gap for teams that prefer managed infrastructure.
- **Mnemo CORS**: `CorsLayer::permissive()` in production. Intentional for development; needs environment-based origin configuration for production deployments.
- **Mnemo auth-exempt routes**: Unauthenticated routes (health, swagger, dashboard) receive a synthetic admin `CallerContext`. Functionally safe but architecturally unsound — needs refactor to distinguish "no auth required" from "admin by default."
- **Mnemo OpenAPI paths**: The OpenAPI 3.1 spec registers schemas but has zero `#[utoipa::path]` annotations on handlers, so the spec contains no endpoint documentation yet. Swagger UI loads but shows only models.
- **Mnemo BYOK key rotation**: BYOK supports a single `key_id` but has no multi-key decryption or rotation workflow. Rotating keys currently requires re-encrypting all data. This is a genuine gap for compliance-sensitive deployments.
- **Mnemo OTLP security**: The OpenTelemetry exporter connects to the collector over plaintext gRPC. No TLS or bearer token auth configuration is available yet.
- **Mnemo internal types in OpenAPI**: `utoipa` derives on internal structs (e.g., `GraphNode`, `RedisEdge`) expose implementation details in the schema. Needs a DTO separation layer.
- **Mnemo Helm Qdrant auth**: Qdrant subchart does not expose an auth toggle. Qdrant runs unauthenticated inside the cluster by default.
- **Mnemo Helm Ingress TLS**: Ingress template has no TLS block. Requires cluster-specific setup (cert-manager, cloud LB).
- **Mnemo Helm NetworkPolicy**: No NetworkPolicy template. Pod-to-pod traffic is unrestricted within the namespace.
- **Mnemo Helm subchart seccomp**: Only the Mnemo deployment has `seccompProfile: RuntimeDefault`. Redis and Qdrant pods inherit cluster defaults.
- **Mnemo dead config sections**: `[graph]` and `[retention]` sections in `config/default.toml` are parsed but silently ignored. Not a security issue; retention is enforced per-user via the policy API.
- **Zep self-hosted**: [Graphiti](https://github.com/getzep/graphiti) is now fully open-source (Apache 2.0, 23.7k stars) with Neo4j, FalkorDB, Kuzu, and Neptune backends. Prior "partial" rating was stale. Zep Cloud remains the managed platform.
- **Zep graph traversal**: BFS shipped (`bfs_origin_node_uuids`), plus node-distance reranker. No shortest-path API.
- **Zep reranking**: Five rerankers: RRF, MMR, cross-encoder, node_distance, episode_mentions. Full parity.
- **Zep community detection**: Leiden algorithm in Graphiti with incremental community updates.
- **Zep temporal**: Full bi-temporal model (`created_at`, `valid_at`, `invalid_at`, `expired_at`). Datetime filters on edge search.
- **Zep memory diff**: No first-class `changes_since` endpoint, but datetime filters on `created_at`/`expired_at` achieve equivalent results.
- **Zep contradiction detection**: Automatic fact invalidation when conflicting information is ingested.
- **Zep tracing**: Graphiti has full OpenTelemetry tracing support (configurable tracer, span prefixes). Zep Cloud has API logging and debug logs.
- **Zep sleep-time**: Graph construction and community building are async/offline. No explicit "sleep-time compute" workflow.
- **Zep agent self-modification**: Full CRUD API allows agents to create, update, and delete graph nodes/edges/episodes.
- **Zep confidence decay**: `recency_weight` with configurable half-lives (7d/30d/90d). No explicit confidence scores or formal revalidation workflow.
- **Zep narrative summaries**: Entity summaries evolve across sessions. Community summaries aggregate across interactions.
- **Zep local embeddings**: Graphiti supports Ollama for local embeddings (e.g. `nomic-embed-text`). Zep Cloud requires cloud embedders.
- **Zep webhook management**: Configured via dashboard UI, not via API. Replay and activity logs available.
- **Zep shared memory regions**: Group graphs for cross-user sharing. RBAC at account/project level but no per-graph agent ACLs.
- **Zep vector backends**: Embeddings are pluggable (OpenAI, Voyage, Ollama) but vector storage is tied to the graph backend's native index.
- **Mem0 graph**: Full entity/edge extraction via LLM into Neo4j/Memgraph/Neptune/Kuzu. Returns `relations` array. Not a traversable graph API (no BFS/shortest-path), but entities and relationships are first-class.
- **Mem0 contradiction**: Built into `add()` pipeline — detects and resolves conflicts via LLM. Internal process, not a user-facing API.
- **Mem0 reranking**: Cross-encoder reranking with 5 providers (Cohere, Sentence Transformer, HuggingFace, LLM Reranker, Zero Entropy). No RRF or MMR fusion.
- **Mem0 temporal**: Custom timestamps on `add()`, time-based filters (`created_at`, `updated_at` with operators). No `as_of` point-in-time query.
- **Mem0 data classification**: Custom categories (default 15 or user-defined) serve as classification labels. Trust center confirms "Data classification and access control."
- **Mem0 RBAC**: Org/project/workspace scoping with API keys. Trust center shows access review controls. No documented role definitions with explicit permissions.
- **Mem0 HIPAA**: Trust center shows HIPAA as Compliant with BAA available.
- **Mem0 BYOK**: Supported. Open-source deployment uses your own keys by design.
- **Mem0 SOC 2**: Type I completed. Type II in progress per trust center.
- **Mem0 shared memory**: OpenMemory + scoping by user_id/agent_id enables cross-agent sharing. No explicit ACLs.
- **Mem0 webhooks**: SDK-managed CRUD for webhooks. No HMAC signing or delivery stats documented.
- **Mem0 multi-modal**: Images (JPG, PNG), documents (MDX, TXT), and PDFs supported.
- **Mem0 CLI**: OpenMemory has a CLI (`npx @openmemory/install`). No dedicated `mem0` CLI for memory operations.
- **Letta sleep-time**: Background "sleep-time agents" process conversation history and write learned context to memory blocks. Research paper published (arXiv:2504.13171).
- **Letta context assembly**: Character-based limits on memory blocks (`chars_current`/`chars_limit`), not token-based budgeting.
- **Letta hybrid retrieval**: Separate vector search (archival) and full-text search (conversation) tools. Not a unified pipeline.
- **Letta multi-agent communication**: Three built-in tools for async, sync, and tag-based messaging. Supervisor-worker patterns documented.
- **Letta self-modification**: Core architecture — agents use `memory_insert`, `memory_replace`, `memory_rethink` tools.
- **Letta HITL**: Per-tool and per-agent approval configuration with approve/deny + reasons.
- **Letta RBAC**: Listed as Enterprise-only feature on pricing page. Not available in OSS.
- **Letta identity**: Memory blocks function as a mutable persona, but no versioning, rollback, or audit trail.
- **Letta tracing**: OTel collector configs for ClickHouse, SigNoz, and file exporters in repo. Runs & Steps model provides structured execution tracing.
- **Letta narrative summaries**: Sleep-time agents generate "learned context" summaries. LLM-driven reflection, not a structured API.
- **Letta multi-modal**: Document processing via code interpreter. No native image embeddings in archival memory.
- **Letta webhooks**: Step-complete webhook via env var config. No memory lifecycle events. With Temporal: retry + replay.
- **Letta CLI**: `@letta-ai/letta-code` npm CLI for terminal-based agent development.
- **Letta BYOK**: Explicitly listed on pricing — "Connect your own LLM API keys."
- **Letta local embeddings**: Docker deployment supports Ollama for local embeddings.
- **Letta vector backends**: pgvector only. You bring your own Postgres but no swap to Pinecone/Qdrant/etc.

## Core Capabilities

- **Temporal Knowledge Graph** - Automatically extracts entities and relationships and tracks how facts change over time.
- **Bi-temporal Retrieval** - Answers both "what is true now" and "what was true then".
- **Thread HEAD + Metadata Planner** - Improves relevance with deterministic head selection and metadata prefilter controls.
- **Identity-aware Context** - Balances stable identity with recent experience signals.
- **Chat History Importer** - Migrates existing histories with async jobs, dry-run validation, and idempotent replay protection.
- **Memory Lifecycle Webhooks** - Proactively emits `head_advanced`, `conflict_detected`, `fact_added`, and `fact_superseded` events as mutations occur during ingestion. All events include retry/backoff delivery and optional HMAC signatures.
- **Time Travel Trace** - Compares memory snapshots across two points in time and returns timeline-level "why it changed" evidence.
- **Time Travel Summary** - Returns fast gained/lost fact and episode counters for first-pass RCA.
- **Governance Policies** - Per-user retention defaults, webhook domain allowlists, and audit trails for policy/destructive operations.
  - Policy preview and violation-window query endpoints for safer rollout dry-runs and incident triage.
  - Default contract/retrieval policy fallback and retention enforcement for episode writes.
- **Operator Endpoints** - Dashboard summary, request-id trace lookup, and drill automation for dead-letter recovery, RCA, and governance workflows.
- **Python SDK** - Zero-dependency sync client (`Mnemo`) and async client (`AsyncMnemo`) with full API coverage, typed results, and `x-mnemo-request-id` propagation.
  - **LangChain adapter** - Drop-in `MnemoChatMessageHistory` (`BaseChatMessageHistory`) via `mnemo.ext.langchain`.
  - **LlamaIndex adapter** - Drop-in `MnemoChatStore` (`BaseChatStore`, all 7 abstract methods) via `mnemo.ext.llamaindex`.
- **TypeScript SDK** - Fetch-based client with full API parity. Works in Node.js, Deno, Bun, and modern browsers.
  - **LangChain.js adapter** - Drop-in `MnemoChatMessageHistory` (`BaseListChatMessageHistory`) via `mnemo-client/langchain`.
  - **Vercel AI SDK adapter** - `mnemoRemember`, `mnemoRecall`, `mnemoDigest` tools via `mnemo-client/vercel-ai`.
- **Raw Vector API** - General-purpose vector database endpoints for external integrations (upsert, similarity search, delete, count, namespace lifecycle).
- **AnythingLLM Integration** - Drop-in vector DB provider for [AnythingLLM](https://github.com/Mintplex-Labs/anything-llm) (55.5k stars). See `integrations/anythingllm/`.
- **LLM Agnostic** - Works with Anthropic, OpenAI, Ollama, Liquid AI, or no external LLM.
- **Scoped API Keys (RBAC)** - Role-based access control (`read`/`write`/`admin`) with optional user, agent, and classification scoping. Key rotation and revocation via API.
- **Data Classification** - Four-tier labeling (`public`/`internal`/`confidential`/`restricted`) on entities and edges, enforced at retrieval time.
- **Memory Views** - Named, reusable access policies that filter context by classification ceiling, entity types, edge labels, temporal scope, and fact count.
- **Memory Guardrails** - Composable rule engine with condition predicates (classification, confidence, age, regex, role) and actions (block, redact, reclassify, audit, warn). Dry-run evaluation endpoint.
- **Multi-Agent Shared Memory** - Memory regions with per-agent ACLs (`read`/`write`/`manage`), owner-only mutation, optional expiry, and lazy cleanup of stale grants.
- **Agent Promotion Governance** - Proposal-based approval workflows for agent identity changes with configurable quorum, cooling periods, auto-reject deadlines, and conflict analysis.
- **MCP Server** - Model Context Protocol over stdio transport with 7 tools and 2 resource templates for Claude Code and compatible clients.
- **OpenAPI 3.1 Spec + Swagger UI** - Machine-readable API specification with embedded Swagger UI at `/swagger-ui/`. CDN pinned for supply-chain safety.
- **OpenTelemetry Export** - OTLP trace export with graceful fallback to console-only tracing when the collector is unavailable.
- **BYOK Envelope Encryption** - AES-256-GCM at-rest encryption for Redis state with customer-managed keys. Key ID rotation support. Intermediate buffers zeroized.
- **Production Helm Chart** - HA-ready Kubernetes deployment with Redis and Qdrant subcharts, security-hardened defaults (seccompProfile, SA automount disabled, emptyDir sizeLimit), and configurable replicas.
- **gRPC API** - 3 services, 8 RPCs on the same port as REST. Proto3 with optional fields, streaming support, and full red-team hardening.
- **Multi-tenant + Self-hosted** - Per-user isolation and deploy-it-yourself control.

## Quality Gates

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --lib --bins`
- `cargo test -p mnemo-storage --test storage -- --test-threads=1`
- `cargo test -p mnemo-ingest --test ingest -- --test-threads=1`
- `cargo test -p mnemo-server --test memory_api -- --test-threads=1`
- `bash tests/e2e_smoke.sh http://localhost:8080` (server running)
- `bash tests/operator_p0_drills.sh`

Reference CI gate: `.github/workflows/quality-gates.yml`.

Nightly soak and flake-detection workflow: `.github/workflows/nightly-soak.yml`.

## Releases and Packages

- Tags matching `v*.*.*` trigger automated GitHub Releases via `.github/workflows/release.yml`.
- Release workflow expectation: bump `Cargo.toml` (`workspace.package.version`) and `sdk/python/pyproject.toml` together before tagging.
- Current in-repo development version: `0.7.0`.
- Release artifacts include:
  - `mnemo-server-<version>-linux-amd64`
  - `mnemo-server-<version>-linux-amd64.tar.gz`
  - `SHA256SUMS.txt`
- Docker images are published to GHCR via `.github/workflows/package-ghcr.yml` on `main` and version tags.
- Published image namespace: `ghcr.io/anjaustin/mnemo/mnemo-server`.

Get latest release assets:

```bash
gh release download --repo anjaustin/mnemo --pattern 'mnemo-server-*' --pattern 'SHA256SUMS.txt'
```

Pull package images:

```bash
# latest default-branch image
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:latest

# immutable tag image
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:<version>

# branch image (main)
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:main
```

## Measured Performance and Evaluation

- Temporal eval harness: `eval/temporal_eval.py`
- Evaluation playbook and metrics: `docs/EVALUATION.md`
- Competitive methodology and scorecard format: `docs/COMPETITIVE.md`

Quick benchmark commands:

- Mnemo only: `python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080`
- Mnemo vs Zep: `python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key`
- Scientific research pack (Mnemo): `python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080`
- Scientific research pack v2 (Mnemo): `python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080`
- Importer stress harness (ChatGPT export zip): `python3 eval/import_stress.py --mode dry-run --iterations 2 --base-url http://localhost:8080`

## Quick Start

```bash
git clone https://github.com/anjaustin/mnemo.git
cd mnemo

# Set your LLM API key (optional — works without it)
cp .env.example .env
# Edit .env with your API key

# Start Redis + Qdrant
docker compose up -d redis qdrant

# Start Mnemo
cargo run --bin mnemo-server

# Verify
curl http://localhost:8080/health
```

For a Python-first flow, see [QUICKSTART.md](QUICKSTART.md).

## Usage

All interaction is via REST API.

### Start here: High-Level Memory API

Use these two endpoints when you just want to remember and recall.

```bash
# Remember
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"acct_mgr_jordan","text":"Acme Corp renewal is due on 2025-09-30 and procurement requires SOC 2 Type II before signature."}'

# Recall
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What are the renewal blockers for Acme?","contract":"default","retrieval_policy":"balanced"}'

# Diff what changed between two points in time
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/changes_since \
  -H "Content-Type: application/json" \
  -d '{"from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'

# Detect active contradiction clusters
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/conflict_radar \
  -H "Content-Type: application/json" \
  -d '{}'

# Explain why memory was retrieved
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/causal_recall \
  -H "Content-Type: application/json" \
  -d '{"query":"Why do we think Acme has legal risk this quarter?"}'

# Trace why an answer changed over time
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/time_travel/trace \
  -H "Content-Type: application/json" \
  -d '{"query":"How did Acme renewal risk evolve?","from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z","contract":"historical_strict"}'

# Lightweight summary for fast first-pass RCA
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/time_travel/summary \
  -H "Content-Type: application/json" \
  -d '{"query":"How did Acme renewal risk evolve?","from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'

# Register a webhook for memory lifecycle events
curl -X POST http://localhost:8080/api/v1/memory/webhooks \
  -H "Content-Type: application/json" \
  -d '{
    "user":"acct_mgr_jordan",
    "target_url":"https://example.com/hooks/memory",
    "signing_secret":"whsec_demo",
    "events":["head_advanced","conflict_detected"]
  }'

# Inspect retained event delivery status
curl http://localhost:8080/api/v1/memory/webhooks/WEBHOOK_ID/events?limit=10

# Set user governance policy (allowlist + retention defaults)
curl -X PUT http://localhost:8080/api/v1/policies/acct_mgr_jordan \
  -H "Content-Type: application/json" \
  -d '{"webhook_domain_allowlist":["hooks.acme.example"],"retention_days_message":365}'

# Preview policy impact before applying
curl -X POST http://localhost:8080/api/v1/policies/acct_mgr_jordan/preview \
  -H "Content-Type: application/json" \
  -d '{"retention_days_message":30}'

# Query policy violations inside a time window
curl "http://localhost:8080/api/v1/policies/acct_mgr_jordan/violations?from=2026-03-01T00:00:00Z&to=2026-03-04T00:00:00Z&limit=50"
```

### Full workflow: Users, Sessions, Episodes

Use this flow when you need explicit user/session lifecycle control.

```bash
# 1. Create a user
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Jordan Lee", "email": "jordan.lee@acme-revenueops.com"}'

# 2. Start a session
curl -X POST http://localhost:8080/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"user_id": "USER_ID_FROM_STEP_1"}'

# 3. Add messages
curl -X POST http://localhost:8080/api/v1/sessions/SESSION_ID/episodes \
  -H "Content-Type: application/json" \
  -d '{"type":"message","role":"user","name":"Jordan Lee","content":"Acme legal approved redlines but procurement still needs SOC 2 evidence before renewal."}'

# 4. Wait a moment for processing, then get context
curl -X POST http://localhost:8080/api/v1/users/USER_ID/context \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"What still blocks Acme renewal?"}]}'
```

Inject the returned `context` string into your agent's system prompt. That's it.

### Import existing chat history

Mnemo supports async import jobs for existing chat logs.

Supported sources: `ndjson`, `chatgpt_export`, `gemini_export`.

```bash
# Start an import job (ndjson source)
curl -X POST http://localhost:8080/api/v1/import/chat-history \
  -H "Content-Type: application/json" \
  -d '{
    "user": "acct_mgr_jordan",
    "source": "ndjson",
    "idempotency_key": "import-001",
    "dry_run": false,
    "default_session": "Imported History",
    "payload": [
      {"role": "user", "content": "Acme procurement requested SOC 2 report by Friday.", "created_at": "2025-02-01T10:00:00Z"},
      {"role": "assistant", "content": "Acknowledged. I will track this as a renewal blocker.", "created_at": "2025-02-01T10:00:05Z"}
    ]
  }'

# Poll job status
curl http://localhost:8080/api/v1/import/jobs/JOB_ID
```

### Python SDK

Install:

```bash
pip install git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python

# With async support (aiohttp)
pip install "mnemo-client[async] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"

# With LangChain adapter
pip install "mnemo-client[langchain] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"

# With LlamaIndex adapter
pip install "mnemo-client[llamaindex] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"
```

Basic usage:

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080")

# Remember
client.add("jordan", "Acme renewal is at risk — procurement needs SOC 2 before signature.")

# Recall
ctx = client.context("jordan", "What is blocking Acme renewal?")
print(ctx.text)  # inject into agent system prompt
```

LangChain adapter:

```python
from mnemo import Mnemo
from mnemo.ext.langchain import MnemoChatMessageHistory

client = Mnemo("http://localhost:8080")
history = MnemoChatMessageHistory(session_name="acme-deal-chat", user_id="jordan", client=client)

history.add_user_message("What are the Acme renewal blockers?")
history.add_ai_message("SOC 2 evidence is still required by procurement.")

print(history.messages)  # [HumanMessage(...), AIMessage(...)]
history.clear()
```

LlamaIndex adapter:

```python
from mnemo import Mnemo
from mnemo.ext.llamaindex import MnemoChatStore
from llama_index.core.llms import ChatMessage, MessageRole

client = Mnemo("http://localhost:8080")
store = MnemoChatStore(client=client, user_id="jordan")

store.add_message("acme-session", ChatMessage(role=MessageRole.USER, content="What blocks renewal?"))
msgs = store.get_messages("acme-session")
```

## Architecture

```
Agent Runtime
    │
    ▼
REST API (mnemo-server)
    │
    ├── Redis   (users, sessions, episodes, graph state)
    └── Qdrant  (vector index for semantic retrieval)
```

Mnemo is a single Rust binary with Redis + Qdrant as backing services.

### Write Path

```
Client message
  -> /api/v1/memory or /api/v1/sessions/:id/episodes or /api/v1/import/chat-history
  -> episode persisted in Redis
  -> ingest worker extracts entities/edges
  -> graph updated in Redis + embeddings upserted to Qdrant
```

### Recall Path

```
Client query
  -> /api/v1/memory/:user/context or /api/v1/users/:id/context
  -> retrieval planner (metadata + temporal intent)
  -> hybrid search (semantic + graph + lexical fallback)
  -> token-budgeted context assembled for the agent prompt
```

### Event Path

```text
Memory lifecycle event
  -> webhook subscription match (user + event_type)
  -> async outbound POST to target_url
  -> exponential retry/backoff on non-2xx
  -> delivery telemetry retained in /api/v1/memory/webhooks/:id/events
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full deep dive.

## How Temporal Memory Works

Most memory systems overwrite facts. Mnemo keeps the timeline.

### Before and After

```text
Before (flat memory)
  "Acme renewal status is green"
  "Acme renewal status is at risk"
  -> no clear answer to "what was true in 2024?"

After (temporal memory)
  Aug 2024: Acme -> renewal_status -> green    (valid)
  Feb 2025: Acme -> renewal_status -> green    (invalidated)
  Feb 2025: Acme -> renewal_status -> at_risk  (valid)
```

Mnemo tracks *when* facts became true and *when* they were superseded:

```
Aug 2024: "Acme legal and procurement are aligned; renewal looks green."
  → Acme ──renewal_status──▶ green  (valid_at: Aug 2024)

Feb 2025: "Procurement blocked signature pending SOC 2 report. Renewal is now at risk."
  → Acme ──renewal_status──▶ green    (invalid_at: Feb 2025)  ← superseded
  → Acme ──renewal_status──▶ at_risk  (valid_at: Feb 2025)    ← current
```

Old facts aren't deleted. This enables point-in-time queries and change tracking.

### Real API Example

```bash
# 1) Initial preference
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"acct_mgr_jordan","text":"Acme renewal status is green and legal has no open issues."}'

# 2) Later correction
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"acct_mgr_jordan","text":"Acme renewal is now at risk because procurement requires SOC 2 evidence before signature."}'

# 3) Ask for current truth
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What is Acme renewal status now?"}'

# 4) Ask for historical truth
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What was Acme renewal status before procurement blocked signature?"}'
```

## Production Readiness Checklist

- Enable API auth and provision keys before exposing Mnemo externally.
- Run Redis and Qdrant with persistent volumes and backup policy.
- Pin release versions (`v*.*.*`) for server binaries or container tags.
- Run the full quality gate stack in CI on every merge.
- Track evaluation drift with the temporal harness on a fixed dataset cadence.
- Keep `CHANGELOG.md`, `docs/OPERATOR_UX_EXECUTION_BACKLOG.md`, and integration READMEs updated with shipped behavior.

## Documentation

| Document | Description |
|----------|-------------|
| [Kubernetes Deployment](docs/DEPLOY.md) | Helm chart install, HA configuration, values reference |
| [Deployment PRD](docs/PRD_DEPLOY.md) | T1–T10 deployment targets (all 10 falsified), gates, rollout phasing |
| [API Reference](docs/API.md) | Every endpoint with request/response examples |
| [Architecture](docs/ARCHITECTURE.md) | Data model, temporal reasoning, pipeline internals |
| [Phase 2 PRD](docs/PHASE_2_PRD.md) | Productization plan for temporal memory and proof gates |
| [Evaluation Playbook](docs/EVALUATION.md) | Reproducible temporal quality and latency measurements |
| [Competitive Plan](docs/COMPETITIVE.md) | Cross-system benchmark methodology and scorecard |
| [Chat Import Guide](docs/IMPORTING_CHAT_HISTORY.md) | Import formats, idempotency, dry run, and migration examples |
| [Webhook Delivery Guide](docs/WEBHOOKS.md) | Event types, retry semantics, and signature verification examples |
| [P0 Ops Control Plane PRD](docs/P0_OPS_CONTROL_PLANE_PRD.md) | Cloud-grade ops goals, scope, falsification matrix, and rollout criteria |
| [Operator UX PRD](docs/OPERATOR_UX_PRD.md) | Control-plane UX strategy, screens, metrics, and phased rollout |
| [Operator UX Backlog](docs/OPERATOR_UX_EXECUTION_BACKLOG.md) | Ticketized execution plan for the two hero operator lanes |
| [SDK Integrations PRD](docs/SDK_INTEGRATIONS_PRD.md) | Python SDK rebuild, LangChain adapter, LlamaIndex adapter |
| [Operator Dashboard PRD](docs/OPERATOR_DASHBOARD_PRD.md) | Embedded zero-deployment operator dashboard |
| [AnythingLLM Integration](integrations/anythingllm/README.md) | Drop-in vector DB provider for AnythingLLM |
| [Domain Readiness Matrix](docs/DOMAIN_READINESS_MATRIX.md) | Domain-by-domain readiness and 30/60/90 roadmap |
| [Agent Identity Substrate](docs/AGENT_IDENTITY_SUBSTRATE.md) | Implemented P0 design for stable identity + adaptive experience |
| [Thread HEAD](docs/THREAD_HEAD.md) | Git-like current thread state and retrieval modes |
| [Temporal Vectorization](docs/TEMPORAL_VECTORIZATION.md) | Time-aware retrieval scoring and rollout plan |
| [Testing Guide](docs/TESTING.md) | Workspace, E2E, and falsification test commands |
| [QA/QC Falsification PRD](docs/QA_QC_FALSIFICATION_PRD.md) | 25 domains, ~170 falsification gates, 3-phase plan |
| [Benchmarks](docs/BENCHMARKS.md) | Latency, throughput, and comparison benchmarks |
| [Metadata Index Layer](docs/METADATA_INDEX_LAYER.md) | App-level metadata prefilter planner design |
| [Configuration](config/default.toml) | All config options with inline comments |
| [Tutorial](docs/TUTORIAL.md) | Build a support agent with Mnemo memory (20-min walkthrough) |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Common issues and solutions |
| [Contributing](CONTRIBUTING.md) | Dev setup, code style, PR process |
| [Changelog](CHANGELOG.md) | Release notes |
| [Security Policy](SECURITY.md) | Vulnerability reporting and disclosure |
| [Code of Conduct](CODE_OF_CONDUCT.md) | Community standards |

## Configuration

Mnemo reads `config/default.toml` and overrides with environment variables:

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_LLM_API_KEY` | API key for entity extraction | (none) |
| `MNEMO_LLM_PROVIDER` | `anthropic`, `openai`, `ollama`, `liquid` | `anthropic` \* |
| `MNEMO_LLM_MODEL` | Model for extraction | `claude-sonnet-4-20250514` \* |
| `MNEMO_EMBEDDING_PROVIDER` | `openai`-compatible remote embeddings or `local` fastembed | `openai` |
| `MNEMO_EMBEDDING_API_KEY` | Embedding API key | (none) |
| `MNEMO_AUTH_ENABLED` | Require API key auth (`true`/`false`) | `false` |
| `MNEMO_AUTH_API_KEYS` | Comma-separated accepted API keys | (none) |
| `MNEMO_REDIS_URL` | Redis connection | `redis://localhost:6379` |
| `MNEMO_QDRANT_URL` | Qdrant connection | `http://localhost:6334` |
| `MNEMO_QDRANT_PREFIX` | Qdrant collection prefix / namespace | `mnemo_` |
| `MNEMO_METADATA_PREFILTER_ENABLED` | Enable metadata prefilter planner | `true` |
| `MNEMO_METADATA_SCAN_LIMIT` | Candidate scan limit for prefilter planner | `400` |
| `MNEMO_METADATA_RELAX_IF_EMPTY` | Relax strict metadata filters when empty | `false` |
| `reranker` (TOML only) | Retrieval reranking strategy: `rrf` or `mmr` | `rrf` |
| `MNEMO_WEBHOOKS_ENABLED` | Enable outbound webhook delivery | `true` |
| `MNEMO_WEBHOOKS_MAX_ATTEMPTS` | Retry attempts before dead-lettering | `3` |
| `MNEMO_WEBHOOKS_BASE_BACKOFF_MS` | Base backoff duration for retries | `200` |
| `MNEMO_WEBHOOKS_TIMEOUT_MS` | Per-attempt request timeout | `3000` |
| `MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK` | Max retained event rows per webhook | `1000` |
| `MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE` | Max outbound sends per webhook per minute | `120` |
| `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD` | Consecutive failures before opening circuit | `5` |
| `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS` | Circuit cooldown before retrying sends | `60000` |
| `MNEMO_WEBHOOKS_PERSISTENCE_ENABLED` | Persist webhook subscriptions/events in Redis | `true` |
| `MNEMO_WEBHOOKS_PERSISTENCE_PREFIX` | Redis key suffix for webhook state | `webhooks` |
| `MNEMO_SESSION_SUMMARY_THRESHOLD` | Episodes per session before progressive summarization triggers (0 = disabled) | `10` |
| `MNEMO_SERVER_HOST` | Server bind address | `0.0.0.0` |
| `MNEMO_SERVER_PORT` | Server port | `8080` |
| `MNEMO_LLM_BASE_URL` | Base URL for LLM provider | Provider default |
| `MNEMO_EMBEDDING_MODEL` | Model for embedding generation | `text-embedding-3-small` |
| `MNEMO_EMBEDDING_BASE_URL` | Base URL for embedding provider | Provider default |
| `MNEMO_EMBEDDING_DIMENSIONS` | Embedding vector dimensions | `1536` |
| `MNEMO_CONFIG` | Path to custom TOML config file (overrides `config/default.toml`) | (none) |
| `MNEMO_SLEEP_ENABLED` | Enable background sleep-time compute (digest generation, re-ranking) | `true` |
| `MNEMO_SLEEP_IDLE_WINDOW_SECONDS` | Seconds of user inactivity before triggering background tasks | `300` |
| `MNEMO_REQUIRE_TLS` | Reject non-HTTPS webhook targets | `false` |
| `MNEMO_AUDIT_SIGNING_SECRET` | HMAC secret for signing audit export responses (SOC 2 compliance) | (none) |
| `MNEMO_ENCRYPTION_ENABLED` | Enable AES-256-GCM at-rest encryption for Redis state | `false` |
| `MNEMO_ENCRYPTION_MASTER_KEY` | Base64-encoded 32-byte master encryption key | (none) |
| `MNEMO_ENCRYPTION_KEY_ID` | Identifier for the active encryption key (rotation support) | `default` |
| `MNEMO_OTEL_ENABLED` | Enable OpenTelemetry OTLP trace export | `false` |
| `MNEMO_OTEL_ENDPOINT` | OTLP gRPC collector endpoint | `http://localhost:4317` |
| `MNEMO_OTEL_SERVICE_NAME` | Service name reported in traces | `mnemo` |

For cloud targets that do not have a managed embedding API available, Mnemo also supports a self-hosted embedding path:

- `MNEMO_EMBEDDING_PROVIDER=local`
- `MNEMO_EMBEDDING_MODEL=AllMiniLML6V2`
- `MNEMO_EMBEDDING_DIMENSIONS=384`
- `MNEMO_QDRANT_PREFIX=<provider-specific-prefix>` to avoid collection-dimension clashes during migrations or side-by-side rollouts

\* Defaults shown are from `config/default.toml`. Without a config file, the compiled-in defaults are `openai` / `gpt-4o-mini`. In practice, you should always set these env vars explicitly or load the provided `default.toml` via `MNEMO_CONFIG`.

Webhook outbound delivery defaults are configured in `config/default.toml` and can be overridden with env vars:

- `max_attempts=3`
- `base_backoff_ms=200`
- `request_timeout_ms=3000`

## Project Status

**Phase 1.5 — Production Hardening** ✅ complete

- compilation + integration coverage
- auth middleware
- full-text + hybrid retrieval
- memory API + falsification CI gate

**Phase 2 — Temporal Productization** ✅ complete

- M1 Thread HEAD completion ✅
- M2 Temporal retrieval v2 diagnostics ✅
- M3 Metadata index layer ✅
- M4 Competitive publication v1 ✅
- M5 Agent Identity Substrate P0 ✅

See `docs/PHASE_2_PRD.md` for milestones.

**Phase 2 Deployment — Cloud IaC** ✅ complete (10/10 targets falsified)

- T1 Docker production compose ✅
- T2 Bare Metal systemd + nginx ✅
- T3 AWS CloudFormation — all 5 gates passed ✅
- T4 GCP Terraform — all 5 gates passed ✅
- T5 DigitalOcean Terraform — all 5 gates passed ✅
- T6 Render — all 5 gates passed ✅
- T7 Railway — all 5 gates passed ✅
- T8 Vultr Terraform — all 5 gates passed ✅
- T9 Northflank — all 5 gates passed ✅
- T10 Linode — all 5 gates passed ✅

See `docs/PRD_DEPLOY.md` for full deployment PRD and falsification gate contract.
See `docs/DEPLOYMENT_STATUS.md` for the current live fleet matrix, provider quirks, and revalidation commands.

**Phase 3 — Operator UX & Control Plane** 🚧 in progress

- Governance policy APIs (retention, allowlists, audit) ✅
- Read/write retention enforcement ✅
- Operator hero-lane backend (summary, trace, preview, violations) ✅
- Webhook ops endpoints (dead-letter, replay, retry, stats) ✅
- Falsification suite: 78 integration tests including 4×4 contract/policy matrix ✅
- Raw Vector API (6 endpoints — upsert, search, delete, count, namespace lifecycle) ✅
- Session Messages API (list, clear, delete-by-index) ✅
- AnythingLLM vector DB provider (`integrations/anythingllm/`) ✅
- Python SDK full rebuild: sync + async clients, 45 methods (18 new async parity methods added in v0.3), full API coverage ✅
- LangChain `MnemoChatMessageHistory` drop-in adapter ✅
- LlamaIndex `MnemoChatStore` drop-in adapter (all 7 abstract methods) ✅
- SDK falsification test suite: 83/83 assertions pass ✅ (40 async + 43 sync)
- Operator-facing frontend surfaces 🚧 (`docs/OPERATOR_DASHBOARD_PRD.md`)
- p95 latency evidence capture 🚧

See `docs/OPERATOR_UX_PRD.md`, `docs/SDK_INTEGRATIONS_PRD.md`, and `docs/OPERATOR_DASHBOARD_PRD.md` for current scope.

**QA/QC Falsification** ✅ complete (3 phases)

- Phase 1: 59 new tests — graph engine, LLM providers, Qdrant store, async SDK, webhook persistence ✅
- Phase 2: 44 new tests — config parsing (24), session messages (7), raw vectors (1), auth integration (6), request-id/API (5) ✅
- Phase 3: 6 new tests — rate limiting, circuit breaker, RRF reranker, credential scan script, deploy artifact validation script ✅
- 109 new tests total, ~293 across the project (includes 18 new async parity tests added in v0.3)
- 3 bugs fixed (Qdrant TOCTOU race, `skip_compatibility_check`, `.gitignore` gaps)
- 3 new scripts (`credential_scan.sh`, `deploy_artifact_validation.sh`, `docker_build_test.sh`)

See `docs/QA_QC_FALSIFICATION_PRD.md` for the full 25-domain falsification plan.

**v0.6.0 — Enterprise Access Control** ✅ released

- Scoped API keys with RBAC (read/write/admin roles, optional user/agent/classification scoping) ✅
- Data classification labels (public/internal/confidential/restricted) ✅
- Policy-scoped memory views ✅
- Memory guardrails engine ✅
- Agent identity Phase B (experience weighting, COW branching, fork) ✅
- Multi-agent shared memory regions with ACLs ✅
- gRPC API (3 services, 8 RPCs) with red-team hardening ✅

**v0.7.0 — DevEx, Kubernetes & Enterprise Hardening** ✅ released

- OpenAPI 3.1 spec + Swagger UI (`/swagger-ui/`) ✅
- Production Helm chart with Redis/Qdrant subcharts ✅
- OpenTelemetry OTLP trace export ✅
- BYOK AES-256-GCM envelope encryption ✅
- Red-team audit (30 findings: 15 fixed — all CRITICAL + HIGH + most MEDIUM; 12 deferred as acceptable risk — see Honesty Notes) ✅
- Version sync across all workspace crates, SDKs, and Helm chart ✅

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions and guidelines.

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

*Named after Mnemosyne (Μνημοσύνη), the Greek Titaness of memory and mother of the Muses.*
