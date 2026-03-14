# Spec 03: Temporal Reasoning

> Target: v0.9.0 (selective investment)
> Priority: Ship the high-value pieces. Defer the research project.

---

## Problem

Mnemo already tracks when facts became true (`valid_at`) and when they were
superseded (`invalid_at`). It supports `as_of` queries, `time_intent` parameters,
and `changes_since` diffs. But this is temporal *storage*, not temporal *reasoning*.

The gap: Mnemo stores the timeline but doesn't think about it. It doesn't
automatically detect that a new statement contradicts an old one for the same
topic. It doesn't know that some facts are time-scoped (quarterly targets) and
others aren't (company name). It doesn't rank more-referenced memories higher
than forgotten ones.

## What Exists Today

| Capability | Status | Location |
|---|---|---|
| `valid_at` / `invalid_at` on Edge (fact) | Implemented | `edge.rs` |
| Fact supersession (new fact invalidates old) | Implemented | `mnemo-ingest` |
| `as_of` parameter on context retrieval | Implemented | `routes.rs` |
| `time_intent` parameter (current, historical, last_week, etc.) | Implemented | `routes.rs` |
| `changes_since` endpoint (fact diff between timestamps) | Implemented | `routes.rs` |
| `time_travel/trace` endpoint (snapshot comparison) | Implemented | `routes.rs` |
| `time_travel/summary` endpoint (lightweight delta counts) | Implemented | `routes.rs` |
| `temporal_weight` parameter on retrieval | Implemented | `routes.rs` |
| Temporal eval harness (accuracy + stale fact rate) | Implemented | `eval/temporal_eval.py` |
| Quality gate: temporal accuracy >= 95%, stale rate <= 5% | Implemented | `quality-gates.yml` |
| `confidence` field on Edge | Implemented | `edge.rs` |
| `confidence_decay` field on Edge | Implemented | `edge.rs` |

### What's Missing

1. **No automatic belief-change detection.** The ingest pipeline creates new facts
   but doesn't proactively check if the new fact contradicts an existing one for the
   same subject-predicate pair. Supersession happens via LLM extraction, not via
   structural comparison.
2. **No fact-type-aware resolution.** All facts are treated the same temporally.
   There's no distinction between facts that should be superseded by newer data
   (preferences, status) and facts that are always true (birthdate, founded_year).
3. **No decay/reinforcement scoring.** The `confidence_decay` field exists but isn't
   used in retrieval ranking. Referenced memories don't gain salience.
4. **No temporal scoring in Qdrant.** Temporal weight is applied in post-processing,
   not in the vector search itself.

## Deliverables

### D1: Structural Belief-Change Detection

**When the ingest pipeline extracts a new fact (subject, predicate, object), check
if a current (non-invalidated) fact exists with the same subject and predicate but
a different object. If so, mark it as a detected belief change.**

This is not automatic supersession (that would be dangerous without human review
for important facts). It's detection and flagging.

**Implementation:**
- After entity/edge extraction in `mnemo-ingest`, query Redis for existing valid
  edges with the same `source_entity_id` + `label` (predicate)
- If found and `target` differs: create a `BeliefChange` event
- Store belief changes in Redis: `{prefix}belief_changes:{user_id}` sorted set
- New endpoint: `GET /api/v1/memory/{user}/belief_changes` — list detected changes
  with old_value, new_value, detected_at, auto_superseded (bool)
- For high-confidence changes (cosine similarity between old and new object < 0.3),
  auto-supersede. For ambiguous changes, flag for review.

**Confidence threshold for auto-supersession:**
- Same subject + same predicate + clearly different object → auto-supersede
- Same subject + same predicate + similar object (rephrasing) → merge/ignore
- Same subject + related predicate + different object → flag only, don't supersede

**Quality gate:** Add to `quality-gates.yml` — belief change detection precision
>= 80% on the temporal eval pack (no false positives on stable facts).

### D2: Fact Type Annotations

**Add `temporal_scope: Option<FactTemporalScope>` to the Edge model.**

```rust
pub enum FactTemporalScope {
    /// Fact is expected to change over time (preferences, status, targets)
    Mutable,
    /// Fact is generally stable (birthdate, company founding, nationality)
    Stable,
    /// Fact is time-bounded (quarterly target, event date, deadline)
    TimeBounded { expires_at: Option<DateTime<Utc>> },
}
```

**Usage in retrieval:**
- When `time_intent` is `current`: strongly prefer `Mutable` facts with recent
  `valid_at` over old ones. `Stable` facts are always included regardless of age.
- When `time_intent` is `historical`: include superseded `Mutable` facts from the
  requested time window. Still include `Stable` facts.
