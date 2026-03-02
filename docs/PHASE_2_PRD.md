# Mnemo Phase 2 — Temporal Memory Productization PRD

**Version:** 0.2.0-PRD  
**Date:** 2026-03-02  
**Predecessor:** Phase 1.5 (production hardening + memory API + temporal v1)

---

## 1. Goal

Make Zep look and feel like an antique by making Mnemo the default choice for production memory systems by turning current strengths into measurable, repeatable product outcomes:

- better temporal correctness,
- lower stale-memory errors,
- faster integration,
- stronger operational trust.

---

## 2. Scope Summary

Phase 2 focuses on four initiatives:

1. **Thread HEAD hardening** (current thread state as a first-class product feature)
2. **Temporal retrieval v2** (more accurate time-aware ranking and diagnostics)
3. **Metadata index layer** (filtering and planner-guided retrieval)
4. **Evaluation + competitive publication** (evidence-backed narrative)

P0 cross-cutting candidate:

5. **Agent Identity Substrate** (homeostatic identity + homeoadaptive experience layer)

---

## 3. Success Criteria (Release Gates)

Phase 2 is complete when all three gates pass.

### Gate A — Product Behavior

- `mode=head|hybrid|historical` is stable and documented.
- `head` diagnostics are returned consistently for session-scoped retrieval.
- Temporal controls (`time_intent`, `as_of`, `temporal_weight`) produce expected ranking shifts in falsification tests.

### Gate B — Reliability

- Memory falsification suite remains green in CI.
- New temporal/head falsification tests are added and required in CI.
- No regression in API usability under local/no-key conditions.

### Gate C — Proof

- Benchmark/eval workflow produces reproducible tables with run IDs.
- At least one published cross-system benchmark table exists with methodology notes.
- README/docs include measurable claims tied to evidence links.

---

## 4. Milestones

| Milestone | Status | Owner | Acceptance |
|---|---|---|---|
| M1: Thread HEAD completion | complete | Core API | HEAD metadata + response diagnostics + SDK ergonomics + tests |
| M2: Temporal retrieval v2 | complete | Retrieval | improved temporal ranking + diagnostics + eval delta vs baseline |
| M3: Metadata index layer v1 | complete | Storage/Retrieval | metadata prefilter planner behind flag + latency/candidate metrics |
| M4: Competitive publication v1 | in_progress | DevRel/Eng | published scorecard with run IDs and methodology caveats |
| M5: Agent Identity Substrate P0 | in_progress | Core/Retrieval | isolated identity+experience planes, guarded updates, identity-aware context, falsification coverage |

---

## 5. Workstreams

### 5.1 Thread HEAD Completion

**Objective:** Make "current thread state" easy and predictable.

**In scope:**
- Ensure session head metadata is always updated on write paths.
- Keep `head` diagnostics consistent across memory context responses.
- Add SDK convenience methods and examples for `mode=head`.
- Add edge-case tests (no sessions, multi-session ties, explicit session override).

**Out of scope:**
- background head summarization quality optimization.

**Exit criteria:**
- deterministic head selection documented and tested.

### 5.2 Temporal Retrieval v2

**Objective:** Improve temporal correctness without sacrificing latency.

**In scope:**
- Refine intent inference and weighting defaults.
- Add transparent scoring diagnostics in responses/logs.
- Strengthen historical alignment behavior for `as_of`.
- Expand temporal falsification scenarios (preference changes, contradictions, point-in-time).

**Out of scope:**
- learned temporal embeddings as default path.

**Exit criteria:**
- measurable improvement over baseline in eval harness (accuracy up, stale rate down).

### 5.3 Metadata Index Layer v1

**Objective:** Improve precision/latency with metadata-first narrowing.

**In scope:**
- Implement metadata index schema for sessions/episodes.
- Add prefilter planner in retrieval pipeline behind feature flag.
- Add metrics: candidate reduction ratio, planner latency contribution.

**Out of scope:**
- full analytics backend replacement.

**Exit criteria:**
- filtered queries show significant candidate reduction and stable latency.

### 5.4 Evaluation + Competitive Publication

**Objective:** Replace narrative claims with reproducible evidence.

**In scope:**
- Keep `eval/temporal_eval.py` and workflow stable.
- Publish run evidence with run IDs and environment notes.
- Maintain parity caveats for non-1:1 API semantics.

**Out of scope:**
- marketing collateral beyond technical docs.

**Exit criteria:**
- docs contain current scorecard with valid recent evidence.

---

## 6. Dependencies and Order

Recommended execution order:

1. M1 Thread HEAD completion
2. M2 Temporal retrieval v2
3. M3 Metadata index layer v1
4. M5 Agent Identity Substrate P0
5. M4 Competitive publication v1 (parallel once evidence exists)

Rationale:
- M1 + M2 directly improve user-visible correctness.
- M3 improves precision and scale control before identity layering.
- M5 addresses long-run identity stability and self/user attribution quality.
- M4 proves value continuously and guides prioritization.

---

## 7. Risks

1. **Overweighting recency** causes relevant historical facts to be buried.
   - Mitigation: explicit `mode`/`time_intent` controls and conservative defaults.

2. **Benchmark parity disputes** due to API semantic differences.
   - Mitigation: publish clear methodology notes and run artifacts.

3. **Complexity creep** across retrieval paths.
   - Mitigation: feature flags, falsification-first rollout, and diagnostics.

---

## 8. Source Documents

- `docs/THREAD_HEAD.md`
- `docs/TEMPORAL_VECTORIZATION.md`
- `docs/METADATA_INDEX_LAYER.md`
- `docs/AGENT_IDENTITY_SUBSTRATE.md`
- `docs/EVALUATION.md`
- `docs/COMPETITIVE.md`
