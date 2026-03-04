# P0 PRD: Cloud-Grade Ops Control Plane

Status: P0 (active)
Owner: Core platform
Last updated: 2026-03-04

## Why this exists

Mnemo already has strong memory primitives (temporal recall, contracts, policies, webhooks, time-travel trace).
The next step-change is operational trust at production scale: predictable delivery, replayability, governance, and measurable reliability.

## Product goals

1. Make webhook/event operations replayable and operator-friendly.
2. Provide auditable operational actions and delivery lifecycle records.
3. Keep reliability behavior explicit: dead letters, retries, circuit behavior.
4. Gate future releases on falsification metrics, not happy-path demos.

## Scope (P0)

### Shipped in this slice

- Webhook event replay API with cursor semantics.
- Manual event retry endpoint for dead-letter recovery.
- Webhook operational audit log endpoint.
- Redis persistence for webhook subscriptions, events, and audit rows.
- Request correlation propagation via `x-mnemo-request-id`.
- Prometheus-compatible `/metrics` endpoint for HTTP and webhook delivery telemetry.
- CI temporal quality budget gate in `.github/workflows/quality-gates.yml`.
- Webhook event/audit records now retain originating request correlation IDs for incident traceability.
- User governance policy endpoints with webhook allowlist enforcement and audit trails (`/api/v1/policies/:user`, `/api/v1/policies/:user/audit`).
- Request IDs now persist into episode metadata and surface in `changes_since`/`time_travel/trace` for cross-pipeline joins.
- Policy defaults now drive memory contract/retrieval fallback, and per-type retention windows enforce episode-write freshness.

### Planned next in P0

- Prometheus metrics endpoint and latency/error counters.
- Request correlation IDs across ingest/retrieval/webhook delivery.
- Tenant policy/audit surfacing for auth + retention controls.
- CI quality budgets (latency ceilings + stale-fact regression thresholds).

## API surface (P0)

- `GET /api/v1/memory/webhooks/:id/events/replay`
- `POST /api/v1/memory/webhooks/:id/events/:event_id/retry`
- `GET /api/v1/memory/webhooks/:id/audit`
- Existing: events, dead-letter, stats, register/get/delete

## Non-goals (for this PRD slice)

- Multi-region active-active failover.
- Exactly-once delivery guarantees.
- Cross-tenant billing/rate plan enforcement.

## Reliability model

- Delivery is at-least-once.
- Events can enter dead-letter after retry exhaustion.
- Operators can replay and manually retry events when downstream recovers.
- Circuit breaker and rate limiting protect downstream systems.

## Falsification matrix (must pass)

1. Replay cursor correctness
   - Given `after_event_id`, replay returns strictly later events only.
2. Dead-letter recovery
   - Event fails into dead-letter, manual retry succeeds once sink recovers.
3. Audit integrity
   - Registration, retry queueing, and dead-letter transitions appear in audit log.
4. Persistence survivability
   - Restart restores webhook subscriptions/events/audit from Redis.
5. Regression safety
   - `mnemo-server` suite remains green; smoke remains green.

## Success criteria

- Operators can recover failed webhook deliveries without direct datastore edits.
- Post-incident investigations can answer "what happened and when" from API-accessible audit rows.
- No regressions in temporal quality or existing API behavior.

## Rollout / verification

- Keep default behavior backward compatible.
- Validate via integration tests + smoke + temporal eval sanity.
- Track changelog under `Unreleased` for every control-plane increment.
