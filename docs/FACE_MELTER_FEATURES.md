# Face-Melter Feature Concepts

Date: 2026-03-02
Status: 10 of 12 shipped — see ✅ annotations below

This document captures high-impact feature ideas designed to create clear product separation. Items marked ✅ have shipped.

## 1) Memory Diff API (`changes_since`) ✅ SHIPPED

Return what changed in memory between two points in time.

- Added facts
- Superseded facts
- Confidence deltas
- Session/head changes

Use cases:

- "What changed about this customer since yesterday?"
- proactive summaries and alerts

## 2) Counterfactual Memory (`simulate_as_if`) ✅ SHIPPED

Simulate retrieval context under hypothetical assumptions.

Example:

- "If the user still preferred Adidas, what context would we send the agent?"

Use cases:

- planning agents
- policy simulations
- product what-if analysis

## 3) Conflict Radar ✅ SHIPPED

Build a contradiction layer over facts/edges.

- conflict severity score
- unstable memory clusters
- needs-resolution queue

Use cases:

- trust/debug dashboards
- automatic clarification prompts

## 4) Causal Recall Chains ✅ SHIPPED

Explain why a memory was retrieved by returning lineage.

- source episodes
- derived entities/facts
- supersession path

Use cases:

- agent explainability
- debugging incorrect answers

## 5) Policy-Scoped Memory Views

Multiple retrieval "lenses" over the same memory.

- support-safe view
- sales-safe view
- internal full-trust view

Use cases:

- compliance
- least-privilege agent architectures

## 6) Goal-Conditioned Memory ✅ SHIPPED

Condition retrieval strategy by active objective, not only semantic similarity.

Examples:

- `goal=resolve_ticket`
- `goal=plan_trip`
- `goal=coach_fitness`

Use cases:

- better task-specific context quality

## 7) Confidence Decay + Revalidation ✅ SHIPPED

Facts decay over time unless reinforced.

- temporal confidence decay curves
- revalidation triggers for stale but important facts

Use cases:

- lower stale-memory errors
- active memory maintenance

## 8) Time Travel Debugger ✅ SHIPPED

Inspect memory state at `HEAD`, `as_of`, and diffs between points.

- point-in-time snapshots
- human-readable change timelines

Use cases:

- agent debugging
- incident retrospectives

## 9) Memory Guardrails Engine

Declarative constraints at storage and retrieval time.

- block storage for restricted classes (e.g., certain PII)
- prevent recall of superseded facts outside historical mode
- tenant-specific safety policy packs

Use cases:

- regulated environments
- enterprise policy enforcement

## 10) Self-Healing Memory ✅ SHIPPED

Auto-detect low-confidence conflicts and trigger single-step clarification.

- detect uncertainty/conflicts
- ask one targeted question
- reconcile graph state after answer

Use cases:

- autonomous memory quality improvement

## 11) Cross-Session Narrative Summaries ✅ SHIPPED

Generate evolving "story of the user" with chapter-style diffs.

- weekly/monthly narrative updates
- "what changed" chapter markers

Use cases:

- customer success views
- long-term assistant continuity

## 12) Memory Webhooks ✅ SHIPPED

Emit real-time events from memory lifecycle.

- `fact_added`
- `fact_superseded`
- `head_advanced`
- `conflict_detected`

Use cases:

- event-driven product features
- real-time alerts and workflows

## Consolidated Backlog (Sorted by Win Speed)

### Quick wins

1. ~~Memory Diff API (`changes_since`)~~ ✅ SHIPPED (v0.5.0)
2. ~~Conflict Radar~~ ✅ SHIPPED (v0.5.0)
3. ~~Causal Recall Chains~~ ✅ SHIPPED (v0.5.0)
4. Memory Contracts
5. Adaptive Retrieval Policies
6. ~~Memory Webhooks~~ ✅ SHIPPED (v0.5.0)

### Mid-term wins

7. Policy-Scoped Memory Views
8. ~~Goal-Conditioned Memory~~ ✅ SHIPPED (v0.5.5)
9. ~~Confidence Decay + Revalidation~~ ✅ SHIPPED (v0.5.5)
10. ~~Time Travel Debugger~~ ✅ SHIPPED (v0.5.0)
11. ~~Self-Healing Memory~~ ✅ SHIPPED (v0.5.5)
12. ~~Cross-Session Narrative Summaries~~ ✅ SHIPPED (v0.5.5)
13. Memory Forks
14. Trust-Weighted Recall
15. Intent Drift Detector
16. Multi-Agent Shared Memory with ACLs
17. Forensic Replay
18. Rerun Trace Visualization Integration

### Moonshots

19. ~~Counterfactual Memory (`simulate_as_if`)~~ ✅ SHIPPED (v0.5.5)
20. Auto-Schema Discovery
21. Memory Stress Simulator

## New candidate details

### Rerun Trace Visualization Integration (`mid`)

Integrate [Rerun.io](https://rerun.io/) to visualize memory evolution, retrieval decisions, and conflict dynamics.

v1 scope:

- export `.rrd` traces for episodes, entities, edges, and session HEAD changes
- timeline playback of supersession and fact validity windows
- simple CLI command to generate and open traces

v2 scope:

- live stream mode from ingestion and retrieval pipelines
- overlays for temporal scores, conflict severity, and retrieval source contribution
