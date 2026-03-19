# Architecture

Deep dive into Mnemo's system architecture and internals.

---

## System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                           Client Layer                               │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐ │
│  │  Python  │  │TypeScript│  │   MCP    │  │   Direct REST/gRPC   │ │
│  │   SDK    │  │   SDK    │  │  Server  │  │                      │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────────┬───────────┘ │
└───────┼─────────────┼─────────────┼───────────────────┼─────────────┘
        │             │             │                   │
        └─────────────┴─────────────┴───────────────────┘
                              │
                        HTTP/gRPC
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Mnemo Server                                  │
│                                                                      │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                         API Layer                              │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐   │  │
│  │  │   Axum   │  │   Tonic  │  │   MCP    │  │  Dashboard   │   │  │
│  │  │   REST   │  │   gRPC   │  │   SSE    │  │     SPA      │   │  │
│  │  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────┬───────┘   │  │
│  └───────┼─────────────┼─────────────┼───────────────┼───────────┘  │
│          │             │             │               │              │
│          └─────────────┴─────────────┴───────────────┘              │
│                              │                                       │
│  ┌───────────────────────────┴───────────────────────────────────┐  │
│  │                      Core Domain Layer                         │  │
│  │                                                                │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐   │  │
│  │  │  Models  │  │  Traits  │  │  Errors  │  │  Encryption  │   │  │
│  │  │  (14+)   │  │  (8+)    │  │          │  │   (at-rest)  │   │  │
│  │  └──────────┘  └──────────┘  └──────────┘  └──────────────┘   │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│          ┌───────────────────┼───────────────────┐                  │
│          │                   │                   │                  │
│          ▼                   ▼                   ▼                  │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐            │
│  │   Ingest     │   │  Retrieval   │   │    Graph     │            │
│  │   Worker     │   │   Engine     │   │  Traversal   │            │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘            │
│         │                  │                  │                     │
│  ┌──────┴──────────────────┴──────────────────┴──────┐             │
│  │                   Storage Layer                    │             │
│  │  ┌──────────────┐  ┌───────────────────────────┐  │             │
│  │  │  StateStore  │  │       VectorStore         │  │             │
│  │  │   (Redis)    │  │        (Qdrant)           │  │             │
│  │  └──────────────┘  └───────────────────────────┘  │             │
│  └───────────────────────────────────────────────────┘             │
│                                                                      │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                     Provider Layer                             │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐   │  │
│  │  │   LLM    │  │Embedding │  │  Vision  │  │Transcription │   │  │
│  │  │ Provider │  │ Provider │  │ Provider │  │   Provider   │   │  │
│  │  └──────────┘  └──────────┘  └──────────┘  └──────────────┘   │  │
│  └───────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Workspace Crates

Mnemo is organized as a Cargo workspace with specialized crates:

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `mnemo-core` | Domain models, traits, error types | Minimal (serde, uuid, chrono) |
| `mnemo-server` | HTTP server, routes, AppState | Axum, Tonic, all other crates |
| `mnemo-storage` | Redis + Qdrant implementations | Redis, Qdrant client |
| `mnemo-ingest` | Background processing pipeline | LLM, storage |
| `mnemo-retrieval` | Hybrid retrieval engine | Graph, storage, GNN |
| `mnemo-graph` | Graph traversal algorithms | Core |
| `mnemo-llm` | LLM provider abstraction | HTTP clients |
| `mnemo-gnn` | Graph neural network reranking | Candle |
| `mnemo-mcp` | Model Context Protocol server | Core |
| `mnemo-lora` | TinyLoRA personalization | Core |

### Dependency Graph

```
mnemo-server
    ├── mnemo-retrieval
    │   ├── mnemo-graph
    │   ├── mnemo-gnn
    │   └── mnemo-storage
    ├── mnemo-ingest
    │   ├── mnemo-llm
    │   └── mnemo-storage
    ├── mnemo-mcp
    └── mnemo-core (shared by all)
```

---

## Core Domain Models

### Primary Types (mnemo-core/models)

