# Operator Dashboard PRD

Status: P0 active
Owner: Platform / Product
Priority: P0
Last updated: 2026-03-04

## 1) Executive Summary

Mnemo has 62 API endpoints, 56 integration tests, and a governance/operability story that makes Zep look like a toy. None of this is visible to anyone who isn't reading curl output.

This PRD defines a minimal, self-hosted operator dashboard that ships inside the Mnemo server binary — no separate frontend build, no Node.js, no npm. A single-binary server that serves both the API and an embedded dashboard at `/_/` using server-rendered HTML with minimal JavaScript for interactivity.

Manifold conclusion: the fastest path to making the ops story tangible is an embedded dashboard, not a separate SPA. Shipping the dashboard inside the server binary means zero additional deployment, zero additional dependencies, and operators get the dashboard the moment they upgrade.

## 2) Problem Statement

1. **Invisible capabilities.** Governance policies, time-travel trace, webhook ops, operator summary — all exist as API endpoints but require curl or Postman to access. Prospects evaluating Mnemo cannot see these capabilities without reading docs.
2. **No incident workflow without scripting.** Dead-letter recovery, RCA trace-joining, and policy violation triage all require chaining multiple curl commands. The `operator_p0_drills.sh` script proves the workflow works, but operators need a UI, not a shell script.
3. **No demo-ability.** Milestones A/B/C in the Operator UX Backlog require demo artifacts. You cannot demo curl commands to a stakeholder.
4. **Zep Cloud has a web UI.** Even though it's limited, the existence of a UI creates a perception of maturity that Mnemo lacks.

## 3) Product Goals

1. **Zero-deployment dashboard.** Operator navigates to `http://server:8080/_/` and gets the full dashboard. No separate service, no frontend build step, no container.
2. **Three hero workflows are point-and-click.** Dead-letter recovery, why-changed RCA, and governance triage — all completable without curl.
3. **Real-time system health at a glance.** One screen shows health, active alerts, webhook circuit state, and recent audit activity.
4. **Demo-ready in 5 minutes.** The dashboard itself is the demo artifact for Milestones A/B/C.

## 4) Non-Goals

