# PRD: Red-Team Deferred Findings (P0)

**Version**: 1.1
**Status**: Resolved
**Created**: 2026-03-13
**Resolved**: 2026-03-14
**Relates to**: v0.7.0 red-team audit (30 findings total, 15 fixed in `13708c0`, 12 deferred → all 12 now resolved)

## Context

The v0.7.0 red-team audit surfaced 30 findings across 4 audit surfaces: OpenAPI, Helm chart, OpenTelemetry, and BYOK encryption. 15 were fixed immediately (all CRITICAL, all HIGH, most MEDIUM). 12 were deferred as "acceptable risk." This PRD promotes all 12 to P0 and defines the fix, test, and acceptance criteria for each.

All 12 findings are documented in the README.md Honesty Notes section. As each is resolved, the corresponding honesty note must be updated or removed.

## Execution Order (easiest first)

Items are ordered by estimated complexity (LOC changed, blast radius, test surface). Each follows the cycle: **code → test → falsify → update README → commit → push**.

---

### RT-01: Remove dead `[graph]` config section

**Severity**: LOW (misleading, not insecure)
**Effort**: XS (< 10 LOC)

**Current state**: `config/default.toml` lines 75–83 define a `[graph]` section with `community_detection`, `community_min_size`, `summarization`, `max_summary_tokens`. No struct in `config.rs` consumes these fields. Serde silently ignores them.

**Fix**:
1. Remove the `[graph]` section from `config/default.toml`.
2. Add `#[serde(deny_unknown_fields)]` to `MnemoConfig` so future dead sections cause a parse error (evaluate feasibility — may conflict with env-var override patterns).

**Test**: `cargo test -p mnemo-server` passes. Verify server starts with the updated config.

**Acceptance**: The `[graph]` section is gone. If `deny_unknown_fields` is feasible, a test proves that adding an unknown TOML section causes a startup error.

---

### RT-02: Remove dead `[retention]` config section

**Severity**: LOW (misleading, not insecure)
**Effort**: XS (< 10 LOC)

**Current state**: `config/default.toml` lines 112–116 define `[retention]` with `default_ttl`, `episode_ttl`, `session_ttl`. These are never read. Actual retention is per-user via `UserPolicyRecord` with hardcoded 3650-day defaults in `routes.rs:221–234`.

**Fix**:
1. Remove the `[retention]` section from `config/default.toml`.
2. Add a comment in `config/default.toml` explaining that retention is per-user via the Policy API (not global config).

**Test**: `cargo test -p mnemo-server` passes. Server starts cleanly.

**Acceptance**: The `[retention]` section is gone. A comment directs operators to the Policy API.

---

### RT-03: Helm subchart seccompProfile

**Severity**: MEDIUM (defense-in-depth gap)
**Effort**: S (< 20 LOC)

**Current state**: `values.yaml` sets `seccompProfile: RuntimeDefault` for the Mnemo pod but not for Redis or Qdrant subchart pods.

**Fix**:
1. Add `podSecurityContext.seccompProfile.type: RuntimeDefault` to the `redis` section in `values.yaml`.
2. Add `podSecurityContext.seccompProfile.type: RuntimeDefault` to the `qdrant` section in `values.yaml` (verify subchart supports this field).

**Test**: `helm lint` passes. `helm template` renders the seccompProfile in Redis and Qdrant pod specs.

**Acceptance**: `helm template` output shows `seccompProfile: RuntimeDefault` on all three workloads.

---

### RT-04: Helm Ingress TLS defaults

**Severity**: MEDIUM (configuration gap)
**Effort**: S (< 30 LOC)

**Current state**: `ingress.yaml` template supports TLS but `values.yaml` defaults to `tls: []`. No validation warns when ingress is enabled without TLS.

**Fix**:
1. Add a `NOTES.txt` warning when `ingress.enabled=true` and `ingress.tls` is empty.
2. Add commented-out cert-manager annotation example with `cluster-issuer` in `values.yaml`.
3. Add a commented-out TLS example block in `values.yaml` (already partially there — make it more explicit).

