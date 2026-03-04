# Mnemo

| CI | Falsification | Benchmarks | Packages | Release |
| --- | --- | --- | --- | --- |
| [![quality-gates](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml) | [![memory-falsification](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml) | [![benchmark-eval](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml) | [![package-ghcr](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml) | [![release](https://github.com/anjaustin/mnemo/actions/workflows/release.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/release.yml) |

| Version | Release Date | License | Stars | Tracked Size |
| --- | --- | --- | --- | --- |
| [![version](https://img.shields.io/github/v/tag/anjaustin/mnemo?sort=semver&label=version)](https://github.com/anjaustin/mnemo/releases) | [![release-date](https://img.shields.io/github/release-date/anjaustin/mnemo)](https://github.com/anjaustin/mnemo/releases) | [![license-apache](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE) | [![stars](https://img.shields.io/github/stars/anjaustin/mnemo)](https://github.com/anjaustin/mnemo/stargazers) | [![tracked-size](https://img.shields.io/badge/tracked-15.5%20MiB-2ea44f)](https://github.com/anjaustin/mnemo) |

| Runtime (compressed) | Runtime (unpacked) | Release Binary |
| --- | --- | --- |
| [![image-compressed](https://img.shields.io/badge/image%20compressed-40.1%20MiB-1f6feb)](https://github.com/anjaustin/mnemo/pkgs/container/mnemo%2Fmnemo-server) | [![image-unpacked](https://img.shields.io/badge/image%20unpacked-102.1%20MiB-1f6feb)](https://github.com/anjaustin/mnemo/pkgs/container/mnemo%2Fmnemo-server) | [![release-binary](https://img.shields.io/badge/release%20binary-8.5%20MiB-2da44e)](https://github.com/anjaustin/mnemo/releases/latest) |

![Mnemosyne](img/mnemosyne.gif)

**Memory infrastructure for production AI agents.**

Mnemo is a free, open-source, self-hosted memory and context engine for agent systems. It is built in Rust, uses Redis and Qdrant, and focuses on temporal correctness, fast recall, and operational simplicity.

## Who Mnemo is for

- Teams shipping assistants or autonomous agents that need memory with auditability and temporal truth.
- Builders who want self-hosted control, not a managed black box.
- Engineering orgs that care about hard quality gates and reproducible evaluation.

## Why teams choose Mnemo

- **Temporal memory, not static notes**: facts can be superseded while preserving history for point-in-time recall (`docs/TEMPORAL_VECTORIZATION.md`).
- **Fast context assembly**: hybrid retrieval and pre-assembled context blocks optimized for LLM prompts (`docs/ARCHITECTURE.md`).
- **Agent identity controls**: identity core, experience weighting, versioning, audit, rollback, and promotion flow (`docs/AGENT_IDENTITY_SUBSTRATE.md`).
- **Proof over claims**: benchmark harness plus falsification and CI gates are first-class (`docs/EVALUATION.md`, `docs/COMPETITIVE.md`, `.github/workflows/quality-gates.yml`).

## Core Capabilities

- **Temporal Knowledge Graph** - Automatically extracts entities and relationships and tracks how facts change over time.
- **Bi-temporal Retrieval** - Answers both "what is true now" and "what was true then".
- **Thread HEAD + Metadata Planner** - Improves relevance with deterministic head selection and metadata prefilter controls.
- **Identity-aware Context** - Balances stable identity with recent experience signals.
- **Chat History Importer** - Migrates existing histories with async jobs, dry-run validation, and idempotent replay protection.
- **Memory Lifecycle Webhooks** - Emits `head_advanced`, `fact_added`, `fact_superseded`, and `conflict_detected` events with retry/backoff delivery and optional HMAC signatures.
- **LLM Agnostic** - Works with Anthropic, OpenAI, Ollama, Liquid AI, or no external LLM.
- **Multi-tenant + Self-hosted** - Per-user isolation and deploy-it-yourself control.

## Quality Gates

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --lib --bins`
- `cargo test -p mnemo-storage --test storage -- --test-threads=1`
- `cargo test -p mnemo-ingest --test ingest -- --test-threads=1`
- `cargo test -p mnemo-server --test memory_api -- --test-threads=1`
- `bash tests/e2e_smoke.sh http://localhost:8080` (server running)

Reference CI gate: `.github/workflows/quality-gates.yml`.

Nightly soak and flake-detection workflow: `.github/workflows/nightly-soak.yml`.

## Releases and Packages

- Tags matching `v*.*.*` trigger automated GitHub Releases via `.github/workflows/release.yml`.
- Release artifacts include:
  - `mnemo-server-<version>-linux-amd64`
  - `mnemo-server-<version>-linux-amd64.tar.gz`
  - `SHA256SUMS.txt`
- Docker images are published to GHCR via `.github/workflows/package-ghcr.yml`.
- Published image namespace: `ghcr.io/anjaustin/mnemo/mnemo-server`.

## Measured Performance and Evaluation

- Temporal eval harness: `eval/temporal_eval.py`
- Evaluation playbook and metrics: `docs/EVALUATION.md`
- Competitive methodology and scorecard format: `docs/COMPETITIVE.md`

Quick benchmark commands:

- Mnemo only: `python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080`
- Mnemo vs Zep: `python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key`
- Scientific research pack (Mnemo): `python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080`
- Scientific research pack v2 (Mnemo): `python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080`
- Importer stress harness (ChatGPT export zip): `python3 eval/import_stress.py --mode dry-run --iterations 2 --base-url http://localhost:8080`

## Quick Start

```bash
git clone https://github.com/anjaustin/mnemo.git
cd mnemo

# Set your LLM API key (optional — works without it)
cp .env.example .env
# Edit .env with your API key

# Start Redis + Qdrant
docker compose up -d redis qdrant

# Start Mnemo
cargo run --bin mnemo-server

# Verify
curl http://localhost:8080/health
```

For a Python-first flow, see [QUICKSTART.md](QUICKSTART.md).

## Usage

All interaction is via REST API.

### Start here: High-Level Memory API

Use these two endpoints when you just want to remember and recall.

```bash
# Remember
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"kendra","text":"I love hiking in Colorado and my dog is named Bear"}'

# Recall
curl -X POST http://localhost:8080/api/v1/memory/kendra/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What are my hobbies?","contract":"default","retrieval_policy":"balanced"}'

# Diff what changed between two points in time
curl -X POST http://localhost:8080/api/v1/memory/kendra/changes_since \
  -H "Content-Type: application/json" \
  -d '{"from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'

# Detect active contradiction clusters
curl -X POST http://localhost:8080/api/v1/memory/kendra/conflict_radar \
  -H "Content-Type: application/json" \
  -d '{}'

# Explain why memory was retrieved
curl -X POST http://localhost:8080/api/v1/memory/kendra/causal_recall \
  -H "Content-Type: application/json" \
  -d '{"query":"What does Kendra prefer?"}'

# Register a webhook for memory lifecycle events
curl -X POST http://localhost:8080/api/v1/memory/webhooks \
  -H "Content-Type: application/json" \
  -d '{
    "user":"kendra",
    "target_url":"https://example.com/hooks/memory",
    "signing_secret":"whsec_demo",
    "events":["head_advanced","conflict_detected"]
  }'

# Inspect retained event delivery status
curl http://localhost:8080/api/v1/memory/webhooks/WEBHOOK_ID/events?limit=10
```

### Full workflow: Users, Sessions, Episodes

Use this flow when you need explicit user/session lifecycle control.

```bash
# 1. Create a user
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Kendra", "email": "kendra@example.com"}'

# 2. Start a session
curl -X POST http://localhost:8080/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"user_id": "USER_ID_FROM_STEP_1"}'

# 3. Add messages
curl -X POST http://localhost:8080/api/v1/sessions/SESSION_ID/episodes \
  -H "Content-Type: application/json" \
  -d '{"type":"message","role":"user","name":"Kendra","content":"I just switched from Adidas to Nike running shoes!"}'

# 4. Wait a moment for processing, then get context
curl -X POST http://localhost:8080/api/v1/users/USER_ID/context \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"What shoes does Kendra like?"}]}'
```

Inject the returned `context` string into your agent's system prompt. That's it.

### Import existing chat history

Mnemo supports async import jobs for existing chat logs.

Supported sources: `ndjson`, `chatgpt_export`, `gemini_export`.

```bash
# Start an import job (ndjson source)
curl -X POST http://localhost:8080/api/v1/import/chat-history \
  -H "Content-Type: application/json" \
  -d '{
    "user": "kendra",
    "source": "ndjson",
    "idempotency_key": "import-001",
    "dry_run": false,
    "default_session": "Imported History",
    "payload": [
      {"role": "user", "content": "I switched to Nike.", "created_at": "2025-02-01T10:00:00Z"},
      {"role": "assistant", "content": "Got it.", "created_at": "2025-02-01T10:00:05Z"}
    ]
  }'

# Poll job status
curl http://localhost:8080/api/v1/import/jobs/JOB_ID
```

## Architecture

```
Agent Runtime
    │
    ▼
REST API (mnemo-server)
    │
    ├── Redis   (users, sessions, episodes, graph state)
    └── Qdrant  (vector index for semantic retrieval)
```

Mnemo is a single Rust binary with Redis + Qdrant as backing services.

### Write Path

```
Client message
  -> /api/v1/memory or /api/v1/sessions/:id/episodes or /api/v1/import/chat-history
  -> episode persisted in Redis
  -> ingest worker extracts entities/edges
  -> graph updated in Redis + embeddings upserted to Qdrant
```

### Recall Path

```
Client query
  -> /api/v1/memory/:user/context or /api/v1/users/:id/context
  -> retrieval planner (metadata + temporal intent)
  -> hybrid search (semantic + graph + lexical fallback)
  -> token-budgeted context assembled for the agent prompt
```

### Event Path

```text
Memory lifecycle event
  -> webhook subscription match (user + event_type)
  -> async outbound POST to target_url
  -> exponential retry/backoff on non-2xx
  -> delivery telemetry retained in /api/v1/memory/webhooks/:id/events
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full deep dive.

## How Temporal Memory Works

Most memory systems overwrite facts. Mnemo keeps the timeline.

### Before and After

```text
Before (flat memory)
  "Kendra prefers Adidas"
  "Kendra prefers Nike"
  -> no clear answer to "what was true in 2024?"

After (temporal memory)
  Aug 2024: Kendra -> prefers -> Adidas   (valid)
  Feb 2025: Kendra -> prefers -> Adidas   (invalidated)
  Feb 2025: Kendra -> prefers -> Nike     (valid)
```

Mnemo tracks *when* facts became true and *when* they were superseded:

```
Aug 2024: "I love my Adidas shoes!"
  → Kendra ──prefers──▶ Adidas  (valid_at: Aug 2024)

Feb 2025: "Adidas fell apart. Nike is my new favorite."
  → Kendra ──prefers──▶ Adidas  (invalid_at: Feb 2025)  ← superseded
  → Kendra ──prefers──▶ Nike    (valid_at: Feb 2025)    ← current
```

Old facts aren't deleted. This enables point-in-time queries and change tracking.

### Real API Example

```bash
# 1) Initial preference
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"kendra","text":"I love my Adidas shoes."}'

# 2) Later correction
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"kendra","text":"Adidas fell apart. Nike is my new favorite."}'

# 3) Ask for current truth
curl -X POST http://localhost:8080/api/v1/memory/kendra/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What shoes does Kendra prefer now?"}'

# 4) Ask for historical truth
curl -X POST http://localhost:8080/api/v1/memory/kendra/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What did Kendra prefer before switching to Nike?"}'
```

## Production Readiness Checklist

- Enable API auth and provision keys before exposing Mnemo externally.
- Run Redis and Qdrant with persistent volumes and backup policy.
- Pin release versions (`v*.*.*`) for server binaries or container tags.
- Run the full quality gate stack in CI on every merge.
- Track evaluation drift with the temporal harness on a fixed dataset cadence.
- Keep `docs/PHASE_2_PRD.md` and `CHANGELOG.md` updated with shipped behavior.

## Documentation

| Document | Description |
|----------|-------------|
| [API Reference](docs/API.md) | Every endpoint with request/response examples |
| [Architecture](docs/ARCHITECTURE.md) | Data model, temporal reasoning, pipeline internals |
| [Phase 2 PRD](docs/PHASE_2_PRD.md) | Productization plan for temporal memory and proof gates |
| [Evaluation Playbook](docs/EVALUATION.md) | Reproducible temporal quality and latency measurements |
| [Competitive Plan](docs/COMPETITIVE.md) | Cross-system benchmark methodology and scorecard |
| [Chat Import Guide](docs/IMPORTING_CHAT_HISTORY.md) | Import formats, idempotency, dry run, and migration examples |
| [Webhook Delivery Guide](docs/WEBHOOKS.md) | Event types, retry semantics, and signature verification examples |
| [Domain Readiness Matrix](docs/DOMAIN_READINESS_MATRIX.md) | Domain-by-domain readiness and 30/60/90 roadmap |
| [Agent Identity Substrate](docs/AGENT_IDENTITY_SUBSTRATE.md) | Implemented P0 design for stable identity + adaptive experience |
| [Thread HEAD](docs/THREAD_HEAD.md) | Git-like current thread state and retrieval modes |
| [Temporal Vectorization](docs/TEMPORAL_VECTORIZATION.md) | Time-aware retrieval scoring and rollout plan |
| [Testing Guide](docs/TESTING.md) | Workspace, E2E, and falsification test commands |
| [Configuration](config/default.toml) | All config options with inline comments |
| [Contributing](CONTRIBUTING.md) | Dev setup, code style, PR process |
| [Changelog](CHANGELOG.md) | Release notes |

## Configuration

Mnemo reads `config/default.toml` and overrides with environment variables:

| Env Var | Description | Default |
|---------|-------------|---------|
| `MNEMO_LLM_API_KEY` | API key for entity extraction | (none) |
| `MNEMO_LLM_PROVIDER` | `openai`, `anthropic`, `ollama`, `liquid` | `openai` |
| `MNEMO_LLM_MODEL` | Model for extraction | `gpt-4o-mini` |
| `MNEMO_EMBEDDING_API_KEY` | Embedding API key | (none) |
| `MNEMO_AUTH_ENABLED` | Require API key auth (`true`/`false`) | `false` |
| `MNEMO_AUTH_API_KEYS` | Comma-separated accepted API keys | (none) |
| `MNEMO_REDIS_URL` | Redis connection | `redis://localhost:6379` |
| `MNEMO_QDRANT_URL` | Qdrant connection | `http://localhost:6334` |
| `MNEMO_METADATA_PREFILTER_ENABLED` | Enable metadata prefilter planner | `true` |
| `MNEMO_METADATA_SCAN_LIMIT` | Candidate scan limit for prefilter planner | `400` |
| `MNEMO_METADATA_RELAX_IF_EMPTY` | Relax strict metadata filters when empty | `false` |
| `MNEMO_SERVER_PORT` | Server port | `8080` |

Webhook outbound delivery defaults are currently runtime defaults in `crates/mnemo-server/src/main.rs`:

- `max_attempts=3`
- `base_backoff_ms=200`
- `request_timeout_ms=3000`

## Project Status

**Phase 1.5 — Production Hardening** ✅ complete

- compilation + integration coverage
- auth middleware
- full-text + hybrid retrieval
- memory API + falsification CI gate

**Phase 2 — Temporal Productization** 🚧 in progress

- M1 Thread HEAD completion ✅
- M2 Temporal retrieval v2 diagnostics ✅
- M3 Metadata index layer ✅
- M4 Competitive publication v1 🚧
- M5 Agent Identity Substrate P0 ✅

See `docs/PHASE_2_PRD.md` for current milestones.

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions and guidelines.

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

*Named after Mnemosyne (Μνημοσύνη), the Greek Titaness of memory and mother of the Muses.*
