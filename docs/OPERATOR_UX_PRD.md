# Operator UX PRD

Status: P0 active
Owner: Platform / Product
Priority: P0
Last updated: 2026-03-04

## 1) Executive Summary

Mnemo's next major moat is Operator UX: a control-plane experience that makes memory systems observable, explainable, and recoverable in minutes.

The product goal is to make incident response, policy governance, and temporal debugging first-class workflows so operators can trust and control memory in production without direct datastore edits.

Manifold conclusion: this must be workflow-first, not dashboard-first. We prioritize two hero operator lanes:

1. dead-letter recovery
2. why-answer-changed RCA

Everything else (dashboard, governance center, trace explorer) supports and accelerates these two lanes.

## 2) Problem Statement

Most memory stacks fail operators in production because they are hard to debug and recover:

- Low visibility into why a memory answer changed.
- Weak tools for replaying failed deliveries and recovering dead-letter backlogs.
- Policy changes that are opaque and hard to audit.
- Fragmented diagnostics across write path, ingest path, retrieval path, and delivery path.

Operator confidence drops when systems require custom scripts or DB access during incidents.

## 3) Product Goals

1. Reduce memory/webhook incident MTTR to under 5 minutes.
2. Make "why changed?" root-cause analysis possible in under 60 seconds.
3. Ensure every policy/destructive action is auditable and inspectable.
4. Give operators one-click recovery loops for common failure classes.

## 3.1) Strategic Decisions (Lincoln Manifold Synthesis)

1. Workflow-first IA over broad nav-first IA.
2. request_id continuity as the primary evidence spine.
3. Bounded policy preview + mandatory post-apply validation.
4. Progressive data loading to protect incident-time latency.
5. Action-affordance coupling: every failure view exposes a safe next action.

## 4) Non-Goals (This PRD)

- Multi-region HA orchestration UI.
- Billing/plan management UI.
- Full tenant IAM/rbac matrix editor (tracked separately).

## 5) Users and JTBD

### Primary users

- AI platform engineers running memory-backed agents in production.
- SRE/ops engineers handling incidents and reliability posture.
- security/compliance owners validating governance and retention controls.

### Jobs to be done

- "When webhook delivery fails, I can identify root cause and recover safely."
- "When an answer shifts, I can prove exactly what memory changed and when."
- "When policy changes, I can assess impact before and after rollout."

## 6) North Star

Turn Mnemo into a memory control plane where operators can:

- observe system health,
- diagnose anomalies with evidence,
- remediate failures quickly,
- and prove governance posture.

## 7) Product Pillars

1. Reliability cockpit
2. time-aware debugging
3. governance center
4. trace explorer
5. incident-first workflows

## 8) UX Principles

- Evidence over abstraction: every summary links to concrete rows/events.
- Fast drill-down: no dead-end screens.
- Deterministic language: timestamps, IDs, counters, status transitions.
- Recovery-first: every failure state exposes next best action.
- Operator density: table/filter/search-first layouts, keyboard-usable.

## 9) Information Architecture

IA ordering is intentional: hero lanes first.

### A) Webhook Operations (Hero Lane 1)

Purpose: reliability and recovery.

Core objects:

- subscriptions, events, dead-letter events, replay cursor, retries, audit.

Key workflows:

- inspect failing webhook -> view failed events -> replay by cursor -> retry selected event -> confirm delivery.

### B) Memory Timeline Debugger (Hero Lane 2)

Purpose: explain memory drift.

Core objects:

- `time_travel/trace` snapshots, gained/lost facts, gained/lost episodes, timeline events.

Key workflows:

- run diff (from/to) -> inspect why changed -> inspect request_id chain -> export incident evidence.

### C) Overview Dashboard

Purpose: system health at a glance.

Primary widgets:

- Request volume and response class counters.
- webhook success/failure/dead-letter trends.
- replay/retry activity.
- policy updates and policy violations.
- active incident panel (dead-letter spikes, circuit-open hooks, policy violations).

### D) Governance Center

Purpose: policy control and compliance posture.

Core objects:

- user policy record, governance audit stream.

Key workflows:

- edit policy defaults -> preview effective behavior -> save -> inspect audit and policy violation telemetry.

### E) Trace Explorer

Purpose: end-to-end joinability.

Core objects:

- `x-mnemo-request-id`, episode metadata, ingest logs, retrieval/timeline rows, webhook events and audit rows.

Key workflows:

- search request_id -> view write/ingest/retrieval/delivery hops -> identify failure origin.

## 10) Functional Requirements

Cross-cutting latency targets for incident UX:

- p95 <= 200ms for summary lists and alert feeds.
- p95 <= 500ms for deep drilldown joins.
- trace explorer deep joins may stream progressively beyond 500ms, but first meaningful result <= 250ms.

### 10.1 Dashboard

- Show live counters from `/metrics` and key gauges.
- Highlight anomaly deltas over 5m, 1h, 24h windows.
- Link each anomaly card to filtered operational views.
- Dashboard is triage-only: no metric card without deep-link action path.

### 10.2 Webhook Ops

- List webhooks with status badges: healthy, degraded, circuit-open.
- show dead-letter backlog per webhook.
- support replay via cursor (`events/replay`).
- support manual retry (`events/:event_id/retry`).
- show webhook audit timeline.

