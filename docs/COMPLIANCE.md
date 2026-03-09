# Mnemo Compliance Overview

This document summarizes Mnemo's compliance posture for operators preparing
for SOC 2 Type II readiness assessments. For the detailed control-by-control
mapping, see [SECURITY_CONTROLS.md](SECURITY_CONTROLS.md).

---

## Scope

Mnemo is an AI memory control plane that ingests, stores, retrieves, and
governs conversational memory on behalf of LLM-powered applications. The
compliance boundary covers:

- **Mnemo Server** — the Rust API server (`mnemo-server`)
- **Ingest Pipeline** — background episode processing (`mnemo-ingest`)
- **Knowledge Graph** — entity/edge storage and traversal (`mnemo-graph`)
- **Storage Layer** — Redis (state, audit) and Qdrant (vectors)
- **Operator Dashboard** — embedded SPA served by mnemo-server
- **Client SDKs** — Python and TypeScript (out of scope for SOC 2; they
  are thin HTTP wrappers with no data persistence)

## Trust Services Criteria Coverage

| TSC Category | Coverage | Key Controls |
|---|---|---|
| **CC1** Control Environment | Full | Security docs, operator dashboard, API key scoping |
| **CC2** Communication | Full | Governance audit trail, webhook events, unified audit export |
| **CC3** Risk Assessment | Full | P0 roadmap, incident panel, HMAC-signed exports |
| **CC5** Control Activities | Full | Bearer auth, Docker health checks, TLS enforcement |
| **CC6** Access Controls | Full | API key auth, per-user policies, encryption in transit |
| **CC7** System Operations | Full | Incident detection, dead-letter monitoring, circuit breakers |
| **CC8** Change Management | Full | CI gates, Docker image versioning, GHCR publish |
| **CC9** Risk Mitigation | Full | Retention policies, LLM provider abstraction |
| **A1** Availability | Full | Health checks, resource limits, restart policies |

## Authentication

Mnemo supports bearer token authentication via environment variables:

```
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=key1,key2,key3
```

When enabled, every API request must include an `Authorization: Bearer <key>`
header. Unauthorized requests receive `401 Unauthorized`.

## TLS Enforcement

Setting `MNEMO_REQUIRE_TLS=true` causes Mnemo to reject webhook registration
for non-HTTPS target URLs. This ensures all outbound event delivery uses
encrypted transport. Inbound TLS termination is delegated to the operator's
reverse proxy or load balancer (nginx, Caddy, cloud ALB, etc.).

## Audit Trail

All governance actions (policy changes, violations, overrides) and webhook
deliveries are recorded in Redis-backed audit logs. The unified audit export
is available at:

```
GET /api/v1/audit/export?from=<ISO-8601>&to=<ISO-8601>
```

### Tamper Evidence

When `MNEMO_AUDIT_SIGNING_SECRET` is set, audit export responses include an
`x-mnemo-audit-signature` header with an HMAC-SHA256 signature:

```
x-mnemo-audit-signature: t=1709000000,v1=<hex_digest>
```

The signed payload is `<timestamp>.<response_body>`, allowing independent
verification that the export has not been modified.

## Data Governance

Per-user governance policies control:

- **Domain allowlists** — restrict which knowledge domains a user's memories
  can cover
- **Retention policies** — configurable per episode type (conversation, fact,
  summary)
- **Access scoping** — users can only access their own memory namespace

Policy changes are audit-logged with before/after snapshots.

## Incident Management

The operator dashboard and API surface incident detection for:

- Dead-letter queue spikes (failed webhook deliveries)
- Circuit breaker trips (external service failures)
- Policy violations (attempted writes outside allowed domains)

Incidents are queryable via `GET /api/v1/ops/incidents` and visible in the
dashboard's incident panel.

## Deployment Hardening Checklist

For production deployments targeting SOC 2 compliance:

1. Set `MNEMO_AUTH_ENABLED=true` and configure strong API keys
2. Set `MNEMO_REQUIRE_TLS=true`
3. Set `MNEMO_AUDIT_SIGNING_SECRET` to a random 32+ character secret
4. Terminate TLS at your reverse proxy (nginx, Caddy, cloud ALB)
5. Enable Redis persistence (`appendonly yes`) for audit durability
6. Configure Qdrant backups for vector data recovery
7. Restrict network access to Redis and Qdrant (not publicly reachable)
8. Monitor `GET /api/v1/ops/incidents` or subscribe to webhook events
9. Periodically export and archive audit logs for retention compliance
10. Review `docs/SECURITY_CONTROLS.md` with your auditor

## Shared Responsibility

| Responsibility | Mnemo | Operator |
|---|---|---|
| API authentication | Implements bearer auth | Configures keys, rotates regularly |
| Encryption in transit | Enforces HTTPS for webhooks | Terminates TLS for inbound traffic |
| Encryption at rest | N/A (delegated) | Configures Redis/Qdrant encryption |
| Audit log integrity | HMAC signing | Stores signing secret securely |
| Network isolation | Binds to configurable host/port | Firewall rules, VPC configuration |
| Backup/recovery | Structured data in Redis/Qdrant | Backup schedules, restore testing |
| Monitoring | Incident API + dashboard | Alerting, on-call response |
