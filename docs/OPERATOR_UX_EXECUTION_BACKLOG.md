# Operator UX Execution Backlog

Status: Active
Source: `docs/OPERATOR_UX_PRD.md`
Updated: 2026-03-04

## Epic 1: Hero Lane 1 — Dead-Letter Recovery

### UX Tickets

- [ ] Build webhook health grid with status facets (healthy/degraded/circuit-open).
- [ ] Build dead-letter queue view with event details pane.
- [ ] Add replay cursor action modal with guardrails.
- [ ] Add single-event retry action with confirmation and post-action verification.

### Backend/API Tickets

- [ ] Add aggregate webhook incident summary endpoint for dashboard cards.
- [ ] Add retry outcome status envelope for immediate UX confirmation.

### Falsification

- [ ] Simulate intermittent 5xx sink and verify operator can clear queue in < 5 min.
- [ ] Validate replay cursor boundaries under pagination and sparse event IDs.

## Epic 2: Hero Lane 2 — Why-Changed RCA

### UX Tickets

- [ ] Build time travel compare canvas (`from/to` snapshots).
- [ ] Build gained/lost facts and episodes side-by-side panes.
- [ ] Build timeline list with request_id drilldown.
- [ ] Add export bundle for incident evidence.

### Backend/API Tickets

- [ ] Add optional endpoint for request_id-centric trace expansion (if latency requires).
- [ ] Add lightweight timeline summary object for fast initial render.

### Falsification

- [ ] Run "why changed" task drill with target < 60s to root cause.
- [ ] Validate consistency under contract/retrieval override combinations.

## Epic 3: Governance Center + Actionable Dashboard

### UX Tickets

- [ ] Build policy editor with retention/allowlist/defaults sections.
- [ ] Add bounded impact preview panel and post-apply validation panel.
- [ ] Build governance audit feed with diff rendering.
- [ ] Build dashboard anomaly cards with mandatory deep-link actions.

### Backend/API Tickets

- [ ] Add policy preview API (estimated impact with confidence label).
- [ ] Add policy violation time-window query endpoint.

### Falsification

- [ ] Misconfiguration drill: apply bad allowlist, detect spike, rollback cleanly.
- [ ] Verify 100% policy/destructive operations appear in audit feed.

## Cross-Cutting Acceptance Gates

- [ ] p95 <= 200ms for summary views.
- [ ] p95 <= 500ms for drilldown joins (or progressive render fallback).
- [ ] No operator action without audit event.
- [ ] Workspace gates + integration + smoke + eval pass on every major increment.

## Demo Milestones

1. Demo A: dead-letter incident from alert to clear backlog.
2. Demo B: answer-drift RCA with request_id hop evidence.
3. Demo C: policy change with preview, apply, validate, and audit replay.