### 10.3 Time Travel Debugger

- Compare snapshots for configurable window.
- visualize gained/lost facts and episodes.
- show event timeline with `request_id` when available.
- filter by session, contract, retrieval policy.

### 10.4 Governance Center

- CRUD-like editing of per-user policy fields:
  - retention days by episode type
  - webhook domain allowlist
  - default memory contract
  - default retrieval policy
- view policy audit timeline.
- display policy violation events and impacted targets.

### 10.5 Trace Explorer

- request_id search with exact and prefix matching.
- show correlated artifacts:
  - originating API calls (if captured)
  - episodes with metadata request_id
  - timeline events and change rows
  - webhook events/delivery attempts/audit rows
- progressive disclosure: summary first, deep raw records on demand.

## 11) API/Backend Mapping

Existing APIs powering v1 UX:

- `/metrics`
- `/api/v1/memory/webhooks/*` (register/get/delete/events/replay/retry/dead-letter/stats/audit)
- `/api/v1/memory/:user/changes_since`
- `/api/v1/memory/:user/time_travel/trace`
- `/api/v1/policies/:user`
- `/api/v1/policies/:user/audit`
- `/api/v1/ops/summary`
- `/api/v1/traces/:request_id`

Likely backend additions for UX polish:

- pre-aggregated incident summaries endpoint.
- query endpoint for governance violations by time window.
- request_id lookup endpoint for faster cross-index joins.

## 12) Proposed Data Contracts for UI

### Incident card

- id
- type (`dead_letter_spike`, `policy_violation_spike`, `circuit_open`)
- severity
- started_at
- affected_resource_ids
- suggested_action

### Timeline row

- at
- event_type
- description
- session_id
- episode_id
- edge_id
- request_id

### Governance audit row

- at
- action
- request_id
- actor (future)
- details

## 13) UX Flows (Critical)

### Flow 1: Dead-letter recovery

1. Dashboard alert opens filtered Webhook Ops.
2. Operator reviews dead-letter events and errors.
3. Operator replays cursor window and/or retries selected events.
4. Operator confirms delivery success and backlog reduction.

### Flow 2: "Why did answer change?"

1. Operator opens Time Travel Debugger.
2. Sets from/to, query, optional session.
3. Reviews gained/lost facts + timeline + request_id links.
4. Exports evidence to incident ticket.

### Flow 3: Policy hardening rollout

1. Operator edits allowlist/retention/defaults.
2. Saves policy.
3. Watches governance audit + policy violation metrics.
4. Rolls back if violations spike.

## 14) Success Metrics

Primary:

- MTTR for webhook incidents < 5 minutes.
- "Why changed" investigation < 60 seconds median.
- dead-letter recovery success > 90% first remediation cycle.
- 100% policy/destructive operations visible in audit timeline.

Secondary:

- operator satisfaction score (internal) >= 8/10.
- fewer manual DB/script interventions per incident.

## 15) Falsification Plan

### 15.1 Functional falsification

- Replay cursor excludes duplicates and respects pagination boundaries.
- manual retry changes delivery state from dead-letter -> delivered when sink recovers.
- policy allowlist blocks disallowed hosts, allows approved hosts/subdomains.
- retention write guards reject stale episode writes per type.
- policy defaults apply only when request fields are omitted.

### 15.2 UX falsification

- run incident task drills with novice + expert operators.
- measure time-to-diagnose and time-to-remediate.
- inject failure scenarios:
  - intermittent 5xx sink
  - circuit-open recovery
  - policy misconfiguration rollback

### 15.3 Reliability falsification

- keep workspace and integration suites green.
- smoke + eval sanity after each UX-affecting backend increment.
- enforce CI quality budgets for temporal accuracy, stale rate, and p95 latency.

## 16) Delivery Plan (Phased)

### Phase 1: Hero Lane 1 (P0) — Dead-letter Recovery

- webhook ops table + event drill-down
- dead-letter + retry/replay action panel
- verification panel with backlog burn-down and delivery confirmation

### Phase 2: Hero Lane 2 (P0) — Why-Changed RCA

- time travel comparison UI
- gained/lost panes
- request_id linkage from timeline rows

### Phase 3: Governance Center + Triage Dashboard (P0)

- policy editor
- governance audit stream
- violation monitoring cards
- dashboard shell with actionable anomaly cards

### Phase 4: Trace Explorer (P1)

- global request_id search
- multi-hop event graph
- saved views and runbook links

## 17) Risks and Mitigations

- Risk: UI complexity overwhelms operators.
  - Mitigation: progressive disclosure; default "incident mode" views.
- Risk: expensive joins for trace explorer.
  - Mitigation: cache/index request_id lookup paths.
- Risk: policy over-enforcement breaks legacy workflows.
  - Mitigation: high defaults, staged rollout, explicit audit + rollback.

## 18) Open Questions

- Should policy be user-level only or add org-level inheritance now?
- Do we need role-based policy edit permissions in the same release?
- Which charts are essential for day-one dashboard vs phase-two?

## 19) Immediate Next Step

Convert this PRD into implementation tickets:

1. API gap list for UI query efficiency.
2. component-level wireframe spec.
3. acceptance criteria per screen with falsification checks.
