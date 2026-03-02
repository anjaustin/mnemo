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
