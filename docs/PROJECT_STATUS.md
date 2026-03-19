# Project Status

Current version: **v0.9.1**

---

## Latest: Security Red-Team Audit Complete

**Date**: March 2026  
**Scope**: Comprehensive security audit of all Mnemo subsystems

### Audit Coverage
- Auth & API Keys
- Encryption (BYOK, at-rest)
- gRPC API
- Webhooks
- Memory/Retrieval endpoints
- Graph API
- Import/Export
- Agent Identity
- Guardrails/Policies
- SDKs

### Remediations by Severity

| Severity | Count | Status |
|----------|-------|--------|
| P0 (Critical) | 4 | ✅ Complete |
| P1 (High) | 5 | ✅ Complete |
| P2 (Medium) | 7 | ✅ Complete |
| P3 (Low) | 7 | ✅ Complete |
| **Total** | **23** | **✅ All Complete** |

### Key Fixes
- **P0-1**: User-scoped access control on all REST + gRPC endpoints
- **P0-2**: CSPRNG API key generation (moved from UUID to OsRng)
- **P0-3**: SSRF protection with IP validation and redirect blocking
- **P0-4**: gRPC entity/edge ownership verification
- **P1-1**: Timestamp validation on imports (±5min future, ≤10 years past)
- **P1-4**: ReDoS protection with regex size limits
- **P1-5**: DNS rebinding protection at webhook delivery
- **P2-1**: Constant-time API key comparison (subtle crate)
- **P3-5**: Nil UUID collision fix (distinct sentinel UUIDs)

See [SECURITY_REMEDIATION_PLAN.md](SECURITY_REMEDIATION_PLAN.md) for full details.

---

## Roadmap: Next Step-Changes

### Multi-Tenancy (v0.10.0) — PRD Complete
Organization → Workspace → User hierarchy with shared memory pools, RBAC, and scoped API keys. See [MULTI_TENANCY_PRD.md](MULTI_TENANCY_PRD.md).

### Multi-Modal Memory (v0.11.0) — In Progress
Image, audio, and document memory support. See [MULTI_MODAL_PRD.md](MULTI_MODAL_PRD.md).

**Phase 1 Foundation — COMPLETE ✅**
- `Modality` enum and `Attachment` model (`crates/mnemo-core/src/models/attachment.rs`)
- `BlobStore` trait (`crates/mnemo-core/src/traits/blob.rs`)
- `LocalBlobStore` implementation (`crates/mnemo-storage/src/local_blob_store.rs`)
- `S3BlobStore` implementation (`crates/mnemo-storage/src/s3_blob_store.rs`)
- `AttachmentStore` trait and Redis implementation
- `BlobSection` configuration (`crates/mnemo-server/src/config.rs`)
- Episode model extended with `modality`, `attachment_ids`, `parent_document_id`
- `BlobHandle` type-erased wrapper in AppState (`crates/mnemo-server/src/state.rs`)
- REST endpoints for attachments:
  - `POST /api/v1/episodes/{episode_id}/attachments` - Upload attachment
  - `GET /api/v1/episodes/{episode_id}/attachments` - List attachments
  - `GET /api/v1/attachments/{attachment_id}` - Get metadata
  - `GET /api/v1/attachments/{attachment_id}/download` - Download content
  - `DELETE /api/v1/attachments/{attachment_id}` - Delete attachment
  - `POST /api/v1/attachments/{attachment_id}/presign` - Get presigned URL

**Phase 1 Security Fixes:**
- P1-1: Authorization checks on all attachment endpoints (CallerContext)
- P1-2: Path traversal protection in LocalBlobStore (canonicalization)
- P1-3: Symlink traversal protection in list() function
- P2-2: Presigned URL expiration capped at 24 hours
- P2-5: Content-Disposition header injection prevention
- P3-3: Async file existence checks (tokio::fs::try_exists)

**Remaining Phases:**
- Phase 2: VisionProvider + image upload endpoints
- Phase 3: TranscriptionProvider + audio upload endpoints
- Phase 4: Document parsing and chunking
- Phase 5: Multi-modal retrieval and SDK extensions

---

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

## Existing Capabilities (Verified)

The following capabilities were verified as fully implemented during the security audit:

### Memory Consolidation & Forgetting
- Sleep-time consolidation pipeline with idle-triggered digest generation
- EWC++ (Elastic Weight Consolidation) for experience decay resistance
- Exponential decay curves with configurable half-life (30 days experiences, 90 days edges)
- Tiered embedding compression (f32 → f16 → int8 → binary)
- Contradiction detection (LLM-based + GNN ContraGat classifier)
- Automatic fact supersession for conflicting facts
- Stale fact detection and revalidation endpoints

### Distributed/Federated Support
- CRDT sync module (1551 lines) with HLC, vector clocks, GCounter, LWWRegister, ORSet
- Multi-node sync via `MNEMO_SYNC_ENABLED` configuration
- Qdrant distributed mode ready (documented scaling path)
- Stateless architecture for horizontal scaling

### Active Memory Framework
- 13 webhook event types for memory lifecycle events
- Self-healing clarification system (detect → question → answer → heal)
- Sleep-time compute with proactive re-ranking
- Evolving user narrative summaries
- Goal-conditioned retrieval
- Memory digests with topic extraction

---

## Roadmap

See:
- [P0_ROADMAP.md](P0_ROADMAP.md) — completed capability gaps
- [MULTI_TENANCY_PRD.md](MULTI_TENANCY_PRD.md) — next major feature (v0.10.0)
- [MULTI_MODAL_PRD.md](MULTI_MODAL_PRD.md) — multi-modal memory (v0.11.0)
- [DOMAIN_READINESS_MATRIX.md](DOMAIN_READINESS_MATRIX.md) — domain-by-domain readiness
- [FACE_MELTER_FEATURES.md](FACE_MELTER_FEATURES.md) — differentiation features (10/12 shipped)