| Module | Types | Purpose |
|--------|-------|---------|
| `user` | User, UserUpdate | Multi-tenant users |
| `session` | Session | Conversation threads |
| `episode` | Episode, EpisodeType | Memory units |
| `entity` | Entity, EntityType | Graph nodes |
| `edge` | Edge, TemporalScope | Graph edges (facts) |
| `context` | ContextRequest, ContextBlock | Retrieval I/O |
| `attachment` | Attachment, Modality | Multi-modal files |
| `agent` | AgentIdentity, Branch, Fork | Agent personality |
| `region` | MemoryRegion, RegionACL | Access control |
| `view` | MemoryView, ViewConstraints | Filtered access |
| `guardrail` | Guardrail, Condition | Content filtering |
| `classification` | Classification | Data labeling |
| `narrative` | Narrative, Chapter | Session summaries |
| `goal` | GoalProfile, RetrievalGoal | Goal-conditioned |

### Traits (mnemo-core/traits)

| Trait | Purpose |
|-------|---------|
| `StateStore` | Persistent state (users, sessions, entities, edges) |
| `VectorStore` | Vector embeddings (semantic search) |
| `BlobStorage` | File storage (attachments) |
| `LlmProvider` | LLM completions |
| `EmbeddingProvider` | Text embeddings |
| `VisionProvider` | Image analysis |
| `TranscriptionProvider` | Audio-to-text |
| `DocumentParser` | Document extraction |

---

## Ingestion Pipeline

When an episode is created:

```
POST /episodes
     │
     ▼
┌─────────────┐
│ Validation  │  Schema, auth, rate limits
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Persist    │  Store episode in Redis
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Enqueue    │  Add to processing queue
└──────┬──────┘
       │
       ▼
┌─────────────────────────────────────────────────────────┐
│               Background Worker (DAG)                    │
│                                                          │
│  ┌────────┐  ┌─────────┐  ┌───────┐  ┌──────────────┐  │
│  │ Digest │─▶│ Extract │─▶│ Embed │─▶│ Graph Update │  │
│  └────────┘  └─────────┘  └───────┘  └──────────────┘  │
│       │           │           │              │          │
│       ▼           ▼           ▼              ▼          │
│   Summary    Entities    Vectors         Edges          │
│   for HEAD   + Edges    in Qdrant      in Redis         │
└─────────────────────────────────────────────────────────┘
```

### Pipeline Steps

1. **Digest** - Generate a summary for session context
2. **Extract** - LLM extracts entities and relationships
3. **Embed** - Generate vector embeddings
4. **Graph Update** - Insert/update entities and edges

### Retry & Dead Letter

Failed steps are retried with exponential backoff. After max retries, episodes go to dead-letter queue for manual inspection.

---

## Retrieval Pipeline

When context is requested:

```
POST /context
     │
     ▼
┌─────────────────┐
│ Intent Parsing  │  Temporal intent, entity extraction
└────────┬────────┘
         │
    ┌────┴────┬─────────────┐
    ▼         ▼             ▼
┌────────┐ ┌───────┐ ┌────────────┐
│Semantic│ │ FTS   │ │   Graph    │
│ Search │ │Search │ │ Traversal  │
└───┬────┘ └───┬───┘ └─────┬──────┘
    │          │           │
    └──────────┴───────────┘
               │
               ▼
        ┌──────────────┐
        │   Reranking   │  RRF / MMR / GNN / Hyperbolic
        └──────┬───────┘
               │
               ▼
        ┌──────────────┐
        │  Temporal    │  Filter by validity, apply decay
        │  Filtering   │
        └──────┬───────┘
               │
               ▼
        ┌──────────────┐
        │Token Budget  │  Fit within max_tokens
        └──────┬───────┘
               │
               ▼
          ContextBlock
```

### Search Types

- **Semantic**: Qdrant vector similarity (cosine)
- **Full-Text**: RediSearch keyword matching
- **Graph**: BFS from query entities

### Reranking Strategies

| Strategy | Description |
|----------|-------------|
| RRF | Reciprocal Rank Fusion - merges ranked lists |
| MMR | Maximal Marginal Relevance - diversity |
| GNN | Graph Attention Network on local subgraph |
| Hyperbolic | Poincare ball for hierarchical data |