**Test**: `helm lint` passes. `helm template --set ingress.enabled=true` renders the NOTES.txt warning.

**Acceptance**: Operators who enable ingress without TLS see a clear warning in `helm install` output.

---

### RT-05: Helm Qdrant auth

**Severity**: MEDIUM (in-cluster exposure)
**Effort**: S (< 30 LOC)

**Current state**: Qdrant subchart section has no auth configuration. Qdrant is accessible unauthenticated inside the cluster.

**Fix**:
1. Add `apiKey` and `readOnlyApiKey` configuration to the `qdrant` section in `values.yaml` (check if the Qdrant Helm subchart supports `config.service.api_key`).
2. If subchart supports it, wire the Qdrant API key into the Mnemo configmap/env so the server authenticates to Qdrant.
3. If subchart does NOT support auth natively, document the gap in NOTES.txt and recommend NetworkPolicy as mitigation.

**Test**: `helm lint` and `helm template` pass. NOTES.txt renders auth guidance.

**Acceptance**: Either Qdrant auth is configurable via values, or the gap is explicitly documented with mitigation guidance.

---

### RT-06: Helm NetworkPolicy

**Severity**: MEDIUM (defense-in-depth gap)
**Effort**: M (new template, ~60 LOC)

**Current state**: No `networkpolicy.yaml` template exists. All pod-to-pod traffic in the namespace is unrestricted.

**Fix**:
1. Create `deploy/kubernetes/mnemo/templates/networkpolicy.yaml`.
2. Default: allow Mnemo → Redis (port 6379), Mnemo → Qdrant (ports 6333/6334), deny all other ingress to Redis/Qdrant.
3. Allow external ingress to Mnemo on port 8080 (and 6831 for gRPC if configured).
4. Make NetworkPolicy toggleable via `networkPolicy.enabled: false` in values.yaml (off by default to avoid breaking clusters without a CNI that supports NetworkPolicy).

**Test**: `helm lint` passes. `helm template --set networkPolicy.enabled=true` renders valid NetworkPolicy resources.

**Acceptance**: `helm template` output shows 3 NetworkPolicy resources (mnemo ingress, redis ingress, qdrant ingress) when enabled.

---

### RT-07: CORS environment-based configuration

**Severity**: HIGH (security for production deployments)
**Effort**: M (~50 LOC + tests)

**Current state**: `main.rs:522` uses `CorsLayer::permissive()` unconditionally. No config option exists.

**Fix**:
1. Add `cors_allowed_origins` field to `ServerSection` in `config.rs` (default: `["*"]` for backward compatibility).
2. Add `MNEMO_CORS_ALLOWED_ORIGINS` env var override.
3. In `main.rs`, build `CorsLayer` conditionally:
   - If `["*"]`: `CorsLayer::permissive()` (current behavior).
   - Otherwise: `CorsLayer::new().allow_origin(origins).allow_methods(...)`.
4. Add to `config/default.toml` with comment.

**Test**:
1. Unit test: config parses `cors_allowed_origins`.
2. Integration test: server with restricted origins rejects cross-origin request from unlisted origin.
3. Integration test: server with `["*"]` allows all (backward compat).

**Acceptance**: `MNEMO_CORS_ALLOWED_ORIGINS=https://app.example.com` restricts CORS. Default behavior unchanged.

---

### RT-08: Auth-exempt routes CallerContext refactor

**Severity**: HIGH (architectural correctness)
**Effort**: M (~80 LOC + tests)

**Current state**: `auth.rs:139` injects `CallerContext::admin_bootstrap()` for exempt paths. `routes.rs` handlers use `.unwrap_or_else(CallerContext::admin_bootstrap)`. This means health, swagger, and dashboard routes carry admin privileges.

