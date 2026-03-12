# Mnemo 0.5.5 Roadmap — Autonomic Memory

**Status**: SHIPPED (tagged v0.5.5)
**Baseline**: v0.5.0 (13-feature mesa: GNN, EWC++, MCP, witness chain, semantic routing, hyperbolic HNSW, COW branching, DAG workflows, delta consensus, domain expansion, verified identity updates)
**Theme**: Make Mnemo self-maintaining. Memory should heal itself, age gracefully, and narrate its own evolution — without human intervention.

---

## Design Principle

v0.5.0 gave agents a homeostatic identity framework — they can accumulate experience, branch, fork, and protect themselves against contamination. But the *memory layer* (user episodes, facts, entities, edges) is still passive. It stores what it's told and retrieves what it's asked for.

v0.5.5 makes memory **autonomic**: it detects its own inconsistencies, decays stale knowledge, triggers revalidation of important-but-aging facts, summarizes its own evolution, and heals conflicts without waiting for a human to notice.

The features below build directly on v0.5.0 infrastructure:
- **EWC++** already computes `fisher_importance` and `effective_experience_weight_ewc()` — Confidence Decay extends this math to the fact/entity layer.
- **Witness chain** already provides hash-chained audit — Self-Healing Memory uses the same event infrastructure for clarification tracking.
- **Conflict Radar** (shipped v0.3.x) already detects contradictions — Self-Healing Memory automates the resolution step.
- **Semantic routing** already classifies queries — Goal-Conditioned Memory extends this to objective-aware retrieval.

---

## Tier 1 — Core Autonomic Capabilities

### 1. Confidence Decay + Revalidation

**Origin**: `FACE_MELTER_FEATURES.md` item 7

**What**: Facts and entity edges decay in confidence over time unless reinforced by new evidence. When a high-importance fact drops below a configurable threshold, Mnemo emits a `revalidation_needed` webhook event and flags the fact for proactive verification.

**Why**: Memory systems accumulate stale facts. "User prefers Nike" may have been true 6 months ago but isn't checked. Decay forces the system to treat old, unreinforced knowledge as progressively uncertain — and actively flag what needs re-confirmation.

**Architecture**:
- Decay function on `Edge` confidence: `effective_confidence = confidence * decay_factor(age, half_life)`. Mirrors the EWC++ decay curve already used for experience events.
- `fisher_importance` on edges: high-importance edges (structurally central, frequently retrieved) decay slower — same consolidation principle as EWC++.
- Revalidation threshold: configurable per-user or global (`MNEMO_REVALIDATION_THRESHOLD=0.3`). When `effective_confidence` drops below threshold, emit `revalidation_needed` webhook with the fact, its source episodes, and suggested clarification question.
- `GET /api/v1/memory/:user/stale` endpoint: returns facts below threshold, ranked by importance (highest-importance stale facts first — these are the ones that matter most to re-confirm).
- Revalidation ack: `POST /api/v1/memory/:user/revalidate` accepts a fact ID + new confidence, resetting the decay clock.

**Scope**:
- `mnemo-core`: `EffectiveEdgeConfidence` type, `edge_fisher_importance()` function, `RevalidationEvent` type
- `mnemo-retrieval`: decay-aware confidence scoring in retrieval pipeline (stale facts get lower retrieval weight)
- `mnemo-server`: `/stale` endpoint, `/revalidate` endpoint, webhook integration
- Config: `MNEMO_CONFIDENCE_DECAY_HALF_LIFE_DAYS=90`, `MNEMO_REVALIDATION_THRESHOLD=0.3`

**Files touched**: `mnemo-core/models/edge.rs`, `mnemo-retrieval/src/lib.rs`, `mnemo-server/routes.rs`

---

### 2. Self-Healing Memory

**Origin**: `FACE_MELTER_FEATURES.md` item 10

**What**: Auto-detect low-confidence conflicts and contradictions, generate a single targeted clarification question, and reconcile graph state when the answer arrives. The full loop: detect -> question -> answer -> heal.

**Why**: Conflict Radar (shipped) already detects contradictions and surfaces them. But it stops there — someone has to manually inspect and resolve. Self-Healing closes the loop: Mnemo generates the clarification question itself, and when the answer is ingested, it automatically reconciles the conflicting facts.