- `TimeBounded` facts past their `expires_at` are excluded from `current` retrieval
  but available in `historical`.

**LLM extraction prompt update:** Instruct the extraction prompt to classify facts
as mutable/stable/time-bounded during extraction. Fall back to `Mutable` if unclear.

**Implementation:**
- Add field to `Edge` in `edge.rs`
- Update LLM extraction prompts in `mnemo-ingest`
- Modify retrieval scoring in `mnemo-retrieval` to use temporal scope
- Migration: existing facts default to `None` (treated as `Mutable`)

### D3: Decay and Reinforcement Scoring

**Memories that are repeatedly retrieved gain salience. Memories that are never
accessed fade in retrieval ranking.**

**Implementation:**
- Add `access_count: u32` and `last_accessed_at: Option<DateTime<Utc>>` to Edge
- Increment `access_count` and update `last_accessed_at` whenever an edge is
  included in a context retrieval response (async, non-blocking — don't add
  latency to retrieval)
- Compute `recency_score` during retrieval:
  ```
  recency = 1.0 / (1.0 + days_since_last_access * decay_rate)
  reinforcement = log2(1 + access_count) * reinforcement_weight
  temporal_score = base_score * recency + reinforcement
  ```
- `decay_rate` and `reinforcement_weight` configurable via
  `MNEMO_TEMPORAL_DECAY_RATE` (default 0.05) and
  `MNEMO_TEMPORAL_REINFORCEMENT_WEIGHT` (default 0.1)
- Facts that have never been accessed have `recency = 1.0` (no penalty)
  and `reinforcement = 0.0` (no boost)

**Non-goal:** This is not memory deletion. Low-scoring facts are deprioritized in
retrieval ranking but never removed. They remain available for historical queries.

### D4: Temporal Eval Pack Expansion

**Expand the eval packs to cover the new capabilities:**

- Add belief-change detection cases (correct detection, false positive resistance)
- Add fact-type cases (stable facts retrieved regardless of age, mutable facts
  superseded correctly, time-bounded facts expired correctly)
- Add decay/reinforcement cases (frequently-accessed facts ranked higher than
  equivalent low-access facts)

**Target:** Temporal eval pack grows from 13 cases (3 temporal + 10 scientific) to
25+ cases covering the three new capabilities.

---

## What to Defer

### Causal Chain Extraction — NOT in v0.9.0

Connecting episodes into causal narratives ("user complained → we switched →
user reported improvement") is a research problem. It requires:
- Multi-hop temporal reasoning across episodes
- Distinguishing correlation from causation
- A way to evaluate causal chain quality (no established benchmarks)

This is worth exploring in `mnemo-gnn` but is not ready for productization.
Track in a separate research spec if/when GNN validation (per STEP_CHANGES.md)
produces positive results.

### Temporal Scoring in Qdrant — deferred

Moving temporal scoring from post-processing into the vector search itself would
improve recall (currently, temporally irrelevant results can displace relevant ones
in the top-K). But this requires custom Qdrant scoring or pre-computed temporal
embeddings, both of which are complex. Defer until temporal eval shows this is a
retrieval quality bottleneck.

---

## Non-Goals

- **Automatic deletion of old facts.** Mnemo never deletes memories. Decay affects
  ranking, not existence.
- **User-facing time-travel UI.** The dashboard RCA page already handles this.
- **Retroactive fact type classification.** Existing facts stay unclassified until
  re-ingested. Not worth a backfill migration.

## Risks

1. **Auto-supersession false positives.** If the belief-change detector incorrectly
   supersedes a valid fact, the user loses memory. Mitigate: high confidence
   threshold (cosine < 0.3) and the ability to un-supersede (restore invalidated
   facts via API).
2. **LLM extraction quality for fact types.** If the LLM doesn't reliably classify
   mutable vs. stable, the fact type system adds complexity without value. Mitigate:
   evaluate classification accuracy on the expanded eval pack before shipping.
3. **Reinforcement gaming.** If agents repeatedly retrieve their own facts to boost
   scores, the reinforcement system can be gamed. Mitigate: only count distinct
   retrieval sessions (not repeated calls in the same session).

## Success Criteria

- [ ] Belief-change detection fires on new facts that contradict existing ones
- [ ] Auto-supersession for high-confidence changes (precision >= 80%)
- [ ] Fact type annotations (mutable/stable/time-bounded) applied during extraction
- [ ] Retrieval respects fact types (stable facts always included, mutable facts
      prefer recent)
- [ ] Decay/reinforcement scoring affects retrieval ranking
- [ ] Temporal eval pack expanded to 25+ cases
- [ ] All existing temporal quality gates still pass (accuracy >= 95%, stale <= 5%)
