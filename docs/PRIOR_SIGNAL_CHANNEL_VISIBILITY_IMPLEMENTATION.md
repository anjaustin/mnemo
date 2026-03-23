# Channel Visibility Implementation Checklist

Status: draft
Owner: Retrieval / API
Priority: P1 candidate
Last updated: 2026-03-23

This checklist maps `docs/PRIOR_SIGNAL_CHANNEL_VISIBILITY_SPEC.md` onto the current Mnemo codebase.

## 1. Request Model Changes

- Add `include_retrieval_channels: bool` to `ContextRequest` in `crates/mnemo-core/src/models/context.rs`.
- Ensure the new field defaults to `false` and is omitted cleanly when not set.
- Update the request defaults test in `crates/mnemo-core/src/models/context.rs`.
- Mirror the field into wrapper request types:
  - `MemoryContextRequest` in `crates/mnemo-server/src/routes.rs`
  - `AgentContextRequest` in `crates/mnemo-server/src/routes.rs`
- If gRPC is included in v1, add the field to the relevant messages in `proto/mnemo/v1/memory.proto` and regenerate via `crates/mnemo-proto/build.rs`.

## 2. Response Model Changes

- Add optional channel diagnostics types near `ContextBlock` in `crates/mnemo-core/src/models/context.rs`.
- Add `retrieval_channels: Option<RetrievalChannels>` to `ContextBlock`.
- Keep `retrieval_channels` omitted by default with `skip_serializing_if = "Option::is_none"`.
- Update `ContextBlock::empty()` in `crates/mnemo-core/src/models/context.rs` to initialize the field as `None`.
- If gRPC is included in v1, add corresponding optional response fields in `proto/mnemo/v1/memory.proto`.

## 3. Retrieval Engine Changes

- In `crates/mnemo-retrieval/src/lib.rs`, preserve semantic hits before `merge_hits` for entities, facts, and episodes.
- Preserve full-text hits before fusion in the same retrieval path.
- Preserve graph-derived results around the graph traversal block before they are folded into final context assembly.
- Build channel diagnostics before later steps mutate the results, including:
  - temporal scoring
  - structured context assembly
  - explanation assembly
  - final context string assembly
- Keep v1 channel hits minimal. Current traits return tuples such as IDs and scores, so preview hydration should be avoided unless clearly needed.

## 4. Route and Controller Changes

- Thread the request flag through `POST /api/v1/users/{user_id}/context` in `crates/mnemo-server/src/routes.rs`.
- Thread the same flag through the wrapper routes that build fresh `ContextRequest` values:
  - `POST /api/v1/memory/{user}/context`
  - `POST /api/v1/agents/{agent_id}/context`
- If gRPC is in scope, map request and response fields in `crates/mnemo-server/src/grpc.rs`.
- Update OpenAPI schema registration in `crates/mnemo-server/src/openapi.rs` if the new response types are exposed there.

## 5. Testing

- Add model tests in `crates/mnemo-core/src/models/context.rs` for:
  - request defaulting
  - omitted serialization when the flag is absent
  - omitted serialization when `retrieval_channels` is `None`
- Add retrieval tests in `crates/mnemo-retrieval/src/lib.rs` to verify:
  - semantic/full-text/graph snapshots are captured pre-fusion
  - diagnostics are absent when not requested
- Add REST integration tests in `crates/mnemo-server/tests/memory_api.rs` for:
  - `/api/v1/users/{id}/context`
  - `/api/v1/memory/{user}/context`
  - `/api/v1/agents/{agent}/context`
  - with and without `include_retrieval_channels`
- If gRPC is included, update gRPC tests in `crates/mnemo-server/tests/grpc_api.rs`.

## 6. Backward Compatibility Gates

- Default fused context output must remain unchanged.
- The new diagnostics field must be optional and omitted by default.
- Existing REST clients must continue working without any request changes.
- Existing MCP integrations that call `/api/v1/memory/{user}/context` must remain unaffected when the flag is absent.
- If gRPC is updated, confirm the proto change is additive and wire-compatible.

## 7. Open Questions

- Should v1 be REST-only, or should gRPC ship at the same time?
- Should the diagnostics key use `full_text` in v1 for consistency with internals?
- Should channel diagnostics represent raw retrieval output only, or post-policy output after memory-route contract and view mutations?
- What is the exact absent-vs-empty contract for channels?

## 8. Known Traps in the Current Codebase

- The graph channel is not a fully independent ranked retrieval path today; it is derived from already-fused entity seeds.
- Memory routes mutate `ContextBlock` after retrieval through contracts, views, guardrails, and goal logic, so diagnostics timing must be explicit.
- Current storage and full-text traits return compact hit tuples, not rich preview payloads.
- GNN reranking happens after semantic/full-text entity fusion, so channel snapshots must be captured before that if v1 means truly pre-fusion diagnostics.

## 9. Suggested V1 Cut

The smallest credible v1 is:

- REST only
- `POST /api/v1/users/{user_id}/context` first
- optional `include_retrieval_channels`
- minimal channel payloads with IDs, kind, and score
- no SDK changes required yet
- no gRPC or MCP surface changes in the first cut

## 10. Exit Criteria

- A context request flag enables pre-fusion channel diagnostics on the primary user context endpoint.
- Default behavior remains backward compatible.
- Tests cover absent/default behavior and diagnostics-enabled behavior.
- Documentation clearly states that the field is diagnostic and does not imply authority ordering.