**Architecture**:
- Conflict scanner: periodic background task (configurable interval, default 1 hour) that runs Conflict Radar across all users with active sessions in the last N days.
- Question generator: for each conflict above a severity threshold, call `LlmProvider::generate_clarification()` to produce a single, natural-language question that would resolve the ambiguity. Store as a `ClarificationRequest` with `conflict_id`, `question`, `status` (pending/answered/expired), `expires_at`.
- Answer ingestion: when a new episode is ingested that matches a pending clarification (semantic similarity to the question above a threshold), automatically mark the clarification as answered and apply the resolution — supersede the losing fact, boost the winning fact's confidence.
- `GET /api/v1/memory/:user/clarifications` endpoint: list pending clarification questions (for agent UIs to proactively ask the user).
- `POST /api/v1/memory/:user/clarifications/:id/resolve` endpoint: manually resolve a clarification with an answer.
- Webhook: `clarification_generated` event when a new question is created, `clarification_resolved` when answered.

**Scope**:
- `mnemo-core`: `ClarificationRequest`, `ClarificationStatus` types
- `mnemo-storage`: Redis storage for clarification requests
- `mnemo-ingest`: answer-matching logic in the ingestion pipeline
- `mnemo-server`: `/clarifications` endpoints, background scanner task, webhook events
- `mnemo-llm`: `generate_clarification()` method on `LlmProvider`

**Files touched**: `mnemo-core/models/`, `mnemo-storage/redis_store.rs`, `mnemo-ingest/src/lib.rs`, `mnemo-server/routes.rs`, `mnemo-server/main.rs`, `mnemo-llm/src/lib.rs`

---

### 3. Cross-Session Narrative Summaries

**Origin**: `FACE_MELTER_FEATURES.md` item 11

**What**: Generate evolving "story of the user" narratives that update after each session. Each narrative has chapter-style diffs — what changed, what was reinforced, what decayed. The narrative is retrievable as a standalone context block.

**Why**: Long-running agents accumulate hundreds of sessions. No human reads 500 session summaries. A narrative summary distills the user's evolution into a readable story that the agent can use as high-level context — "this user started as a fitness beginner, transitioned to marathon training, recently shifted focus to recovery."

**Architecture**:
- Narrative generation: after each session closes (or on-demand), call `LlmProvider::generate_narrative_update()` with the previous narrative + new session summary + key fact changes. Returns an updated narrative with change annotations.
- `UserNarrative` type: `user_id`, `version`, `narrative_text`, `chapters` (array of `NarrativeChapter` with `period`, `summary`, `key_changes`), `last_updated`, `session_count`.
- Storage: Redis, keyed by user ID. Versioned — each update creates a new version (mirrors agent identity versioning pattern).
- `GET /api/v1/memory/:user/narrative` endpoint: returns the current narrative.
- `POST /api/v1/memory/:user/narrative/refresh` endpoint: force-regenerate the narrative from scratch (expensive — scans all sessions).
- Integration with context assembly: `get_memory_context()` can optionally include the narrative as a preamble block (controlled by `include_narrative: true` in the request body).
- Config: `MNEMO_NARRATIVE_ENABLED=true`, `MNEMO_NARRATIVE_AUTO_UPDATE=true` (update after each session close).

**Scope**:
- `mnemo-core`: `UserNarrative`, `NarrativeChapter` types
- `mnemo-storage`: Redis storage for narrative versions
- `mnemo-server`: `/narrative` endpoints, auto-update hook in session close
- `mnemo-llm`: `generate_narrative_update()` method
- `mnemo-retrieval`: optional narrative inclusion in context assembly

**Files touched**: `mnemo-core/models/`, `mnemo-storage/redis_store.rs`, `mnemo-server/routes.rs`, `mnemo-llm/src/lib.rs`, `mnemo-retrieval/src/lib.rs`

---

## Tier 2 — Autonomic Extensions

### 4. Goal-Conditioned Memory

**Origin**: `FACE_MELTER_FEATURES.md` item 6

**What**: Condition retrieval strategy by active objective, not only semantic similarity. The query `"what does the user like?"` returns different context when `goal=resolve_ticket` (focus on recent complaints, account status) vs `goal=plan_trip` (focus on travel preferences, past destinations).

