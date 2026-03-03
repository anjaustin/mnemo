# Testing Guide

This project has four practical testing layers.

## 1) Workspace tests

```bash
cargo test --workspace
```

## 2) Deterministic end-to-end smoke script

Assumes the server is already running on `http://localhost:8080`.

```bash
./tests/e2e_smoke.sh
```

This smoke test is designed to pass without external LLM credentials.

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

## 5) Importer stress harness (large real-world export)

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
