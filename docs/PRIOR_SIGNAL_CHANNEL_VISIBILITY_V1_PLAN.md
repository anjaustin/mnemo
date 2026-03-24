# Channel Visibility V1 Freeze and Task Plan

Status: draft
Owner: Retrieval / API
Priority: P1 candidate
Last updated: 2026-03-23

This document freezes the v1 scope for channel visibility and breaks implementation into concrete tasks.

## 1. Frozen V1 Decisions

These decisions are fixed for v1 unless a blocking implementation issue is discovered.

### Scope

- REST and JSON first
- no gRPC or protobuf API changes in v1
- no MCP-specific surface changes in v1
- no SDK changes in v1
- primary request-surface support only in v1: `POST /api/v1/users/{user_id}/context`

### Request Shape

Add to `ContextRequest`:

```rust
#[serde(default)]
pub include_retrieval_channels: bool,
```

Default is `false`.

### Response Shape

Add to `ContextBlock`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub retrieval_channels: Option<RetrievalChannels>,
```

### Channel Semantics

- `semantic`: direct semantic hits captured before fusion
- `full_text`: direct full-text hits captured before fusion
- `graph_expansion`: derived graph traversal results captured after fused entity seeding

### Omission Semantics

- `retrieval_channels: None` means diagnostics were not requested or retrieval did not execute because the request short-circuited early
- channel field omitted means that channel was not executed or not applicable in that request path
- present channel with `result_count: 0` and `results: []` means the channel executed and returned no hits

### Frozen Field Shapes

```rust
use uuid::Uuid;

pub struct RetrievalChannels {
    pub semantic: Option<ChannelResults>,
    pub full_text: Option<ChannelResults>,
    pub graph_expansion: Option<GraphExpansionResults>,
}

pub struct ChannelResults {
    pub backend: RetrievalBackend,
    pub result_count: u32,
    pub results: Vec<ChannelHit>,
}

pub struct GraphExpansionResults {
    pub derived_from: GraphExpansionSource,
    pub result_count: u32,
    pub results: Vec<GraphExpansionHit>,
}

pub struct ChannelHit {
    pub id: Uuid,
    pub kind: ChannelHitKind,
    pub score: Option<f32>,
    pub label: Option<String>,
}

pub struct GraphExpansionHit {
    pub id: Uuid,
    pub score: Option<f32>,
    pub label: Option<String>,
}
```

### Frozen Enumerations

```rust
pub enum RetrievalBackend {
    Qdrant,
    Redisearch,
}

pub enum GraphExpansionSource {
    FusedEntitySeeds,
}

pub enum ChannelHitKind {
    Entity,
    Fact,
    Episode,
}
```

### Score Semantics

- scores are channel-local diagnostics only
- scores must not be compared across channels as if they share one scale
- `graph_expansion.score` is a derived heuristic, not a backend similarity score

### Label Decision

`label` is explicitly included in v1.

Rationale:

- diagnostics without a short human-readable preview are much less useful
- `label` materially improves debugging, operator inspection, and eval review
- it remains optional so implementation can omit it when hydration is too expensive

## 2. Out of Scope for V1

- gRPC and protobuf API parity
- MCP parity
- request-surface parity for `/api/v1/memory/{user}/context`
- request-surface parity for `/api/v1/agents/{agent_id}/context`
- disagreement scoring
- evidence-deference modes
- LoRA diagnostics or gating
- reranker internals
- authority ordering

## 3. Implementation Tasks

### Task A: Core Request and Response Models

- update `crates/mnemo-core/src/models/context.rs`
- add `include_retrieval_channels` to `ContextRequest`
- add `retrieval_channels` and the new diagnostic types to `ContextBlock`
- add serde annotations for omission behavior
- update `ContextBlock::empty()`
- update all `ContextRequest` constructors and `ContextBlock` consumers needed to preserve compile and default behavior, including internal gRPC adapter code paths even though no gRPC API change ships in v1

Acceptance:

- models compile
- omission behavior is explicit and tested

### Task B: Retrieval Engine Snapshots

- update `crates/mnemo-retrieval/src/lib.rs`
- preserve semantic hits before fusion
- preserve full-text hits before fusion
- preserve graph-derived results as `graph_expansion`
- define a small adapter layer from existing hit tuples to frozen v1 diagnostic structs

Acceptance:

- semantic and full-text snapshots are pre-fusion
- graph expansion is captured post-seed and labeled honestly

### Task C: Primary REST Endpoint Wiring

- update `crates/mnemo-server/src/routes.rs`
- thread `include_retrieval_channels` through `POST /api/v1/users/{user_id}/context`
- keep wrapper and agent routes compiling with the new `ContextRequest` field even though they do not get request-surface support in v1
- keep diagnostics omitted by default
- include diagnostics only when requested

Acceptance:

- endpoint remains backward compatible by default
- requested diagnostics can only be enabled through the primary user context route in v1, even though shared response structs may inherit the optional field elsewhere

### Task D: OpenAPI Surface

- update `crates/mnemo-server/src/openapi.rs`
- register new response schema types if needed
- ensure `include_retrieval_channels` appears in request docs

Acceptance:

- OpenAPI generation succeeds
- docs do not overstate authority or independence of `graph_expansion`

### Task E: Tests

- add model tests in `crates/mnemo-core/src/models/context.rs`
- add retrieval tests in `crates/mnemo-retrieval/src/lib.rs`
- add REST integration tests in `crates/mnemo-server/tests/memory_api.rs`

Required cases:

- flag absent -> no diagnostics
- flag `false` -> no diagnostics
- flag `true` -> diagnostics present
- executed zero-hit channel -> present with empty results
- graph expansion serializes as `graph_expansion`
- default context output remains unchanged when diagnostics are off

### Task F: Documentation Sync

- keep `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_SPEC.md` aligned with implementation
- keep `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_IMPLEMENTATION.md` aligned with any task discoveries
- add a short note to API docs only after the feature ships

## 4. Risks to Watch During Implementation

- preview hydration for `label` may add more cost than expected
- graph expansion may be sparse enough that empty-channel semantics show up frequently
- current retrieval internals may make it awkward to preserve hits without extra cloning
- future wrapper-route parity may force a second pass on omission semantics

## 5. Done Means

V1 is done when:

- the primary user context endpoint supports `include_retrieval_channels`
- default behavior is unchanged
- semantic and full-text hits are visible pre-fusion
- graph-derived results are visible as `graph_expansion`
- `label` is included when cheaply available and omitted safely otherwise
- tests cover default, enabled, and empty-channel cases
