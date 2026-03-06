# Testing Guide

## Test Count Summary

| Location | Tests | Type |
|----------|-------|------|
| `crates/mnemo-server/tests/memory_api.rs` | 80 | Integration (requires Redis + Qdrant) |
| `crates/mnemo-server/src/config.rs` | 24 | Unit (inline `#[cfg(test)]`) |
| `crates/mnemo-server/src/middleware/auth.rs` | 7 | Unit (inline) |
| `crates/mnemo-graph/src/lib.rs` | 10 | Unit (inline) |
| `crates/mnemo-llm/src/openai_compat.rs` | 17 | Unit (wiremock) |
| `crates/mnemo-llm/src/anthropic.rs` | 7 | Unit (wiremock) |
| `crates/mnemo-retrieval/src/lib.rs` | 11 | Unit (6 existing + 5 RRF) |
| `crates/mnemo-storage/tests/qdrant.rs` | 6 | Integration (requires Qdrant) |
| `crates/mnemo-storage/tests/storage.rs` | ~7 | Integration (requires Redis) |
| `crates/mnemo-ingest/tests/ingest.rs` | ~4 | Integration (requires Redis + Qdrant) |
| `sdk/python/tests/test_sdk.py` | 65 assertions | Python (requires live server) |
| `sdk/python/tests/test_async_client.py` | 18 | Python (aioresponses, no server needed) |
| `tests/credential_scan.sh` | 5 gates | Bash script |
| `tests/deploy_artifact_validation.sh` | 36 gates | Bash script |
| `tests/docker_build_test.sh` | 3 gates | Bash script |
| `tests/dashboard_smoke.sh` | 12 gates | Bash script (requires running server) |
| `tests/phase_b_screenshots.py` | 8 screenshots | Playwright (requires running server) |
| Phase B falsification | 35 gates | Playwright (manual, requires running server) |
| **Total** | **~280+** | |

This project has several practical testing layers.

## 1) Workspace tests

```bash
cargo test --workspace
```

## 2) Deterministic end-to-end smoke script

Assumes the server is already running on `http://localhost:8080`.

```bash
bash tests/e2e_smoke.sh http://localhost:8080
```

This smoke test is designed to pass without external LLM credentials.

## 2.5) Operator P0 workflow drills

Runs targeted deterministic integration tests that mirror the three operator milestone loops.

```bash
bash tests/operator_p0_drills.sh
```

This drill script exercises the three operator milestone loops:

- dead-letter recovery (fail -> dead-letter -> retry -> delivered)
- why-changed RCA (`time_travel/summary` + trace lookup evidence)
- governance misconfig detection (`policy_violation_*` capture + time-window query)

## 3) Full end-to-end script (LLM-dependent)

Assumes the server is already running on `http://localhost:8080`.

```bash
./tests/e2e.sh
```

## 4) Memory API falsification suite

This is the high-value regression suite for the new memory surface.

```bash
cargo test -p mnemo-server --test memory_api -- --test-threads=1
```

It verifies:

- input validation (`user`, `text`, `query`)
- user/session auto-resolution behavior
- role persistence on remembered episodes
- unknown user handling
- immediate recall fallback (non-empty context right after remember)
- head mode behavior (auto selection, explicit session override, empty-head safety)
- temporal intent ranking shifts (`current` vs `historical`)
- metadata prefilter planner behavior (`enabled`, `scan_limit`, `relax_if_empty`)
- identity contamination guardrails (`identity_core` write blocking)
- identity versioning/audit/rollback semantics
- promotion gating and approve/reject flows
- async chat-history import pathway (`/api/v1/import/chat-history` + job polling)
- import falsification checks for malformed rows, mixed timestamp quality, and idempotent replay
- scientific retrieval provenance checks (episode citation coverage for current/historical queries)
- memory diff checks (`/api/v1/memory/:user/changes_since`) for timeline windows and head movement
- conflict radar checks (`/api/v1/memory/:user/conflict_radar`) for active contradiction cluster detection
- causal recall chain checks (`/api/v1/memory/:user/causal_recall`) for fact-to-episode lineage output
- time travel trace/summary checks (`/api/v1/memory/:user/time_travel/trace`, `/api/v1/memory/:user/time_travel/summary`) for snapshot diffs, timeline evidence, and lightweight delta counters
- memory contract checks (`support_safe`, `current_strict`, `historical_strict`) for policy-scoped context behavior
- adaptive retrieval policy checks (`balanced`, `precision`, `recall`, `stability`) for effective threshold diagnostics
- memory webhook checks (`/api/v1/memory/webhooks`) for event capture, replay cursors, manual retry flows (including retry response event envelope), delivery telemetry, retry/backoff, dead-letter transitions, stats endpoint, audit rows, and signature correctness
- observability checks (`/metrics`, `x-mnemo-request-id`) for telemetry exposure and request correlation propagation
- governance policy checks (`/api/v1/policies/:user`, `/api/v1/policies/:user/preview`, `/api/v1/policies/:user/violations`) for webhook allowlist enforcement, default contract/retrieval fallback behavior, retention write guards, preview impact estimation, violation-window filtering, and destructive-operation audit trail coverage