**Fix**:
1. Add `CallerContext::anonymous()` variant with a new role `ApiKeyRole::Anonymous` (or use `Option<CallerContext>` = `None` for exempt routes).
2. Auth middleware: inject `CallerContext::anonymous()` for exempt paths instead of `admin_bootstrap()`.
3. Auth-disabled mode: continue injecting `admin_bootstrap()` (operators who disable auth expect full access).
4. Update `caller_from_extension()` in `routes.rs` to return `CallerContext::anonymous()` as fallback.
5. Audit all 40+ handlers: handlers on exempt routes (health, swagger, dashboard) don't check roles. Handlers on protected routes already check `caller.role` — verify they reject `Anonymous`.

**Test**:
1. Unit test: `CallerContext::anonymous()` has `Anonymous` role.
2. Integration test: health/swagger/dashboard work without auth.
3. Integration test: protected endpoint rejects `Anonymous` context.

**Acceptance**: Exempt routes no longer carry `Admin` privileges. No behavioral change for authenticated or auth-disabled flows.

---

### RT-09: OTLP TLS and authentication

**Severity**: MEDIUM (production hardening)
**Effort**: M (~60 LOC + tests)

**Current state**: `telemetry.rs` calls `with_endpoint()` only. No TLS, no auth headers.

**Fix**:
1. Add to `ObservabilitySection` in `config.rs`:
   - `otel_tls_enabled: bool` (default: `false`)
   - `otel_tls_cert_path: Option<String>` (client cert for mTLS)
   - `otel_tls_key_path: Option<String>`
   - `otel_tls_ca_path: Option<String>` (CA cert for verifying collector)
   - `otel_auth_header: Option<String>` (bearer token or API key)
2. In `telemetry.rs`, conditionally configure tonic with TLS (`tonic::transport::ClientTlsConfig`) and metadata (auth header).
3. Add env var overrides: `MNEMO_OTEL_TLS_ENABLED`, `MNEMO_OTEL_TLS_CA_PATH`, `MNEMO_OTEL_AUTH_HEADER`.

**Test**:
1. Unit test: config parses all new fields.
2. Server starts with `otel_enabled=true, otel_tls_enabled=false` (current behavior preserved).
3. Config validation: if `otel_tls_enabled=true` but `otel_tls_ca_path` is missing, log a warning.

**Acceptance**: Operators can configure TLS and auth headers for OTLP export. Default behavior unchanged.

---

### RT-10: BYOK key rotation mechanism

**Severity**: HIGH (compliance requirement)
**Effort**: L (~150 LOC + tests)

**Current state**: `EnvelopeEncryptor` holds a single KEK and `key_id`. Encrypted envelopes store `_key_id` but decryption ignores it — always uses the single loaded key.

**Fix**:
1. Change `EnvelopeEncryptor` to hold a `HashMap<String, [u8; 32]>` of key_id → KEK, plus an `active_key_id: String`.
2. Encryption always uses `active_key_id`.
3. Decryption reads `_key_id` from envelope and looks up the correct KEK. If the key is unknown, return a clear error ("unknown key_id '...', available keys: [...]").
4. Config: support `MNEMO_ENCRYPTION_MASTER_KEYS` as a comma-separated list of `key_id:base64_key` pairs, or keep the single-key config and add `MNEMO_ENCRYPTION_RETIRED_KEYS` for old keys.
5. Add a `/api/v1/ops/encryption/status` endpoint showing active key_id and count of values encrypted with each key_id.
6. Document the rotation workflow: set new active key → restart → old key still decrypts → optionally re-encrypt via future admin endpoint.

**Test**:
1. Unit test: encrypt with key A, rotate to key B, decrypt with key A still works.
2. Unit test: encrypt with key B after rotation uses new key_id.
3. Unit test: unknown key_id returns descriptive error.
4. Integration test: server starts with multiple keys configured.

**Acceptance**: Key rotation is possible without data loss. Old keys decrypt existing data. New data uses the active key.

---

### RT-11: Internal types leaked in OpenAPI schemas