---

## Storage Architecture

### Redis (State Store)

All structured data lives in Redis:

```
mnemo:{user_id}:user          → User JSON
mnemo:{user_id}:sessions      → Session IDs (sorted set)
mnemo:{session_id}:session    → Session JSON
mnemo:{session_id}:episodes   → Episode IDs (sorted set)
mnemo:{episode_id}:episode    → Episode JSON
mnemo:{user_id}:entities      → Entity IDs (set)
mnemo:{entity_id}:entity      → Entity JSON
mnemo:{user_id}:edges         → Edge IDs (sorted set by valid_at)
mnemo:{edge_id}:edge          → Edge JSON
```

### RediSearch (Full-Text)

Index on episode content and entity names:

```
FT.CREATE mnemo:idx:episodes
  ON HASH PREFIX 1 mnemo:
  SCHEMA content TEXT WEIGHT 1.0
         user_id TAG
         created_at NUMERIC SORTABLE
```

### Qdrant (Vectors)

Collections:
- `mnemo_episodes` - Episode embeddings
- `mnemo_entities` - Entity embeddings
- `mnemo_edges` - Edge embeddings (fact statements)

Payload includes user_id for filtering.

---

## LLM Provider Architecture

Pluggable providers for different LLM operations:

```rust
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<String>;
}

pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
}

pub trait VisionProvider: Send + Sync {
    async fn analyze(&self, image: &[u8], config: &VisionConfig) -> Result<VisionAnalysis>;
}

pub trait TranscriptionProvider: Send + Sync {
    async fn transcribe(&self, audio: &[u8], config: &TranscriptionConfig) -> Result<Transcript>;
}
```

### Implemented Providers

| Operation | Providers |
|-----------|-----------|
| Completion | Anthropic, OpenAI, Ollama, Liquid |
| Embedding | FastEmbed (local), OpenAI, Voyage, Ollama |
| Vision | OpenAI GPT-4V, Anthropic Claude Vision |
| Transcription | OpenAI Whisper |

---

## Authentication & Authorization

### API Key Flow

```
Request
   │
   ▼
┌─────────────────┐
│ Extract Key     │  Bearer token or x-api-key header
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Hash & Lookup   │  SHA-256 hash → Redis lookup
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Validate        │  Not revoked, not expired
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Check Scope     │  Role, user scope, agent scope
└────────┬────────┘
         │
         ▼
    CallerContext
```

### CallerContext

Every request carries:
```rust
struct CallerContext {
    api_key_id: Uuid,
    role: Role,                    // read, write, admin
    user_scope: Option<Uuid>,      // Restrict to user
    agent_scope: Option<String>,   // Restrict to agent
    max_classification: Classification,
}
```

---

## Horizontal Scaling

### Stateless Server

Mnemo servers are stateless - scale by adding replicas:

```yaml
apiVersion: apps/v1
kind: Deployment
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: mnemo
        image: ghcr.io/anjaustin/mnemo/mnemo-server:latest
```

### Redis Clustering

For high availability:
- Redis Sentinel (failover)
- Redis Cluster (sharding)

### Qdrant Clustering

Qdrant supports distributed mode:
```yaml
cluster:
  enabled: true
  replication_factor: 2
```

---

## Performance Characteristics

### Latency

| Operation | P50 | P99 |
|-----------|-----|-----|
| Store episode | 15ms | 50ms |
| Get context | 45ms | 120ms |
| List entities | 5ms | 15ms |

### Throughput

| Configuration | Episodes/sec | Context/sec |
|---------------|--------------|-------------|
| Single server | 500 | 200 |
| 3 replicas | 1,500 | 600 |

### Memory

| Component | RAM |
|-----------|-----|
| Mnemo server | 200-500 MB |
| FastEmbed (local) | 1.5 GB |
| Redis (per 1M episodes) | ~500 MB |
| Qdrant (per 1M vectors) | ~1 GB |

---

## Next Steps

- **[Configuration](configuration.md)** - All settings
- **[Benchmarks](../../BENCHMARKS.md)** - Performance measurements
- **[Deployment](../../DEPLOY.md)** - Production setup