- Full CRUD admin panel for users, sessions, episodes (future).
- Agent identity management UI (future).
- Real-time WebSocket streaming (polling is fine for v1).
- CSS framework or design system (utility CSS + semantic HTML is sufficient).
- Authentication UI (dashboard inherits the server's API key auth).
- Mobile-responsive layout (desktop operators only).

## 5) Architecture

### 5.1) Embedded Dashboard Approach

The dashboard is served by the Mnemo server itself. Static HTML, CSS, and JavaScript files are embedded in the Rust binary at compile time using `include_str!` or `rust-embed`. The server registers dashboard routes under `/_/`:

```
/_/                → Dashboard home (system overview)
/_/webhooks        → Webhook operations grid
/_/webhooks/:id    → Webhook detail + dead-letter + replay
/_/rca             → Time-travel RCA canvas
/_/governance      → Governance center
/_/governance/:user → Policy editor + audit feed
/_/traces/:req_id  → Request-id trace viewer
/_/graph/:user     → Knowledge graph explorer
```

All dashboard pages are server-side rendered HTML with embedded `<script>` tags for interactivity. No build step — the HTML/CSS/JS files are authored directly and embedded at compile time.

### 5.2) Technology Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| HTML rendering | Server-side templates via Rust `askama` or raw `include_str!` | No frontend build; templates compile into the binary |
| CSS | Single embedded `style.css` (~500 lines utility CSS) | No npm, no Tailwind build, no external CDN |
| JavaScript | Vanilla JS, `<script>` tags (~2000 lines total) | No React, no bundler, no node_modules |
| Charts | Inline SVG or `<canvas>` with minimal JS | No chart library dependency |
| HTTP | `fetch()` calls to the existing API endpoints | Dashboard is a client of the same API |
| Data refresh | `setInterval` polling (configurable, default 5s for health, 30s for lists) | Simple, no WebSocket complexity |
| Embedding | `rust-embed` crate or `include_str!` macros | Zero runtime file I/O; dashboard files baked into binary |

### 5.3) Route Registration

```rust
// In routes.rs or a new dashboard.rs module
fn dashboard_routes() -> Router {
    Router::new()
        .route("/_/", get(dashboard_home))
        .route("/_/webhooks", get(dashboard_webhooks))
        .route("/_/webhooks/:id", get(dashboard_webhook_detail))
        .route("/_/rca", get(dashboard_rca))
        .route("/_/governance", get(dashboard_governance))
        .route("/_/governance/:user", get(dashboard_governance_user))
        .route("/_/traces/:request_id", get(dashboard_trace))
        .route("/_/graph/:user", get(dashboard_graph))
        .route("/_/static/style.css", get(serve_css))
        .route("/_/static/app.js", get(serve_js))
}
```

### 5.4) Page Architecture

Each page follows the same pattern:

```html
<!DOCTYPE html>
<html>
<head>
  <title>Mnemo — {{page_title}}</title>
  <link rel="stylesheet" href="/_/static/style.css">
</head>
<body>
  <nav class="sidebar">
    <!-- Fixed nav with links to all sections -->
  </nav>
  <main class="content">
    <!-- Page-specific content -->
    <!-- Data placeholders filled by fetch() on load -->
  </main>
  <script src="/_/static/app.js"></script>
  <script>
    // Page-specific initialization
    mnemo.init('{{page_id}}', { /* page config */ });
  </script>
</body>
</html>
```

The `app.js` file contains:
- `mnemo.api(method, path, body?)` — wrapper around `fetch()` that adds auth headers and handles errors
- `mnemo.poll(path, interval, callback)` — polling helper
- `mnemo.render(elementId, template, data)` — simple template rendering
- `mnemo.confirm(message)` — confirmation modal for destructive actions
- Page-specific modules for each dashboard section

## 6) Dashboard Screens

### 6.1) Home — System Overview (`/_/`)

**Data sources:** `GET /health`, `GET /api/v1/ops/summary`

**Layout:**
```
┌─────────────────────────────────────────────────┐
│  Mnemo v0.3.1          [health: ●]  [uptime]    │
├─────────┬─────────┬─────────┬───────────────────┤
│ Users   │ Episodes│ Webhooks│ Dead-Letter Queue  │
│  142    │  8,341  │  12     │  3 ⚠               │
├─────────┴─────────┴─────────┴───────────────────┤
│  Recent Activity (last 5 min)                    │
│  ├─ 14:23:01  policy_updated  user:acme_bot      │
│  ├─ 14:22:45  delivery_dead_letter  wh:abc123    │
│  ├─ 14:22:30  entity_deleted  user:test_cleanup  │
│  └─ 14:22:12  head_advanced  user:kendra         │
├──────────────────────────────────────────────────┤
│  Webhook Circuit State                           │
│  ├─ hooks.acme.example     ● closed (healthy)    │
│  ├─ alerts.internal.co     ◐ half-open (testing) │
│  └─ old-system.legacy.io   ○ open (5 failures)   │
└──────────────────────────────────────────────────┘
```

**Polling:** Health every 5s, ops summary every 10s.

### 6.2) Webhook Operations (`/_/webhooks`)

**Data sources:** `GET /api/v1/memory/webhooks/:id/stats` for each webhook, `GET /api/v1/memory/webhooks/:id/events/dead-letter`

**Layout:**
```
┌──────────────────────────────────────────────────┐
│  Webhook Operations                              │
├──────┬────────────┬────────┬──────┬──────┬───────┤
│  ID  │ Target URL │ Status │ Pend │ Dead │  Rate │
├──────┼────────────┼────────┼──────┼──────┼───────┤
│ abc  │ hooks.acme │ ● OK   │   2  │  0   │ 12/m  │
│ def  │ alerts.int │ ◐ Test │   0  │  3   │  0/m  │
│ ghi  │ old.legacy │ ○ Open │   0  │ 14   │  0/m  │
└──────┴────────────┴────────┴──────┴──────┴───────┘
         [Click row to drill into webhook detail]
```

### 6.3) Webhook Detail (`/_/webhooks/:id`)

**Data sources:** Stats, events, dead-letter, audit for this webhook.

**Operator actions:**
- **[Retry]** button on each dead-letter event → `POST .../retry`
- **[Replay All]** button → opens replay modal with cursor pagination
- **[Delete Webhook]** button → confirmation modal → `DELETE .../webhooks/:id`

**Layout:**
```
┌──────────────────────────────────────────────────┐
│  Webhook: hooks.acme.example                     │
│  Status: ● closed   Circuit: healthy             │
│  Events: head_advanced, conflict_detected        │
├──────────────────────────────────────────────────┤
│  Delivery Stats (last 5 min)                     │
│  Total: 47  Delivered: 44  Pending: 2  Dead: 1   │
├──────────────────────────────────────────────────┤
│  Dead-Letter Queue                               │
│  ┌─────┬──────────┬─────────┬─────────┬────────┐ │
│  │ ID  │ Event    │ Attempts│ Error   │ Action │ │
│  ├─────┼──────────┼─────────┼─────────┼────────┤ │
│  │ e01 │ head_adv │    3    │ timeout │ [Retry]│ │
│  └─────┴──────────┴─────────┴─────────┴────────┘ │
├──────────────────────────────────────────────────┤
│  Audit Log                                       │
│  ├─ 14:23  retry_queued (request_id: req-abc)    │
│  ├─ 14:20  delivery_dead_letter                  │
│  └─ 14:15  webhook_registered                    │
└──────────────────────────────────────────────────┘
```

### 6.4) RCA Canvas (`/_/rca`)

**Data sources:** `POST .../time_travel/trace`, `POST .../time_travel/summary`, `GET /api/v1/traces/:request_id`

**Layout:**
```
┌──────────────────────────────────────────────────┐
│  Why-Changed RCA                                 │
│  User: [________]  Query: [____________________] │
│  From: [2025-02-01]        To: [2025-04-01]      │
│  Contract: [default ▾]  Policy: [balanced ▾]     │
│  [Run Trace]                                     │
├────────────────────┬─────────────────────────────┤
│  Snapshot FROM      │  Snapshot TO               │
│  As of: Feb 1       │  As of: Apr 1              │
│  Facts: 3           │  Facts: 5                  │
│  Episodes: 8        │  Episodes: 12              │
│  Tokens: 340        │  Tokens: 520               │
├────────────────────┴─────────────────────────────┤
│  Gained Facts                                    │
│  ├─ Kendra → prefers → Nike (valid Mar 10)       │
│  └─ Kendra → training → Boston Marathon          │
│                                                  │
│  Lost Facts                                      │
│  └─ Kendra → prefers → Adidas (invalidated Feb)  │
├──────────────────────────────────────────────────┤
│  Timeline                                        │
│  ●─────●─────────●──────────●──────────→         │
│  Feb 1  Feb 20    Mar 10     Apr 1               │
│         fact_sup  fact_add                        │
│         erseded   ed                              │
├──────────────────────────────────────────────────┤
│  Policy Diagnostics                              │
│  max_tokens: 500  min_relevance: 0.30            │
│  temporal_intent: historical  weight: null        │
└──────────────────────────────────────────────────┘
```

### 6.5) Governance Center (`/_/governance`)

**Data sources:** `GET /api/v1/policies/:user`, `GET .../audit`, `GET .../violations`

**Layout:**
```
┌──────────────────────────────────────────────────┐
│  Governance Center                               │
│  User: [________] [Load Policy]                  │
├──────────────────────────────────────────────────┤
│  Current Policy                                  │
│  ├─ retention_days_message: 365                  │
│  ├─ retention_days_text:    180                  │
│  ├─ retention_days_json:    90                   │
│  ├─ webhook_domain_allowlist: [hooks.acme.ex]    │
│  ├─ default_contract: default                    │
│  └─ default_retrieval_policy: balanced           │
│                                                  │
│  [Edit Policy]  [Preview Impact]                 │
├──────────────────────────────────────────────────┤
│  Recent Violations (last 24h)                    │
│  ├─ 14:23  policy_violation_webhook_domain       │
│  │         target: evil.example  req: req-abc    │
│  └─ 09:15  policy_violation_retention            │
│            type: message  age: 400d              │
├──────────────────────────────────────────────────┤
│  Audit Trail                                     │
│  ├─ 14:23  policy_updated  (req: req-xyz)        │
│  ├─ 14:20  entity_deleted  (entity: Nike)        │
│  ├─ 14:15  session_deleted (session: old-chat)   │
│  └─ 14:10  user_deleted   (GDPR wipe)           │
└──────────────────────────────────────────────────┘
```

### 6.6) Request Trace Viewer (`/_/traces/:request_id`)

**Data sources:** `GET /api/v1/traces/:request_id`

**Layout:**
```
┌──────────────────────────────────────────────────┐
│  Trace: req-abc-123                              │
│  Window: 2026-03-04 14:00 → 14:30               │
├──────────────────────────────────────────────────┤
│  Episodes (2 matches)                            │
│  ├─ ep-001  message  "Kendra prefers Nike"       │
│  └─ ep-002  message  "Updated renewal status"    │
├──────────────────────────────────────────────────┤
│  Webhook Events (1 match)                        │
│  └─ evt-abc  head_advanced  delivered ✓          │
├──────────────────────────────────────────────────┤
│  Governance Audit (1 match)                      │
│  └─ policy_override_context  contract=default    │
└──────────────────────────────────────────────────┘
```

### 6.7) Knowledge Graph Explorer (`/_/graph/:user`)

**Data sources:** `GET /api/v1/users/:user_id/entities`, `GET /api/v1/entities/:id/subgraph`

Renders an interactive graph using `<canvas>` with a simple force-directed layout in vanilla JS. Nodes are entities, edges are relationships. Click a node to see its detail panel. Invalidated edges are shown as dashed lines.

## 7) Execution Plan

### Milestone D1: Infrastructure — Embed and Serve (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D1-1 | Add `rust-embed` to `mnemo-server` dependencies. Create `crates/mnemo-server/dashboard/` directory for static files. | `cargo check` passes. |
| D1-2 | Create `dashboard.rs` module with route registration. Serve `/_/` with a placeholder HTML page. | `curl http://localhost:8080/_/` returns HTML with 200. |
| D1-3 | Create `style.css` with base layout (sidebar + content + cards). | Dashboard page renders with styled layout. |
| D1-4 | Create `app.js` with `mnemo.api()`, `mnemo.poll()`, and `mnemo.render()` helpers. | Browser console: `mnemo.api('GET', '/health')` returns health JSON. |

### Milestone D2: System Overview Home (0.5 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D2-1 | Implement `/_/` home page with health indicator, metric cards, recent activity feed. | Load page, verify it shows real data from ops/summary. |
| D2-2 | Add 5-second health polling and 10-second ops summary polling. | Health dot updates within 5s of server restart. |
| D2-3 | Add webhook circuit state panel. | Panel shows correct circuit state per webhook. |

### Milestone D3: Webhook Operations (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D3-1 | Implement `/_/webhooks` grid with stats per webhook. | Grid shows correct pending/dead/delivered counts. |
| D3-2 | Implement `/_/webhooks/:id` detail with dead-letter queue table. | Dead-letter events display with correct attempt counts. |
| D3-3 | Add [Retry] button on dead-letter events. | Click Retry → event moves to delivered state → UI updates. |
| D3-4 | Add [Replay All] button with cursor pagination modal. | Click Replay → events paginate correctly via cursor. |
| D3-5 | Add audit log panel. | Shows `webhook_registered`, `retry_queued`, etc. |

### Milestone D4: RCA Canvas (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D4-1 | Implement `/_/rca` form with user, query, from/to, contract, policy inputs. | Form submits and displays trace response. |
| D4-2 | Render snapshot comparison (from/to side-by-side). | Snapshots show correct fact/episode counts. |
| D4-3 | Render gained/lost facts panels. | Facts display with correct valid_at/invalid_at. |
| D4-4 | Render timeline visualization. | Timeline shows events in chronological order. |
| D4-5 | Display retrieval_policy_diagnostics panel. | Diagnostics show resolved effective values. |

### Milestone D5: Governance Center (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D5-1 | Implement `/_/governance` user lookup and policy display. | Load user, see current policy values. |
| D5-2 | Add policy editor form with [Save] action. | Edit retention_days, save, verify policy updated. |
| D5-3 | Add [Preview Impact] button. | Preview shows estimated affected episode counts. |
| D5-4 | Add violations timeline panel. | Violations display with correct time window. |
| D5-5 | Add audit trail panel. | Shows all governance audit records for user. |

### Milestone D6: Trace Viewer + Graph Explorer (1 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D6-1 | Implement `/_/traces/:request_id` viewer. | Enter request_id, see correlated episodes/webhooks/audit. |
| D6-2 | Implement `/_/graph/:user` entity list and subgraph fetch. | Load user, see entities and BFS subgraph. |
| D6-3 | Canvas-based force-directed graph rendering. | Nodes and edges render, click node shows detail. |
| D6-4 | Invalidated edges shown as dashed lines. | Create superseded edge, verify visual distinction. |

### Milestone D7: Polish and Integration (0.5 day)

| Ticket | Description | Falsification |
|--------|-------------|---------------|
| D7-1 | Navigation sidebar with active-state highlighting. | All nav links work, current page highlighted. |
| D7-2 | Error handling and loading states on all panels. | Disconnect server, verify graceful degradation. |
| D7-3 | Confirmation modals on all destructive actions. | Delete webhook → modal → confirm → deleted. |
| D7-4 | Update `docs/API.md` to document `/_/` dashboard routes. | API docs include dashboard section. |

## 8) Falsification Matrix

| Claim | Falsification Test | Pass Criteria |
|-------|-------------------|---------------|
| Dashboard serves from server binary | `cargo build --release`, run binary, `curl /_/` | Returns HTML 200 with no external file deps |
| Dead-letter recovery is point-and-click | Create failing webhook, trigger dead-letter, click [Retry] in UI | Event transitions to delivered without curl |
| RCA is completable in UI | Enter user/query/window in RCA form, submit | Gained/lost facts and timeline render correctly |
| Policy edit works in UI | Edit retention_days in governance form, save | `GET /policies/:user` reflects new value |
| Policy preview works in UI | Click [Preview Impact] with new retention | Impact counts display correctly |
| Graph renders | Load graph page for user with entities/edges | Canvas shows nodes and edges |
| Polling updates in real-time | Change data via API, wait for poll interval | Dashboard reflects change without manual refresh |
| Auth inherits from server | Set `MNEMO_AUTH_ENABLED=true`, load dashboard without key | Returns 401 |
| No external dependencies | Disconnect internet, load dashboard | All CSS/JS loads from embedded binary |

## 9) Docker Testing Strategy

All dashboard testing uses Docker Compose to stand up a full Mnemo environment:

```yaml
# docker-compose.dashboard-test.yml
services:
  redis:
    image: redis/redis-stack:latest
    ports: ["6379:6379"]
  qdrant:
    image: qdrant/qdrant:latest
    ports: ["6333:6333", "6334:6334"]
  mnemo:
    build: .
    ports: ["8080:8080"]
    environment:
      MNEMO_REDIS_URL: redis://redis:6379
      MNEMO_QDRANT_URL: http://qdrant:6334
      MNEMO_SERVER_HOST: 0.0.0.0
      MNEMO_SERVER_PORT: 8080
    depends_on: [redis, qdrant]
```

Dashboard smoke tests:

```bash
#!/usr/bin/env bash
# tests/dashboard_smoke.sh

set -euo pipefail
BASE="${1:-http://localhost:8080}"

echo "=== Dashboard smoke tests ==="

# 1. Home page loads
status=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/_/")
[ "$status" = "200" ] || { echo "FAIL: /_/ returned $status"; exit 1; }

# 2. CSS loads
status=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/_/static/style.css")
[ "$status" = "200" ] || { echo "FAIL: style.css returned $status"; exit 1; }

# 3. JS loads
status=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/_/static/app.js")
[ "$status" = "200" ] || { echo "FAIL: app.js returned $status"; exit 1; }

# 4. All dashboard routes return 200
for path in "/_/" "/_/webhooks" "/_/rca" "/_/governance"; do
    status=$(curl -s -o /dev/null -w '%{http_code}' "$BASE$path")
    [ "$status" = "200" ] || { echo "FAIL: $path returned $status"; exit 1; }
done

echo "All dashboard smoke tests passed."
```

## 10) Competitive Impact

| Capability | Zep Cloud UI | Mnemo Dashboard (after this PRD) |
|-----------|-------------|----------------------------------|
| Self-hosted | No (cloud only) | Yes (embedded in server binary) |
| Dead-letter recovery | No webhook system | Point-and-click retry + replay |
| Why-changed RCA | No time-travel | Full RCA canvas with gained/lost diffs |
| Governance editor | No governance | Policy editor + preview + violations |
| Request-id tracing | No trace correlation | Cross-pipeline trace viewer |
| Knowledge graph viz | No graph UI | Force-directed canvas rendering |
| Deployment overhead | Separate service | Zero (same binary) |
| External dependencies | React, Node.js | None (vanilla HTML/CSS/JS) |

## 11) Risk Register

| Risk | Mitigation |
|------|------------|
| Vanilla JS becomes unmaintainable as dashboard grows | Keep pages independent; each page is < 300 lines JS. Migrate to lightweight framework only if > 10 pages. |
| `rust-embed` increases binary size | Dashboard is ~50KB of HTML/CSS/JS. Binary is already 30MB+ release. Negligible. |
| Dashboard auth bypasses API auth | Dashboard routes go through the same middleware stack. No separate auth path. |
| Graph rendering performance with large graphs | Limit default BFS depth to 2, max_nodes to 50. Progressive loading on expand. |
| CSS conflicts with future themes | All dashboard CSS is scoped under `.mnemo-dashboard` class. |
| Template injection / XSS | All dynamic data rendered via `textContent`, never `innerHTML`. API responses are JSON-escaped by default. |

## 12) Rollout Criteria

### Gate 1: Skeleton (D1 complete)
- `/_/` serves HTML from embedded binary
- CSS and JS load without external dependencies
- API helper works from browser console

### Gate 2: Hero Workflows (D2 + D3 + D4 + D5 complete)
- Dead-letter → Retry workflow completable in UI
- RCA trace renders correctly
- Policy edit + preview works

### Gate 3: Ship (D6 + D7 complete)
- All pages functional
- Smoke test passes
- Destructive actions have confirmation modals
- README updated with dashboard section
