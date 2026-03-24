# Channel Visibility V1 Execution Checklist

Status: draft
Owner: Retrieval / API
Priority: P1 candidate
Last updated: 2026-03-23

This is the file-level execution checklist for channel visibility v1.

## Phase 1: Model Surface

### `crates/mnemo-core/src/models/context.rs`

- add `include_retrieval_channels: bool` to `ContextRequest`
- add `retrieval_channels: Option<RetrievalChannels>` to `ContextBlock`
- add new types:
  - `RetrievalChannels`
  - `ChannelResults`
  - `GraphExpansionResults`
  - `ChannelHit`
  - `GraphExpansionHit`
  - `RetrievalBackend`
  - `GraphExpansionSource`
  - `ChannelHitKind`
- add serde defaults and `skip_serializing_if` annotations
- update `ContextBlock::empty()` to set `retrieval_channels: None`
- update existing tests for `ContextRequest` defaults and `ContextBlock` serialization
- add new tests for:
  - `include_retrieval_channels` defaulting to `false`
  - `retrieval_channels` omitted when `None`
  - present empty-channel serialization behavior

Exit check:

- `cargo test -p mnemo-core`

## Phase 2: Retrieval Snapshots

### `crates/mnemo-retrieval/src/lib.rs`

- identify the semantic hit vectors before each `merge_hits` call
- identify the full-text hit vectors before each `merge_hits` call
- capture graph-derived results around the graph traversal block
- add a small internal adapter from current hit tuples to:
  - `ChannelHit`
  - `GraphExpansionHit`
- ensure semantic/full-text snapshots are taken before fusion and reranking
- ensure `graph_expansion` is captured after fused entity seeding and before final context assembly
- keep `label` optional and hydrate it only when cheap
- document in code comments that scores are channel-local diagnostics

Open implementation question to resolve in code:

- whether label hydration should use existing summaries or a minimal extra fetch path

Exit checks:

- `cargo test -p mnemo-retrieval`
- targeted assertions for pre-fusion semantic/full-text capture

## Phase 3: Primary REST Route Wiring

### `crates/mnemo-server/src/routes.rs`

- thread `include_retrieval_channels` through `POST /api/v1/users/{user_id}/context`
- keep diagnostics omitted unless the flag is `true`
- update all fresh `ContextRequest` constructors to compile with the new field, including:
  - primary user context route
  - memory wrapper route
  - agent wrapper route
- do not add request-surface support to wrapper routes in v1
- preserve backward-compatible default response behavior

Important note:

- shared response structs may now carry the optional `retrieval_channels` field even where request-surface support is absent

Exit checks:

- `cargo test -p mnemo-server memory_api`

## Phase 4: Internal gRPC Adapter Compatibility

### `crates/mnemo-server/src/grpc.rs`

- update every `ContextRequest` or equivalent core request literal to set `include_retrieval_channels: false`
- do not change protobuf request or response schemas in v1
- keep gRPC behavior functionally unchanged

Exit checks:

- `cargo test -p mnemo-server grpc_api`

## Phase 5: OpenAPI

### `crates/mnemo-server/src/openapi.rs`

- register the new schema types if they are surfaced through `ContextBlock`
- confirm request docs show `include_retrieval_channels`
- confirm response docs show `retrieval_channels` as optional
- ensure descriptions do not imply that `graph_expansion` is an independent retrieval lane

Exit checks:

- OpenAPI build/test path passes

## Phase 6: Integration Tests

### `crates/mnemo-server/tests/memory_api.rs`

- add test: flag absent -> no `retrieval_channels`
- add test: flag `false` -> no `retrieval_channels`
- add test: flag `true` -> `retrieval_channels` present on `/api/v1/users/{user_id}/context`
- add test: executed zero-hit channel serializes with `result_count: 0` and `results: []`
- add test: `graph_expansion` key exists when graph-derived output is present
- add test: default fused context body is unchanged when diagnostics are off

### `crates/mnemo-retrieval/src/lib.rs` tests

- add focused tests for:
  - semantic snapshot timing
  - full-text snapshot timing
  - graph expansion derivation semantics
  - omitted-vs-empty behavior

## Phase 7: Docs Sync

### Keep aligned

- `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_SPEC.md`
- `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_IMPLEMENTATION.md`
- `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_TYPES.md`
- `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_V1_PLAN.md`

### Post-ship docs update

- add a concise note to `docs/API.md` only after the feature ships

## Final Verification

Run at minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mnemo-core
cargo test -p mnemo-retrieval
cargo test -p mnemo-server memory_api
cargo test -p mnemo-server grpc_api
```

## Done Checklist

- request flag exists and defaults to `false`
- primary REST route supports diagnostics
- semantic and full-text hits are captured pre-fusion
- graph-derived output is exposed as `graph_expansion`
- `label` is in v1 and omitted only when unavailable or too expensive
- shared structs remain backward compatible by default
- gRPC internal code compiles without protobuf changes
- tests cover absent, enabled, and empty-channel cases
- fmt, clippy, and targeted tests pass
