# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Mnemo, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: **security@mnemo.dev**

Include:

- A description of the vulnerability
- Steps to reproduce (or a proof-of-concept)
- The affected version(s)
- Any potential impact assessment

## Response Timeline

| Stage | Target |
|-------|--------|
| Acknowledgment | Within 48 hours |
| Initial assessment | Within 5 business days |
| Fix or mitigation plan | Within 15 business days |
| Public disclosure | After fix is released, coordinated with reporter |

We will credit reporters in release notes unless they prefer to remain anonymous.

## Scope

The following are in scope for security reports:

- The Mnemo server (`mnemo-server` binary)
- The Python SDK (`mnemo-client`)
- The TypeScript SDK (`mnemo-client`)
- Official Docker images (`ghcr.io/anjaustin/mnemo/mnemo-server`)
- Infrastructure-as-Code templates in `deploy/`
- Webhook HMAC signature verification
- API authentication and authorization
- Data isolation between users/tenants

The following are **out of scope**:

- Third-party dependencies (report upstream; mention in your report if relevant)
- Attacks requiring physical access to the host
- Social engineering
- Denial-of-service attacks against self-hosted instances (resource limits are the operator's responsibility)

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.3.x (latest) | Yes |
| 0.2.x | Security fixes only |
| < 0.2.0 | No |

## Security Controls

Mnemo implements the following security controls:

- **API key authentication** with constant-time comparison
- **HMAC-SHA256 webhook signatures** for outbound event verification
- **TLS enforcement** for webhook delivery endpoints (configurable)
- **Webhook domain allowlists** for restricting outbound delivery targets
- **Per-user data isolation** at the storage layer (Redis key prefixes, Qdrant collection prefixes)
- **Governance audit trail** for policy changes and destructive operations
- **SOC 2 Trust Service Criteria mapping** (see `docs/SECURITY_CONTROLS.md`)

For the full compliance posture, see [`docs/COMPLIANCE.md`](docs/COMPLIANCE.md).
