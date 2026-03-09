# Mnemo Security Controls

This document maps Mnemo's security capabilities to SOC 2 Trust Services Criteria.
It is intended for use by compliance teams, auditors, and operators preparing
for SOC 2 Type II readiness assessments.

## CC1 — Control Environment

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC1.1 — Security commitment | This document; `docs/COMPLIANCE.md` | Implemented |
| CC1.2 — Board/management oversight | Operator dashboard at `/_/` with live incident triage | Implemented |
| CC1.3 — Organizational structure | API key scoping, per-user governance policies | Implemented |

## CC2 — Communication and Information

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC2.1 — Internal communication | Governance audit trail (`GET /api/v1/policies/:user/audit`) | Implemented |
| CC2.2 — External communication | Webhook event delivery with audit trail | Implemented |
| CC2.3 — Relevant information | `GET /api/v1/audit/export` — unified audit log export (SOC 2 ready) | Implemented |

## CC3 — Risk Assessment

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC3.1 — Objective specification | P0 roadmap (`docs/P0_ROADMAP.md`), architecture docs | Implemented |
| CC3.2 — Risk identification | Operator dashboard incident panel, policy violation tracking | Implemented |
| CC3.3 — Fraud risk | HMAC-signed webhook delivery, HMAC-signed audit export | Implemented |

## CC5 — Control Activities

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC5.1 — Logical access | `MNEMO_AUTH_ENABLED` + `MNEMO_AUTH_API_KEYS` bearer token auth | Implemented |
| CC5.2 — Technology infrastructure | Docker Compose deployment, health checks, resource limits | Implemented |
| CC5.3 — Security awareness | API key auth, TLS enforcement (`MNEMO_REQUIRE_TLS`) | Implemented |

## CC6 — Logical and Physical Access Controls

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC6.1 — Logical access security | API key bearer auth on all endpoints | Implemented |
| CC6.2 — Provisioning/deprovisioning | Comma-separated `MNEMO_AUTH_API_KEYS` env var | Implemented |
| CC6.3 — Role-based access | Per-user policy records with domain allowlists | Implemented |
| CC6.6 — Encryption in transit | `MNEMO_REQUIRE_TLS=true` rejects non-https targets | Implemented |
| CC6.7 — Encryption at rest | Redis persistence (RDB), Qdrant storage — operator responsibility | Delegated |

## CC7 — System Operations

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC7.1 — Detection of anomalies | Dead-letter spike detection, circuit breaker incidents | Implemented |
| CC7.2 — Incident monitoring | `GET /api/v1/ops/incidents`, operator dashboard | Implemented |
| CC7.3 — Evaluation of security events | Governance audit trail with violation tracking | Implemented |
| CC7.4 — Incident response | Dead-letter recovery, webhook retry/replay API | Implemented |

## CC8 — Change Management

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC8.1 — Infrastructure/software changes | CI quality gates, Docker image versioning, GHCR publish | Implemented |

## CC9 — Risk Mitigation

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| CC9.1 — Risk acceptance criteria | Configurable retention policies per user per episode type | Implemented |
| CC9.2 — Vendor management | External LLM provider abstraction with rate-limit handling | Implemented |

## Availability (A1)

| Control | Mnemo Implementation | Status |
|---------|---------------------|--------|
| A1.1 — Availability commitment | Docker health checks, circuit breaker for webhooks | Implemented |
| A1.2 — Environmental protections | Qdrant ulimits, Redis save policy, restart: unless-stopped | Implemented |

## Audit Log Format

The unified audit export at `GET /api/v1/audit/export` returns:

```json
{
  "ok": true,
  "from": "ISO-8601",
  "to": "ISO-8601",
  "total": 42,
  "records": [
    {
      "audit_type": "governance|webhook",
      "id": "UUID",
      "user_id": "UUID",
      "action": "policy_update|policy_violation|...",
      "at": "ISO-8601",
      "request_id": "UUID or null",
      "details": {},
      "webhook_id": "UUID or null"
    }
  ]
}
```

When `MNEMO_AUDIT_SIGNING_SECRET` is set, the response includes an
`x-mnemo-audit-signature` header with HMAC-SHA256 tamper evidence:
`t=<unix_timestamp>,v1=<hex_digest>`.

## Environment Variables for Compliance

| Variable | Description |
|----------|-------------|
| `MNEMO_AUTH_ENABLED` | Enable bearer token authentication |
| `MNEMO_AUTH_API_KEYS` | Comma-separated list of valid API keys |
| `MNEMO_REQUIRE_TLS` | Reject non-HTTPS webhook targets |
| `MNEMO_AUDIT_SIGNING_SECRET` | HMAC secret for audit export signatures |
