# Operator UX Execution Backlog

Status: Active
Source: `docs/OPERATOR_UX_PRD.md`
Updated: 2026-03-04

## P0 Completion Tracker (Strict)

- Backend/API foundations: 6/6 complete (2 dead-letter recovery + 2 RCA + 2 governance).
- Operator drill automation: 1/1 complete (`tests/operator_p0_drills.sh`).
- UX surfaces in this repository: 0/12 complete (no frontend package present in current workspace).
- Cross-cutting acceptance gates: 2/4 complete (quality/falsification gates + audit action coverage); latency SLO drill capture pending.

### Done now

- [x] Hero lane backend coverage shipped (`ops/summary`, `traces/:request_id`, `time_travel/summary`, policy preview, policy violations).
- [x] Operator P0 scripted drills for dead-letter recovery, why-changed RCA, and governance violation triage.
- [x] Docs/API/test guides aligned with shipped endpoints.

### Remaining to declare full P0 UX finish

- [ ] Implement operator-facing frontend surfaces (webhook ops grid, RCA canvas, governance center, dashboard cards).
- [ ] Capture and publish p95 latency evidence for summary/drilldown endpoints under representative load.
- [ ] Record and link demo artifacts for Milestones A/B/C.

## Epic 1: Hero Lane 1 — Dead-Letter Recovery

### UX Tickets

- [ ] Build webhook health grid with status facets (healthy/degraded/circuit-open).
- [ ] Build dead-letter queue view with event details pane.
- [ ] Add replay cursor action modal with guardrails.
- [ ] Add single-event retry action with confirmation and post-action verification.

### Backend/API Tickets

- [x] Add aggregate webhook incident summary endpoint for dashboard cards.
- [x] Add retry outcome status envelope for immediate UX confirmation.

### Falsification

- [x] Simulate intermittent 5xx sink and verify operator can clear queue in < 5 min.
- [ ] Validate replay cursor boundaries under pagination and sparse event IDs.

## Epic 2: Hero Lane 2 — Why-Changed RCA

### UX Tickets

- [ ] Build time travel compare canvas (`from/to` snapshots).
- [ ] Build gained/lost facts and episodes side-by-side panes.
- [ ] Build timeline list with request_id drilldown.
- [ ] Add export bundle for incident evidence.

### Backend/API Tickets

- [x] Add optional endpoint for request_id-centric trace expansion (if latency requires).
- [x] Add lightweight timeline summary object for fast initial render.

### Falsification

- [x] Run "why changed" task drill with target < 60s to root cause.
- [ ] Validate consistency under contract/retrieval override combinations.

## Epic 3: Governance Center + Actionable Dashboard

### UX Tickets

- [ ] Build policy editor with retention/allowlist/defaults sections.
- [ ] Add bounded impact preview panel and post-apply validation panel.
- [ ] Build governance audit feed with diff rendering.
- [ ] Build dashboard anomaly cards with mandatory deep-link actions.

### Backend/API Tickets

- [x] Add policy preview API (estimated impact with confidence label).
- [x] Add policy violation time-window query endpoint.

### Falsification

- [x] Misconfiguration drill: apply bad allowlist, detect spike, rollback cleanly.
- [ ] Verify 100% policy/destructive operations appear in audit feed.

## Cross-Cutting Acceptance Gates

- [ ] p95 <= 200ms for summary views.
- [ ] p95 <= 500ms for drilldown joins (or progressive render fallback).
- [x] No operator action without audit event (including entity/edge deletes as of v0.3.1).
- [x] Workspace gates + integration + smoke + eval pass on every major increment.

## Demo Milestones

1. Demo A: dead-letter incident from alert to clear backlog.
2. Demo B: answer-drift RCA with request_id hop evidence.
3. Demo C: policy change with preview, apply, validate, and audit replay.
