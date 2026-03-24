# Channel Visibility Minimal Types

Status: draft
Purpose: implementation planning
Last updated: 2026-03-23

This document proposes the smallest useful Rust types for channel visibility v1.

Scope note: this is a REST and JSON-first v1 shape for planning. gRPC would require a separate protobuf change and is not assumed here.

## Design Goals

- keep the payload diagnostic and compact
- avoid introducing a second full retrieval API
- preserve backward compatibility for REST responses by making the field optional
- distinguish pre-fusion channels from graph-derived expansion honestly

## Semantics Before Types

- `semantic` and `full_text` are direct per-channel hits captured before fusion
- `graph_expansion` is derived traversal output captured after fused entity seeding
- scores are channel-local diagnostics and must not be compared across channels as if they shared one scale
- in v1, omitted channel fields mean `not requested`, `not executed`, or `not applicable`
- in v1, executed zero-hit channels should serialize as present with `result_count: 0` and `results: []`

## Proposed Request Field

Add to `ContextRequest`:

```rust
pub include_retrieval_channels: bool,
```

Recommended serde behavior:

```rust
#[serde(default)]
pub include_retrieval_channels: bool,
```

## Proposed Response Field

Add to `ContextBlock`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub retrieval_channels: Option<RetrievalChannels>,
```

## Proposed Types

```rust
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetrievalChannels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<ChannelResults>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_text: Option<ChannelResults>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_expansion: Option<GraphExpansionResults>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelResults {
    pub backend: RetrievalBackend,
    pub result_count: u32,
    pub results: Vec<ChannelHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphExpansionResults {
    pub derived_from: GraphExpansionSource,
    pub result_count: u32,
    pub results: Vec<GraphExpansionHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelHit {
    pub id: Uuid,
    pub kind: ChannelHitKind,
    pub score: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphExpansionHit {
    pub id: Uuid,
    pub score: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalBackend {
    Qdrant,
    Redisearch,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GraphExpansionSource {
    FusedEntitySeeds,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelHitKind {
    Entity,
    Fact,
    Episode,
}
```

## Notes on Field Choices

### `backend`

Use `backend` instead of `source` so the field reads as implementation detail, not authority signal.

Examples:

- `qdrant`
- `redisearch`

`graph_expansion` does not get a `backend` field in v1 because it is not a peer backend response. It is derived traversal output.

### `result_count`

Keep `result_count` explicit even though `results.len()` exists. In v1 it should be defined as `results.len()` after any diagnostic truncation.

### `id`

Use `Uuid` for v1. Mnemo already exposes UUIDs broadly in context and summary models, so this is more precise and better aligned with current code paths.

### `kind`

Keep the enum coarse:

- `entity`
- `fact`
- `episode`

That is enough for debugging and evals.

`graph_expansion` hits do not need a `kind` field in v1 because current graph expansion is effectively fact and edge oriented.

### `score`

Keep `score` optional.

- semantic and full-text channels can expose a per-channel score
- graph expansion can expose a derived heuristic score if available
- callers must not compare these scores across channels as if they were on one shared scale

### `label`

Keep `label` in v1.

It should remain an optional short preview, not a full payload. If hydrating a preview proves expensive in a specific path, it may be omitted for that hit, but the field itself belongs in the v1 contract because it materially improves debugging value.

### `graph_expansion`

This should remain a separate type because it needs one extra semantic field:

```rust
pub derived_from: GraphExpansionSource,
```

Recommended v1 value:

```rust
GraphExpansionSource::FusedEntitySeeds
```

This keeps the contract honest and avoids an overly loose free-string field.

## Intentionally Excluded from V1

Do not include these in the first implementation:

- reranker contribution details
- temporal weighting internals
- raw score plus reranked score pairs
- authority or trust labels
- explanation text
- hydrated full record payloads
- gRPC-specific message shapes

## Example JSON

```json
{
  "retrieval_channels": {
    "semantic": {
      "backend": "qdrant",
      "result_count": 2,
      "results": [
        {
          "id": "8d6d0b8f-7d74-4b6c-8d1c-1e6f3d5a8f02",
          "kind": "entity",
          "score": 0.91,
          "label": "Kendra"
        }
      ]
    },
    "full_text": {
      "backend": "redisearch",
      "result_count": 1,
      "results": [
        {
          "id": "03a36d3c-9d8e-4c73-a6eb-f54d279dc64f",
          "kind": "episode",
          "score": 1.0,
          "label": "Kendra switched to Nike shoes"
        }
      ]
    },
    "graph_expansion": {
      "derived_from": "fused_entity_seeds",
      "result_count": 2,
      "results": [
        {
          "id": "6df0f9db-6c53-4bd1-85d5-7dd88e784dc5",
          "score": 0.78,
          "label": "Kendra -> prefers -> Nike"
        }
      ]
    }
  }
}
```

## Recommendation

Use these shapes as the frozen REST-first starting point for v1 unless implementation friction reveals a clearly simpler option. The main principle is more important than any one field: make semantic and full-text diagnostics direct, keep `label` in v1 for debugging value, and make graph-derived diagnostics transparently derived.
