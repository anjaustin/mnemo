# Mnemo 0.5.0 Roadmap — Self-Learning Memory Control Plane

**Status**: In Progress
**Baseline**: v0.4.0 (Agent Identity Substrate complete)
**Inspiration**: Architecture patterns from [RuVector](https://github.com/anjaustin/ruvector) — self-learning vector database with GNN re-ranking, SONA optimization, and graph intelligence.

---

## Tier 1 — High-value, architecturally aligned

### 1. GNN-Enhanced Retrieval Re-Ranking

**Origin**: `ruvector-gnn` (Graph Neural Network re-ranking layer)

**What**: Add a lightweight GNN re-ranking layer that runs after Qdrant returns candidate results. The GNN operates on Mnemo's existing knowledge graph (entities + edges) and learns from implicit feedback — which retrieved facts the agent actually used in its response.

**Why**: Mnemo already has a knowledge graph with entities, edges, and traversal. A GNN layer would leverage this structure to improve retrieval quality over time without manual tuning. This is the single highest-leverage retrieval improvement possible.

**Architecture**:
- GNN operates on the subgraph of HNSW neighbors (10-50 nodes), not the full dataset
- Multi-head attention weights which neighbors matter most for a given query
- Feedback signal: compare retrieved facts against what the agent actually cited/used
- Three architecture options: GCN (simple), GAT (attention-weighted), GraphSAGE (handles new nodes)
- Target: <1ms additional latency on top of existing retrieval

**Scope**:
- New crate: `crates/mnemo-gnn/` with `GnnReranker` trait
- Integration point: `crates/mnemo-retrieval/` after Qdrant candidate fetch
- Feedback endpoint: `POST /api/v1/memory/feedback` to record which results were useful
- Config: `MNEMO_GNN_ENABLED=true`, `MNEMO_GNN_ARCHITECTURE=gat`

**Files touched**: `mnemo-retrieval`, `mnemo-server/routes.rs`, new `mnemo-gnn` crate

---

### 2. SONA/EWC++ Experience Weight Consolidation

**Origin**: `sona` (Self-Optimizing Neural Architecture with EWC++)

**What**: Replace the simple exponential decay in `effective_experience_weight()` with Elastic Weight Consolidation (EWC++) — a principled method that protects "important" experience weights from being erased by new ones, solving catastrophic forgetting for agent personality.

**Why**: The current decay formula `weight * confidence * 2^(-age / half_life)` treats all experiences equally — old experiences simply fade. EWC++ identifies which experiences are load-bearing for the agent's current identity and protects them, while still allowing new learning.

**Architecture**:
- Compute Fisher Information Matrix (diagonal approximation) over experience events
- High-Fisher experiences resist decay even when old (they're structurally important)
- Low-Fisher experiences decay normally (they were incidental)
- Fisher matrix updated incrementally on each new experience event
- Stored per-agent alongside the identity profile

**Scope**:
- Modify `effective_experience_weight()` in `routes.rs`
- Add `fisher_importance: f32` field to `ExperienceEvent` model
- Add Fisher matrix computation on `add_experience_event`
- New endpoint: `GET /api/v1/agents/:id/experience/importance` — returns events ranked by Fisher importance

**Files touched**: `mnemo-core/models/agent.rs`, `mnemo-server/routes.rs`, `mnemo-storage/redis_store.rs`

---

### 3. Temporal Tensor Compression for Old Episodes

**Origin**: `ruvector-temporal-tensor` (adaptive tiered compression, 2-32x memory reduction)

**What**: Compress old episode embeddings in Qdrant using adaptive tiered quantization. Recent episodes stay at full f32 precision; older episodes get progressively quantized (f32 -> f16 -> int8 -> binary). Massive storage savings at scale.

**Why**: Mnemo stores every episode embedding at full 384-dimensional f32 precision (1,536 bytes each) forever. At scale, this becomes a storage cost problem. A 1M-episode deployment uses ~1.5 GB just for embeddings. Tiered compression could reduce that to ~200 MB for episodes older than 30 days.

**Architecture**:
- Tier 0 (0-7 days): Full f32 precision (1,536 bytes/vector)
- Tier 1 (7-30 days): f16 quantization (768 bytes/vector, ~50% savings)
- Tier 2 (30-90 days): int8 scalar quantization (384 bytes/vector, ~75% savings)
- Tier 3 (90+ days): Binary quantization (48 bytes/vector, ~97% savings, re-rank from text)
- Background job runs on configurable schedule
- Qdrant native quantization support used where available

**Scope**:
- New background task in `mnemo-server` or `mnemo-retrieval`
- Config: `MNEMO_EMBEDDING_COMPRESSION_ENABLED=true`, tier thresholds
- Endpoint: `GET /api/v1/ops/compression` — reports compression stats per tier
- Integration with Qdrant's scalar/binary quantization APIs

**Files touched**: `mnemo-retrieval`, `mnemo-server/routes.rs`, `mnemo-server/state.rs`

---

### 4. Coherence Scoring Endpoint + Digest Integration

**Origin**: `ruvector-coherence` (signal quality measurement)

**What**: Add a coherence scoring system that measures how internally consistent a user's knowledge graph is. Surface the score in memory digests and use it to trigger sleep-time consolidation.

**Why**: We already have conflict radar (detects contradictions) and memory digest (prose summary). Coherence scoring unifies these into a single health metric — a number from 0.0 to 1.0 that says "how well does this user's memory hold together?"

**Architecture**:
- **Entity coherence**: Do connected entities have semantically related embeddings?
- **Fact coherence**: Are active facts mutually consistent? (leverages conflict radar)
- **Temporal coherence**: Do recent episodes align with the established knowledge graph?
- **Graph structural coherence**: Is the graph well-connected or fragmented?
- Composite score = weighted average of sub-scores
- When score drops below threshold, auto-trigger digest regeneration

**Scope**:
- New endpoint: `GET /api/v1/users/:user/coherence`
- Response: `{ score, entity_coherence, fact_coherence, temporal_coherence, structural_coherence, recommendations }`
- Add `coherence_score` field to `MemoryDigest` model
- Config: `MNEMO_COHERENCE_AUTO_CONSOLIDATION_THRESHOLD=0.7`

**Files touched**: `mnemo-retrieval`, `mnemo-core/models/digest.rs`, `mnemo-server/routes.rs`

---

### 5. MCP Server (Model Context Protocol)

**Origin**: `mcp-gate` (Model Context Protocol gateway)

**What**: Expose Mnemo as an MCP tool server so any MCP-compatible agent (Claude, GPT, Cursor, etc.) can `remember`, `recall`, `manage_identity`, and `query_graph` without SDK integration.

**Why**: MCP is becoming the standard agent-to-tool protocol. This would dramatically lower the integration barrier — any MCP-compatible client can use Mnemo by pointing at a URL, with zero SDK code.

**Architecture**:
- MCP transport: stdio (for local) and SSE (for remote)
- Tools exposed:
  - `mnemo_remember` — store a memory
  - `mnemo_recall` — retrieve context for a query
  - `mnemo_graph_query` — query the knowledge graph
  - `mnemo_agent_identity` — get/update agent identity
  - `mnemo_agent_context` — full agent context assembly
  - `mnemo_digest` — get memory digest
  - `mnemo_health` — health check
- Resources exposed:
  - `mnemo://users/{user}/memory` — user memory as a resource
  - `mnemo://agents/{agent}/identity` — agent identity as a resource

**Scope**:
- New crate: `crates/mnemo-mcp/` with MCP server implementation
- Binary: `mnemo-mcp-server` (stdio transport for Claude Code / Cursor integration)
- HTTP SSE transport for remote MCP connections
- Config: `MNEMO_MCP_ENABLED=true`, `MNEMO_MCP_TRANSPORT=stdio`

**Files touched**: New `mnemo-mcp` crate, `Cargo.toml` workspace

---

## Tier 2 — Valuable, moderate effort

### 6. Witness Chain Tamper-Proof Audit

**Origin**: `rvf-crypto` (cryptographic witness chains)

**What**: Hash-chain the agent identity audit log entries. Each audit event includes `prev_hash = SHA256(prev_event)`, creating a tamper-evident chain. Anyone can verify no entries were deleted or modified.

**Why**: The agent identity audit log is currently a simple append-only list in Redis. A witness chain adds cryptographic tamper-evidence — if any entry is deleted or modified, the chain breaks and verification fails. Strong compliance/trust differentiator.

**Scope**:
- Add `prev_hash: String` and `event_hash: String` fields to `AgentIdentityAuditEvent`
- Compute `event_hash = SHA256(action + from_version + to_version + prev_hash + timestamp)`
- New endpoint: `GET /api/v1/agents/:id/identity/audit/verify` — walks the chain, returns `{ valid: bool, chain_length, breaks: [] }`
- Zero-dependency: use `sha2` crate (already common in Rust ecosystem)

---

### 7. Semantic Routing for Retrieval Strategy

**Origin**: `ruvector-router-core`, `ruvector-tiny-dancer-core` (semantic routing via FastGRNN)

**What**: Route incoming `context()` calls to different retrieval strategies (hybrid, graph-focused, episode-only, head-mode) based on query semantics, instead of requiring the caller to choose `mode`.

**Why**: Most callers pass `mode=hybrid` because they don't know which strategy is best. A lightweight classifier on the query text can route to the optimal strategy automatically — "What did we discuss yesterday?" -> head mode, "What are Alice's core beliefs?" -> graph-focused, "Tell me about the project meeting" -> hybrid.

**Scope**:
- Query classifier using embedding similarity to prototype queries per mode
- Fallback: hybrid (current default)
- Config: `MNEMO_SEMANTIC_ROUTING_ENABLED=true`
- Diagnostic field in context response: `routing_decision: { selected_mode, confidence, alternatives }`

---

### 8. Hyperbolic HNSW for Entity Hierarchy

**Origin**: `ruvector-hyperbolic-hnsw` (Poincare ball space for hierarchical data)

**What**: Use Poincare ball embeddings for entity hierarchy in the knowledge graph. Entities that form natural trees/taxonomies get better nearest-neighbor results in hyperbolic space vs. Euclidean.

**Why**: Real-world knowledge graphs are inherently hierarchical (person -> works at -> company -> in -> industry). Euclidean space distorts tree structures; hyperbolic space preserves them with exponentially more room at the periphery.

**Scope**:
- Hyperbolic embedding projection for entity vectors
- Modified distance function for graph traversal (Poincare distance vs. cosine)
- Config: `MNEMO_HYPERBOLIC_GRAPH_ENABLED=true`
- Requires: entity embedding storage in Qdrant with custom distance metric

---

### 9. COW Branching for Agent Identity A/B Testing

**Origin**: `rvf-cow` (Git-like copy-on-write branching)

**What**: Git-like branching for agent identities — create a branch, run it for N conversations, compare metrics against main, then merge or discard.

**Why**: Currently, updating an agent identity is a one-way operation (with rollback as a safety net). Branching allows controlled experimentation: "try this personality change on 10% of traffic and measure the impact before committing."

**Scope**:
- `POST /api/v1/agents/:id/branches` — create a branch from current identity
- `GET /api/v1/agents/:id/branches/:branch/context` — context assembly using branched identity
- `POST /api/v1/agents/:id/branches/:branch/merge` — merge branch into main
- `DELETE /api/v1/agents/:id/branches/:branch` — discard branch
- Branch storage: separate Redis keys with `branch:` prefix

---

### 10. DAG Workflows for Memory Consolidation Pipeline

**Origin**: `ruvector-dag` (self-learning directed acyclic graph execution)

**What**: Express the memory consolidation pipeline (ingest -> extract -> embed -> graph-update -> digest) as a typed DAG with retry, dead-letter, and observability at each node.

**Why**: The current pipeline uses ad-hoc channels. A DAG formalization adds: per-step retry with backoff, dead-letter queues for failed extractions, step-level latency metrics, and the ability to add new processing steps (e.g., coherence check) without rewiring the pipeline.

**Scope**:
- DAG definition in `mnemo-ingest` or new `mnemo-pipeline` crate
- Steps: `Ingest -> Extract -> Embed -> GraphUpdate -> WebhookNotify -> DigestInvalidate`
- Per-step Prometheus counters and error tracking
- Config: `MNEMO_PIPELINE_RETRY_MAX=3`, `MNEMO_PIPELINE_DEAD_LETTER_ENABLED=true`

---

## Tier 3 — Interesting, longer-term

### 11. Delta Consensus (CRDT Multi-Node Sync)

**Origin**: `ruvector-delta-consensus` (CRDTs, causal ordering, vector clocks)

**What**: CRDT-based delta synchronization for user memory across multiple Mnemo nodes. Enables geo-distributed deployments where each region has a local Mnemo instance that eventually converges.

**Why**: Currently Mnemo is single-node. For enterprise deployments requiring regional presence (EU data residency, US-East/West latency), CRDT sync would allow each region to operate independently with eventual consistency.

**Scope**: Multi-crate effort. Requires conflict resolution strategies for facts, entities, and edges. Vector clocks for causal ordering. Significant architectural change.

---

### 12. Domain Expansion / Transfer Learning for Agents

**Origin**: `ruvector-domain-expansion` (cross-domain transfer learning)

**What**: Let a well-trained agent identity bootstrap a new agent — "create agent B from agent A's experience, but with different boundaries." Knowledge transfers across domains so new agents don't start from scratch.

**Why**: Organizations often have multiple agents that share common knowledge (company values, communication style) but differ in domain. Transfer learning would let a mature agent seed a new one, dramatically reducing the cold-start problem.

**Scope**:
- `POST /api/v1/agents/:id/fork` — create a new agent from an existing one
- Selective experience transfer (filter by category, confidence threshold)
- Identity core override (new agent gets its own mission/boundaries)

---

### 13. Verified/Proof-Carrying Identity Updates

**Origin**: `ruvector-verified` (formal proof-carrying writes)

**What**: Every promotion proposal or identity update carries a cryptographic proof that the candidate_core satisfies all contamination guard invariants. The proof is verified server-side before the write is accepted.

**Why**: The contamination guard currently runs at request time. Proof-carrying updates shift validation to the proposer — the server just verifies the proof, which is cheaper and provides a stronger guarantee (the proof is stored alongside the update for auditability).

**Scope**: Requires a proof system (e.g., Merkle proof of allowlist membership). Moderate cryptographic complexity. Lower priority until the contamination guard needs to handle more complex invariants.

---

## Execution Order

1. GNN-Enhanced Retrieval Re-Ranking
2. SONA/EWC++ Experience Weight Consolidation
3. Temporal Tensor Compression
4. Coherence Scoring
5. MCP Server
6. Witness Chain Audit
7. Semantic Routing
8. Hyperbolic HNSW
9. COW Branching
10. DAG Workflows
11. Delta Consensus
12. Domain Expansion
13. Verified Identity Updates

Each item follows the cycle: **code -> test -> falsify -> document -> commit -> push**.