**Why**: Semantic routing (v0.5.0) classifies queries by structure — "latest" routes to Head mode, "timeline" to Historical. But it's blind to the agent's current task. Goal conditioning adds a second routing dimension: what the agent is trying to accomplish right now.

**Architecture**:
- `RetrievalGoal` enum or free-form string in `ContextRequest`. If provided, the semantic router uses goal-specific keyword/category boosting in addition to structural classification.
- Goal profiles: configurable mappings from goal names to retrieval biases — which entity categories to boost, which temporal windows to prefer, which edge types to prioritize. Stored as JSON in Redis, manageable via API.
- `POST /api/v1/goals` CRUD endpoints for managing goal profiles.
- Integration: `get_memory_context()` accepts optional `goal` parameter. Semantic router consults goal profile to adjust retrieval weights before Qdrant query.

**Scope**:
- `mnemo-core`: `RetrievalGoal`, `GoalProfile` types
- `mnemo-retrieval`: goal-aware retrieval weight adjustment in router
- `mnemo-server`: `/goals` CRUD endpoints, `goal` parameter in context request
- `mnemo-storage`: Redis storage for goal profiles

**Files touched**: `mnemo-core/models/`, `mnemo-retrieval/src/router.rs`, `mnemo-server/routes.rs`, `mnemo-storage/redis_store.rs`

---

### 5. Counterfactual Memory

**Origin**: `FACE_MELTER_FEATURES.md` item 2

**What**: Simulate retrieval context under hypothetical assumptions. "If the user still preferred Adidas, what context would we send the agent?" Returns a full context block as if certain facts were different — without modifying actual memory state.

**Why**: Planning agents need to reason about alternatives. Policy teams need to test "what if we remove this fact?" without touching production data. Counterfactual memory turns Mnemo into a simulation engine for agent context.

**Architecture**:
- `POST /api/v1/memory/:user/counterfactual` endpoint. Request body includes the normal context request plus a `hypotheticals` array — each hypothetical is a fact override: `{ "entity": "user", "attribute": "brand_preference", "value": "Adidas", "confidence": 0.9 }`.
- The endpoint runs the normal retrieval pipeline but injects the hypothetical facts into the candidate set (replacing any conflicting real facts) before fusion and assembly.
- Response includes a `counterfactual_diff` showing which real facts were overridden and how the context changed.
- Read-only operation — no state is modified.
- Builds on COW branching concept: internally, the counterfactual creates a transient, in-memory branch of the fact graph, runs retrieval against it, then discards it.

**Scope**:
- `mnemo-core`: `CounterfactualRequest`, `HypotheticalFact`, `CounterfactualDiff` types
- `mnemo-retrieval`: hypothetical injection in the retrieval pipeline (after candidate fetch, before fusion)
- `mnemo-server`: `/counterfactual` endpoint

**Files touched**: `mnemo-core/models/`, `mnemo-retrieval/src/lib.rs`, `mnemo-server/routes.rs`

---

## Execution Order

1. Confidence Decay + Revalidation
2. Self-Healing Memory
3. Cross-Session Narrative Summaries
4. Goal-Conditioned Memory
5. Counterfactual Memory

Each item follows the cycle: **code -> test -> falsify -> document -> commit -> push**.

Items 1-3 are the core autonomic capabilities — they make memory self-maintaining. Items 4-5 extend the autonomic theme into retrieval intelligence.

---

## Future (v0.6.0 — Enterprise Access Control)

The following features are deferred to v0.6.0:
- Policy-Scoped Memory Views (support-safe, sales-safe, internal views)
- Memory Guardrails Engine (declarative storage/retrieval constraints)
- Agent Identity Phase B: Governance UI, richer conflict handling
- Agent Identity Phase C: Multi-agent templates, cross-agent identity contracts
- Multi-Agent Shared Memory with ACLs

## Future (v0.6.5 — Qdrant-Native Scale)

The following optimizations are deferred to v0.6.5:
- Named vectors / multi-vector points
- Hybrid sparse + dense search
- Grouped search (session-balanced context)
- Quantization + HNSW tuning
- Aliases + snapshots (zero-downtime migrations)
- Sharding and replication controls
