# Testing Guide

This project has three practical testing layers.

## 1) Workspace tests

```bash
cargo test --workspace
```

## 2) End-to-end smoke script

Assumes the server is already running on `http://localhost:8080`.

```bash
./tests/e2e.sh
```

## 3) Memory API falsification suite

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
cargo test -p mnemo-server --test memory_api -- --test-threads=1
docker compose -f docker-compose.test.yml down
```

## CI troubleshooting

### Symptom: "containers failed to initialize"

This usually comes from probing Qdrant on the wrong port/protocol.

- `6333` is Qdrant HTTP
- `6334` is Qdrant gRPC

If a health check tries `curl http://localhost:6334/readyz`, it can fail even when Qdrant is fine because `6334` is not HTTP.

Current workflow guidance:

- Prefer explicit startup waits over brittle health probes.
- For the memory falsification gate, wait for open sockets on:
  - `127.0.0.1:6379` (Redis)
  - `127.0.0.1:6334` (Qdrant gRPC used by tests)

Reference workflow: `.github/workflows/memory-falsification.yml`.