**Severity**: LOW (API hygiene)
**Effort**: L (~100 LOC refactor)

**Current state**: `openapi.rs` registers ~90 schemas including internal types like `ImportJobRecord`, `UserPolicyRecord`, `GovernanceAuditRecord`, `MemoryWebhookSubscription` (which has a `#[serde(skip)]` signing secret field).

**Fix**:
1. Audit the 90 registered schemas. Classify each as "public API" or "internal."
2. Remove internal types from the `schemas()` block in `openapi.rs`.
3. For types that are returned by API endpoints but expose internal fields, create DTO wrappers (e.g., `WebhookSubscriptionResponse` that omits `signing_secret`).
4. This is partially blocked by RT-12 (OpenAPI paths) — once paths are annotated, utoipa will auto-discover referenced schemas. Consider doing RT-12 first and then pruning.

**Test**: OpenAPI spec validates. No internal-only types appear in the schema.

**Acceptance**: The OpenAPI spec contains only types that correspond to API request/response shapes.

---

### RT-12: OpenAPI path annotations

**Severity**: MEDIUM (DevEx, SDK codegen)
**Effort**: XL (~500+ LOC, 131 endpoints)

**Current state**: Zero `#[utoipa::path]` annotations. The spec has schemas and tags but no endpoint documentation. Swagger UI shows models only.

**Fix**:
1. Add `#[utoipa::path]` annotations to all 131 REST handlers in `routes.rs`.
2. Each annotation needs: HTTP method, path, tag, summary, request body (if any), response types, security requirement.
3. Register paths in `openapi.rs` `paths()` block (or use `utoipa-axum` auto-discovery if available).
4. Group by the 20 existing tags.

**Test**:
1. `cargo test -p mnemo-server` passes (utoipa compile-time validation).
2. OpenAPI spec has 131 path entries.
3. Swagger UI shows all endpoints grouped by tag.

**Acceptance**: Every REST endpoint is documented in the OpenAPI spec with method, path, request/response schemas, and tag. Swagger UI is fully functional for API exploration.

**Note**: This is the largest item. Consider breaking into sub-batches by tag (health, users, sessions, episodes, memory, etc.) across multiple commits.

---

## Summary

All 12 findings have been resolved. Each followed the cycle: code → test → falsify → update README → commit → push.

| ID | Finding | Severity | Effort | Status | Commit |
|----|---------|----------|--------|--------|--------|
| RT-01 | Dead `[graph]` config | LOW | XS | ✅ Resolved | `f926915` |
| RT-02 | Dead `[retention]` config | LOW | XS | ✅ Resolved | `f926915` |
| RT-03 | Helm subchart seccomp | MEDIUM | S | ✅ Resolved | `ffdddc7` |
| RT-04 | Helm Ingress TLS warning | MEDIUM | S | ✅ Resolved | `3e50dbc` |
| RT-05 | Helm Qdrant auth | MEDIUM | S | ✅ Resolved | `1f2da6b` |
| RT-06 | Helm NetworkPolicy | MEDIUM | M | ✅ Resolved | `bf720db` |
| RT-07 | CORS env-based config | HIGH | M | ✅ Resolved | `dd49021` |
| RT-08 | Auth-exempt CallerContext | HIGH | M | ✅ Resolved | `91b5fe7` |
| RT-09 | OTLP TLS + auth | MEDIUM | M | ✅ Resolved | `e7190c8` |
| RT-10 | BYOK key rotation | HIGH | L | ✅ Resolved | `85a41c8` |
| RT-11 | Internal types in OpenAPI | LOW | L | ✅ Resolved | `daa071f` |
| RT-12 | OpenAPI path annotations | MEDIUM | XL | ✅ Resolved | `59d084e` |

### Post-Resolution Fixes

| Fix | Commit |
|-----|--------|
| OpenAPI stack overflow (29 `Value` fields + recursive `GuardrailCondition`) | `b3265a8` |
| Integration test harness CallerContext injection (244/244 pass) | `0899904` |
