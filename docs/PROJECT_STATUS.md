# Project Status

Current version: **v0.9.0**

## Completed Phases

### Phase 1.5 — Production Hardening

- Compilation + integration coverage
- Auth middleware
- Full-text + hybrid retrieval
- Memory API + falsification CI gate

### Phase 2 — Temporal Productization

- M1 Thread HEAD completion
- M2 Temporal retrieval v2 diagnostics
- M3 Metadata index layer
- M4 Competitive publication v1
- M5 Agent Identity Substrate P0

See [PHASE_2_PRD.md](PHASE_2_PRD.md) for milestones.

### Phase 2 Deployment — Cloud IaC (10/10 targets falsified)

| Target | Status |
|--------|--------|
| T1 Docker production compose | All 5 gates passed |
| T2 Bare Metal systemd + nginx | All 5 gates passed |
| T3 AWS CloudFormation | All 5 gates passed |
| T4 GCP Terraform | All 5 gates passed |
| T5 DigitalOcean Terraform | All 5 gates passed |
| T6 Render | All 5 gates passed |
| T7 Railway | All 5 gates passed |
| T8 Vultr Terraform | All 5 gates passed |
| T9 Northflank | All 5 gates passed |
| T10 Linode | All 5 gates passed |

See [PRD_DEPLOY.md](PRD_DEPLOY.md) and [DEPLOYMENT_STATUS.md](DEPLOYMENT_STATUS.md).

### QA/QC Falsification (3 phases complete)

- Phase 1: 59 tests — graph engine, LLM providers, Qdrant store, async SDK, webhook persistence
- Phase 2: 44 tests — config parsing, session messages, raw vectors, auth integration
- Phase 3: 6 tests — rate limiting, circuit breaker, RRF reranker
- **Total: ~293 tests across the project**
- 3 bugs fixed, 3 new scripts added

See [QA_QC_FALSIFICATION_PRD.md](QA_QC_FALSIFICATION_PRD.md).

## Current Releases

### v0.9.0 — gRPC Parity, Temporal Accuracy, Homeoadaptive LoRA

- gRPC expanded to 6 services / 30 RPCs with full data-plane parity
- gRPC red-team hardening: role enforcement, ownership checks, input caps
- Temporal accuracy gate 96.8% (gate: 95%)
- Homeoadaptive LoRA (Spec 07): explicit feedback, agent-view stats
- `as_of` hard-filter consistency (Spec 08)
- Dedicated gRPC port option (`MNEMO_GRPC_PORT`)

### v0.7.0 — DevEx, Kubernetes & Enterprise Hardening

- OpenAPI 3.1 spec + Swagger UI
- Production Helm chart with Redis/Qdrant subcharts
- OpenTelemetry OTLP trace export
- BYOK AES-256-GCM envelope encryption
- Red-team audit (30 findings resolved)

### v0.6.0 — Enterprise Access Control

- Scoped API keys with RBAC
- Data classification labels
- Policy-scoped memory views
- Memory guardrails engine
- Agent identity Phase B
- Multi-agent shared memory regions with ACLs
- gRPC API (initial)

## In Progress

### Phase 3 — Operator UX & Control Plane

| Feature | Status |
|---------|--------|
| Governance policy APIs | Complete |
| Read/write retention enforcement | Complete |
| Operator hero-lane backend | Complete |
| Webhook ops endpoints | Complete |
| Falsification suite (78 tests) | Complete |
| Raw Vector API | Complete |
| Session Messages API | Complete |
| AnythingLLM integration | Complete |
| Python SDK full rebuild | Complete |
| LangChain adapter | Complete |
| LlamaIndex adapter | Complete |
| SDK falsification (83/83) | Complete |
| Operator-facing frontend | In progress |
| p95 latency evidence capture | In progress |

See [OPERATOR_UX_PRD.md](OPERATOR_UX_PRD.md) and [OPERATOR_DASHBOARD_PRD.md](OPERATOR_DASHBOARD_PRD.md).

## Quality Gates

All gates run on every PR:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins
cargo test -p mnemo-storage --test storage -- --test-threads=1
cargo test -p mnemo-ingest --test ingest -- --test-threads=1
cargo test -p mnemo-server --test memory_api -- --test-threads=1
bash tests/e2e_smoke.sh http://localhost:8080
bash tests/operator_p0_drills.sh
```

Reference CI: `.github/workflows/quality-gates.yml`

Nightly soak: `.github/workflows/nightly-soak.yml`

## Releases & Packages

- Tags `v*.*.*` trigger automated GitHub Releases
- Docker images: `ghcr.io/anjaustin/mnemo/mnemo-server`
- Release artifacts: Linux amd64 binary, tarball, SHA256SUMS

```bash
# Get latest release
gh release download --repo anjaustin/mnemo --pattern 'mnemo-server-*'

# Pull Docker image
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:latest
```

## Roadmap

See:
- [P0_ROADMAP.md](P0_ROADMAP.md) — completed capability gaps
- [DOMAIN_READINESS_MATRIX.md](DOMAIN_READINESS_MATRIX.md) — domain-by-domain readiness
- [FACE_MELTER_FEATURES.md](FACE_MELTER_FEATURES.md) — differentiation features (10/12 shipped)
