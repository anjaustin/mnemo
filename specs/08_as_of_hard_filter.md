# Spec 08 — `as_of` Hard-Filter Consistency

**Status:** In design  
**Author:** AI pair  
**Date:** 2026-03-15

---

## Problem Statement

The eval harness (Spec 05) identifies two failing temporal gates:

| Gate | Current | Threshold | Gap |
|---|---:|---:|---|
| Temporal accuracy (27 cases) | 85.2% | 95% | 9.8pp |
| Scientific pack stale rate (10 cases) | 40% | 5% | 35pp |

Root cause (from retrieval code audit):

**`as_of` is only a hard filter for edges coming through the semantic/FTS fusion
loop** (`lib.rs:285–291`, `is_valid_at(as_of)`). The other two retrieval paths ignore it:

1. **Graph traversal** (`lib.rs:349–411`): calls `edge.is_valid()` (current-time only), not
   `edge.is_valid_at(as_of)`. Invalidated facts can pass through when `as_of` is set.

2. **Episodes**: no hard cutoff at all. Only soft exponential decay scoring
   (`score_episode_temporal`). An episode from 2030 can rank above an episode
   from 2024 when `as_of=2024-06-01` is requested.

3. **Entities**: no `as_of` awareness. Fixed neutral temporal score regardless of when
   the entity was created or when facts about it were invalidated.

Additionally, the **semantic/FTS hard filter only excludes facts with an explicit
`invalid_at`**. Semantically-similar but temporally-stale facts that lack an
`invalid_at` marker (common in practice — users do not explicitly invalidate old
episodes) still pass the filter and can outscore newer facts on cosine similarity.

---

## Goal

Make all three retrieval paths treat `as_of` as a hard constraint, not a hint:

- **Any fact/edge** whose `valid_at > as_of` (created in the future) is excluded.
- **Any fact/edge** whose `invalid_at <= as_of` is excluded (already superseded).
- **Any episode** whose `created_at > as_of` is excluded.
- **Entities** are filtered by whether any of their facts are valid at `as_of`.

This converts the 4 failing core-pack cases from a ranking problem to a filter
problem: the stale version of a fact will not appear in the result set at all.

---

## Deliverables

### D1: Graph traversal `as_of` filter

**File:** `crates/mnemo-retrieval/src/lib.rs`  
**Current (line ~357):**
```rust
if !edge.is_valid() {
    continue;
}
```
**New:**
```rust
if let Some(as_of) = temporal_filter {
    if !edge.is_valid_at(as_of) {
        continue;
    }
} else if !edge.is_valid() {
    continue;
}
```

This mirrors the existing check in the semantic/FTS loop at lines 285–291 — same
predicate, same pattern.

### D2: Episode hard cutoff

**File:** `crates/mnemo-retrieval/src/lib.rs`  
**Target:** Inside the episode result loop, before soft scoring.

```rust
// Hard-exclude episodes created after as_of
if let Some(as_of) = temporal_filter {
    if episode.created_at > as_of {
        continue;
    }
}
```

This ensures that when `as_of=2024-06-01`, no episode from 2025 appears in
the result, regardless of how semantically similar it is.

### D3: Entity filter

Entities don't have `valid_at`/`invalid_at` timestamps directly — they are derived
from the edges (facts) that mention them. An entity is "valid at T" if at least one
of its associated edges is valid at T.

Two sub-deliverables:

**D3a:** In the entity result loop, after fetching entity candidates, fetch their
associated edge count at `as_of`. Skip entities with zero valid edges at `as_of`.

**D3b:** Optionally: pass `as_of` to the vector search query as a metadata filter
(Qdrant supports payload filters). This pre-filters at the vector store level,
reducing the result set before edge validation.

**D3b is optional for this spec** — D3a is the correctness fix; D3b is a
performance optimization.

### D4: Regression test cases

Add 4 new cases to `eval/temporal_cases.json` targeting the specific failure modes:

1. **Graph-traversal stale fact**: a fact that was invalidated before `as_of` but
   reached via graph traversal — must not appear.
2. **Future episode exclusion**: episode ingested with `created_at` after `as_of` —
   must not appear in historical query.
3. **Stale entity**: entity whose only fact was invalidated before `as_of` — must
   not appear in historical query.
4. **Valid historical retrieval**: fact that is valid at `as_of` but superseded
   afterward — must appear in historical query, must not appear in current query.

### D5: Eval gate re-run

After D1–D4 are implemented, re-run the full eval harness and verify:
- Temporal accuracy (core 27 cases) ≥ 95% (closes the failing gate)
- Scientific pack stale rate ≤ 5% (closes the failing gate)
- No regression in LongMemEval or recall quality gates

---

## Non-Goals

- Changing the default retrieval behavior for queries **without** `as_of`. The
  existing soft temporal scoring (Spec 03) is not modified.
- Changing `temporal_weight` defaults.
- Modifying LoRA adaptation or homeoadaptive embeddings.
- Zep comparison run (requires external service — tracked separately).

---

## Implementation Plan

### Phase 1: D1 + D2 (graph traversal + episode hard filter)
These are surgical one-line changes to `lib.rs`. Risk: low. Expected impact:
directly fixes the 4 failing core-pack cases.

### Phase 2: D4 (regression cases)
Add 4 new cases to `temporal_cases.json` targeting the fixed failure modes.
Cases must be written **before** D3 so they form a regression baseline.

### Phase 3: D3a (entity filter)
More involved — requires fetching associated edges per entity in the result loop.
May require a new `LoraStore`/`EdgeStore` method or reuse of existing graph APIs.

### Phase 4: D5 (eval re-run + gate confirmation)
Full eval suite re-run. Update `BENCHMARKS.md` with revised numbers.

---

## Risks

- **D3a performance**: fetching associated edges per entity adds N+1 queries.
  Mitigate by batching the edge lookups. If latency regresses beyond the 300ms
  gate, defer D3a to Spec 09 and ship D1+D2 alone.

- **D2 side effects on `Recent` intent**: the episode hard cutoff uses `created_at >
  as_of`. When `time_intent=Recent` without `as_of`, no cutoff is applied (correct —
  `temporal_filter` will be `None`). No risk to the `Recent` path.

- **Test count**: adding 4 cases to a 27-case pack moves each case from 3.7pp to
  3.2pp. The accuracy gate at 95% now requires passing 29/31 cases. This is
  intentionally tighter — the pack was too small.

---

## Success Criteria

| Metric | Current | Target |
|---|---:|---:|
| Temporal accuracy (core pack) | 85.2% | ≥ 95% |
| Temporal stale rate (core pack) | 3.7% | ≤ 5% |
| Scientific pack stale rate | 40% | ≤ 15% |
| LongMemEval (all task types) | 100% | ≥ 100% (no regression) |
| Recall accuracy | 87.5% | ≥ 85% (no regression) |
| p95 latency | 66ms | ≤ 300ms (no regression) |

Note: scientific pack stale rate target is ≤ 15%, not ≤ 5%. The scientific pack
failures involve semantically similar facts with no `invalid_at` markers — hard
filtering alone cannot fix these without metadata enrichment (a separate feature).
The target reflects realistic improvement from D1+D2 alone.