## 5) QA/QC Falsification Tests

Added across three phases of the [QA/QC Falsification PRD](QA_QC_FALSIFICATION_PRD.md). These tests verify every major feature, endpoint, and claim in the codebase.

### Config parsing (24 unit tests)

```bash
cargo test -p mnemo-server --lib config -- --test-threads=1
```

Covers: TOML loading, env var overrides, invalid values, missing required config, auth-enabled-without-keys.

### Graph engine (10 unit tests)

```bash
cargo test -p mnemo-graph --lib
```

Covers: BFS traversal (depth, max_nodes), community detection, relevance discount, temporal filtering.

### LLM providers (24 unit tests with wiremock)

```bash
cargo test -p mnemo-llm --lib
```

Covers: OpenAI/Anthropic prompt construction, malformed response handling, rate limit (429), empty content, embedding dimension mismatch.

### Qdrant store (6 integration tests)

```bash
cargo test -p mnemo-storage --test qdrant -- --test-threads=1
```

Requires Qdrant at `http://localhost:6334`. Covers: upsert/search roundtrip, tenant isolation, GDPR delete, TOCTOU race handling.

### Retrieval engine (5 RRF diversity tests)

```bash
cargo test -p mnemo-retrieval --lib
```

Covers: RRF score correctness, overlap boosting, diversity verification, temporal intent resolution.

### Async SDK (18 unit tests)

```bash
cd sdk/python && python -m pytest tests/test_async_client.py -v
```

Covers: AsyncMnemo roundtrip, all 27 methods exist, context manager, error handling. Uses `aioresponses` (no live server needed).

### Credential scan

```bash
bash tests/credential_scan.sh
```

5 gates: `.keys/` gitignored, sensitive patterns gitignored, terraform state not tracked, no API keys in tracked files, `.env.example` has no real values.

### Deploy artifact validation

```bash
bash tests/deploy_artifact_validation.sh
```

36 gates: CloudFormation template validates, Terraform configs validate (4 targets), Render blueprint validates, Railway config validates, Northflank stack validates.

### Docker build test

```bash
bash tests/docker_build_test.sh
```

3 gates: `docker build .` succeeds, image size < 50MB, container starts and responds to `/health`.

### Dashboard smoke test

```bash
bash tests/dashboard_smoke.sh
```

11 gates: `/health`, `/_/` index, `/_/static/style.css`, `/_/static/app.js`, 5 SPA routes, 404 for missing static asset, `GET /api/v1/memory/webhooks`. Requires a running server on `http://localhost:8080` (override with `MNEMO_URL`).

## 6) Importer stress harness (large real-world export)

```bash
python3 eval/import_stress.py --mode dry-run --iterations 2 --base-url http://localhost:8080
python3 eval/import_stress.py --mode import --iterations 1 --base-url http://localhost:8080
```

This harness loads a real ChatGPT export zip, runs async import jobs, and reports per-iteration and aggregate throughput metrics.

### Test infrastructure

By default the falsification suite expects:

- Redis at `redis://localhost:6379`
- Qdrant at `http://localhost:6334`

You can override with:

- `MNEMO_TEST_REDIS_URL`
- `MNEMO_TEST_QDRANT_URL`
- `MNEMO_TEST_QDRANT_PREFIX`

For isolated local test services:

```bash
docker compose -f docker-compose.test.yml up -d
cargo test -p mnemo-storage --test storage -- --test-threads=1
cargo test -p mnemo-ingest --test ingest -- --test-threads=1
cargo test -p mnemo-server --test memory_api -- --test-threads=1
docker compose -f docker-compose.test.yml down
```

If `mnemo-ingest` integration tests fail with `Connection refused` on Redis, set:

```bash
export MNEMO_TEST_REDIS_URL=redis://localhost:6379
```

(`crates/mnemo-ingest/tests/ingest.rs` defaults to `redis://localhost:6399` when the env var is unset.)

## CI troubleshooting

### Symptom: "containers failed to initialize"

This usually comes from probing Qdrant on the wrong port/protocol.

- `6333` is Qdrant HTTP
- `6334` is Qdrant gRPC

If a health check tries `curl http://localhost:6334/readyz`, it can fail even when Qdrant is fine because `6334` is not HTTP.

Current workflow guidance:

- Prefer explicit startup waits over brittle health probes.
- For integration and smoke gates, wait for open sockets on:
  - `127.0.0.1:6379` (Redis)
  - `127.0.0.1:6334` (Qdrant gRPC used by tests)
- CI quality gates also enforce:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo check --workspace`
  - `cargo test --workspace --lib --bins`

Reference workflows:

- `.github/workflows/quality-gates.yml`
- `.github/workflows/memory-falsification.yml`
- `.github/workflows/nightly-soak.yml`
