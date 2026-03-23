# Channel Visibility Spec

Status: draft
Owner: Retrieval / API
Priority: P1 candidate
Last updated: 2026-03-23

## 1) Goal

Preserve semantic, full-text, and graph retrieval results long enough to inspect them separately before fusion, without intentionally changing default fused retrieval output or ranking behavior.

This is the simplest prior-signal opportunity because it adds observability first. It does not require evidence-deference policies, LoRA gating, or replacing the default retrieval contract.

## 2) Problem

Today Mnemo fuses retrieval channels before returning context. That means:

- disagreement is hard to inspect
- evaluation cannot easily compare channels on the same query
- future evidence-first work lacks a clean observational baseline

## 3) Scope

### In scope

- retain per-channel retrieval results inside the retrieval pipeline before fusion
- add an internal diagnostics structure for semantic, full-text, and graph channel outputs
- optionally expose that structure behind a request flag or debug field
- keep default context behavior unchanged

### Out of scope

- changing ranking or fusion behavior
- introducing `guided` or `strict` deference modes
- LoRA gating or suppression
- redesigning SDKs around a new default response shape

## 4) Product Behavior

### Baseline behavior

`POST /api/v1/users/{user_id}/context` and `POST /api/v1/memory/{user}/context` continue returning the same fused context by default.

### New optional behavior

When diagnostics are explicitly requested, the response includes an optional channel-level retrieval diagnostics object.

Proposed request surface (non-final):

```json
{
  "messages": [ ... ],
  "structured": true,
  "include_retrieval_channels": true
}
```

Candidate response shape:

```json
{
  "context": "...",
  "sources": ["semantic", "full_text", "graph"],
  "retrieval_channels": {
    "semantic": {
      "backend": "qdrant",
      "results": [ ... ]
    },
    "full_text": {
      "backend": "redis_fulltext",
      "results": [ ... ]
    },
    "graph": {
      "backend": "graph_traversal",
      "results": [ ... ]
    }
  }
}
```

Notes:

- `retrieval_channels` should be optional and omitted by default
- current fused fields remain the primary contract
- open question: whether a later public-facing payload should use `literal` for readability; v1 should stay consistent with `full_text`

## 5) Technical Design

### Likely code touch points

- `crates/mnemo-retrieval/src/lib.rs`
  - preserve semantic hits before `merge_hits`
  - preserve full-text hits before fusion
  - preserve graph expansion results before graph-derived results are folded into final context assembly
- `crates/mnemo-core/src/models/context.rs`
  - add optional retrieval channel diagnostics types
- `crates/mnemo-server/src/routes.rs`
  - thread request flag through context endpoints
  - include diagnostics only when requested

### Minimal data model

Add optional types similar to:

```rust
pub struct RetrievalChannels {
    pub semantic: Option<ChannelResults>,
    pub full_text: Option<ChannelResults>,
    pub graph: Option<ChannelResults>,
}

pub struct ChannelResults {
    pub source: String,
    pub result_count: usize,
    pub results: Vec<ChannelHit>,
}
```

`ChannelHit` should be intentionally minimal in v1. It only needs enough information for debugging and evals, not a second full public retrieval API.

Recommended fields:

- `id`
- `kind` (`entity`, `fact`, `episode`)
- `score`
- `label` or short text preview
- optional temporal markers if already cheaply available

### V1 design constraint

Do not expose every internal reranker detail in v1. The goal is channel visibility, not full retrieval introspection.

## 6) Acceptance Criteria

1. Default context responses remain backward compatible.
2. A request flag enables channel diagnostics on the primary user context endpoint in v1.
3. Semantic, literal, and graph outputs can be inspected separately before fusion.
4. Tests verify that channel diagnostics are omitted by default and present when requested.
5. Docs clearly state that these diagnostics are observational and do not imply authority ordering.

## 7) Risks

1. **API creep**
   - Mitigation: keep diagnostics optional and compact.

2. **Misinterpretation of raw channel outputs**
   - Mitigation: label them as diagnostic, not authoritative.

3. **Latency increase**
   - Mitigation: reuse already computed intermediate results rather than duplicating retrieval work.

4. **Premature semantic commitment to channel names**
   - Mitigation: keep the v1 schema simple and document it as diagnostic.

## 8) Test Plan

- unit tests for retrieval diagnostics assembly
- integration tests for context endpoints with and without `include_retrieval_channels`
- regression tests confirming fused context output is unchanged when the flag is absent
- falsification tests defining and verifying the intended contract for absent vs empty channel entries and ensuring neither case crashes the endpoint

## 9) Why This First

This is the best first step because it creates learning without forcing a product bet.

- useful on its own for debugging and evals
- safe relative to stronger policy features
- creates the baseline needed for future disagreement scoring, evidence annotations, or policy modes
