# Mnemo

| CI | Falsification | Benchmarks | Packages | Release |
| --- | --- | --- | --- | --- |
| [![quality-gates](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml) | [![memory-falsification](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml) | [![benchmark-eval](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml) | [![package-ghcr](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml) | [![release](https://github.com/anjaustin/mnemo/actions/workflows/release.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/release.yml) |

| Version | Release Date | License | Stars | Downloads |
| --- | --- | --- | --- | --- |
| [![version](https://img.shields.io/github/v/tag/anjaustin/mnemo?sort=semver&label=version)](https://github.com/anjaustin/mnemo/releases) | [![release-date](https://img.shields.io/github/release-date/anjaustin/mnemo)](https://github.com/anjaustin/mnemo/releases) | [![license-apache](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE) | [![stars](https://img.shields.io/github/stars/anjaustin/mnemo)](https://github.com/anjaustin/mnemo/stargazers) | [![downloads](https://img.shields.io/github/downloads/anjaustin/mnemo/total?label=downloads&color=blue)](https://github.com/anjaustin/mnemo/releases) |

| Latest Release | Release Artifacts | GHCR Package |
| --- | --- | --- |
| [![latest-release](https://img.shields.io/github/v/release/anjaustin/mnemo?display_name=tag&sort=semver)](https://github.com/anjaustin/mnemo/releases/latest) | [![release-assets](https://img.shields.io/badge/assets-linux--amd64%20%7C%20tar.gz%20%7C%20sha256-2da44e)](https://github.com/anjaustin/mnemo/releases/latest) | [![ghcr-package](https://img.shields.io/badge/ghcr-mnemo--server-1f6feb)](https://github.com/anjaustin/mnemo/pkgs/container/mnemo%2Fmnemo-server) |

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
- **Time Travel Trace** - Compares memory snapshots across two points in time and returns timeline-level "why it changed" evidence.
- **Time Travel Summary** - Returns fast gained/lost fact and episode counters for first-pass RCA.
- **Governance Policies** - Per-user retention defaults, webhook domain allowlists, and audit trails for policy/destructive operations.
  - Policy preview and violation-window query endpoints for safer rollout dry-runs and incident triage.
  - Default contract/retrieval policy fallback and retention enforcement for episode writes.
- **Operator Endpoints** - Dashboard summary, request-id trace lookup, and drill automation for dead-letter recovery, RCA, and governance workflows.
- **Python SDK** - Zero-dependency sync client (`Mnemo`) and async client (`AsyncMnemo`) with full API coverage, typed results, and `x-mnemo-request-id` propagation.
  - **LangChain adapter** - Drop-in `MnemoChatMessageHistory` (`BaseChatMessageHistory`) via `mnemo.ext.langchain`.
  - **LlamaIndex adapter** - Drop-in `MnemoChatStore` (`BaseChatStore`, all 7 abstract methods) via `mnemo.ext.llamaindex`.
- **Raw Vector API** - General-purpose vector database endpoints for external integrations (upsert, similarity search, delete, count, namespace lifecycle).
- **AnythingLLM Integration** - Drop-in vector DB provider for [AnythingLLM](https://github.com/Mintplex-Labs/anything-llm) (55.5k stars). See `integrations/anythingllm/`.
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
- `bash tests/operator_p0_drills.sh`

Reference CI gate: `.github/workflows/quality-gates.yml`.

Nightly soak and flake-detection workflow: `.github/workflows/nightly-soak.yml`.

## Releases and Packages

- Tags matching `v*.*.*` trigger automated GitHub Releases via `.github/workflows/release.yml`.
- Release workflow expectation: bump `Cargo.toml` (`workspace.package.version`) and `sdk/python/pyproject.toml` together before tagging.
- Current in-repo development version: `0.3.3`.
- Release artifacts include:
  - `mnemo-server-<version>-linux-amd64`
  - `mnemo-server-<version>-linux-amd64.tar.gz`
  - `SHA256SUMS.txt`
- Docker images are published to GHCR via `.github/workflows/package-ghcr.yml` on `main` and version tags.
- Published image namespace: `ghcr.io/anjaustin/mnemo/mnemo-server`.

Get latest release assets:

```bash
gh release download --repo anjaustin/mnemo --pattern 'mnemo-server-*' --pattern 'SHA256SUMS.txt'
```

Pull package images:

```bash
# latest default-branch image
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:latest

# immutable tag image
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:<version>

# branch image (main)
docker pull ghcr.io/anjaustin/mnemo/mnemo-server:main
```

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
  -d '{"user":"acct_mgr_jordan","text":"Acme Corp renewal is due on 2025-09-30 and procurement requires SOC 2 Type II before signature."}'

# Recall
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What are the renewal blockers for Acme?","contract":"default","retrieval_policy":"balanced"}'

# Diff what changed between two points in time
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/changes_since \
  -H "Content-Type: application/json" \
  -d '{"from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'

# Detect active contradiction clusters
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/conflict_radar \
  -H "Content-Type: application/json" \
  -d '{}'

# Explain why memory was retrieved
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/causal_recall \
  -H "Content-Type: application/json" \
  -d '{"query":"Why do we think Acme has legal risk this quarter?"}'

# Trace why an answer changed over time
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/time_travel/trace \
  -H "Content-Type: application/json" \
  -d '{"query":"How did Acme renewal risk evolve?","from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z","contract":"historical_strict"}'

# Lightweight summary for fast first-pass RCA
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/time_travel/summary \
  -H "Content-Type: application/json" \
  -d '{"query":"How did Acme renewal risk evolve?","from":"2025-02-01T00:00:00Z","to":"2025-04-01T00:00:00Z"}'

# Register a webhook for memory lifecycle events
curl -X POST http://localhost:8080/api/v1/memory/webhooks \
  -H "Content-Type: application/json" \
  -d '{
    "user":"acct_mgr_jordan",
    "target_url":"https://example.com/hooks/memory",
    "signing_secret":"whsec_demo",
    "events":["head_advanced","conflict_detected"]
  }'

# Inspect retained event delivery status
curl http://localhost:8080/api/v1/memory/webhooks/WEBHOOK_ID/events?limit=10

# Set user governance policy (allowlist + retention defaults)
curl -X PUT http://localhost:8080/api/v1/policies/acct_mgr_jordan \
  -H "Content-Type: application/json" \
  -d '{"webhook_domain_allowlist":["hooks.acme.example"],"retention_days_message":365}'

# Preview policy impact before applying
curl -X POST http://localhost:8080/api/v1/policies/acct_mgr_jordan/preview \
  -H "Content-Type: application/json" \
  -d '{"retention_days_message":30}'

# Query policy violations inside a time window
curl "http://localhost:8080/api/v1/policies/acct_mgr_jordan/violations?from=2026-03-01T00:00:00Z&to=2026-03-04T00:00:00Z&limit=50"
```

### Full workflow: Users, Sessions, Episodes

Use this flow when you need explicit user/session lifecycle control.

```bash
# 1. Create a user
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Jordan Lee", "email": "jordan.lee@acme-revenueops.com"}'

# 2. Start a session
curl -X POST http://localhost:8080/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"user_id": "USER_ID_FROM_STEP_1"}'

# 3. Add messages
curl -X POST http://localhost:8080/api/v1/sessions/SESSION_ID/episodes \
  -H "Content-Type: application/json" \
  -d '{"type":"message","role":"user","name":"Jordan Lee","content":"Acme legal approved redlines but procurement still needs SOC 2 evidence before renewal."}'

# 4. Wait a moment for processing, then get context
curl -X POST http://localhost:8080/api/v1/users/USER_ID/context \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"What still blocks Acme renewal?"}]}'
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
    "user": "acct_mgr_jordan",
    "source": "ndjson",
    "idempotency_key": "import-001",
    "dry_run": false,
    "default_session": "Imported History",
    "payload": [
      {"role": "user", "content": "Acme procurement requested SOC 2 report by Friday.", "created_at": "2025-02-01T10:00:00Z"},
      {"role": "assistant", "content": "Acknowledged. I will track this as a renewal blocker.", "created_at": "2025-02-01T10:00:05Z"}
    ]
  }'

# Poll job status
curl http://localhost:8080/api/v1/import/jobs/JOB_ID
```

### Python SDK

Install:

```bash
pip install git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python

# With async support (aiohttp)
pip install "mnemo-client[async] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"

# With LangChain adapter
pip install "mnemo-client[langchain] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"

# With LlamaIndex adapter
pip install "mnemo-client[llamaindex] @ git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python"
```

Basic usage:

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080")

# Remember
client.add("jordan", "Acme renewal is at risk — procurement needs SOC 2 before signature.")

# Recall
ctx = client.context("jordan", "What is blocking Acme renewal?")
print(ctx.text)  # inject into agent system prompt
```

LangChain adapter:

```python
from mnemo import Mnemo
from mnemo.ext.langchain import MnemoChatMessageHistory

client = Mnemo("http://localhost:8080")
history = MnemoChatMessageHistory(session_name="acme-deal-chat", user_id="jordan", client=client)

history.add_user_message("What are the Acme renewal blockers?")
history.add_ai_message("SOC 2 evidence is still required by procurement.")

print(history.messages)  # [HumanMessage(...), AIMessage(...)]
history.clear()
```

LlamaIndex adapter:

```python
from mnemo import Mnemo
from mnemo.ext.llamaindex import MnemoChatStore
from llama_index.core.llms import ChatMessage, MessageRole

client = Mnemo("http://localhost:8080")
store = MnemoChatStore(client=client, user_id="jordan")

store.add_message("acme-session", ChatMessage(role=MessageRole.USER, content="What blocks renewal?"))
msgs = store.get_messages("acme-session")
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
  "Acme renewal status is green"
  "Acme renewal status is at risk"
  -> no clear answer to "what was true in 2024?"

After (temporal memory)
  Aug 2024: Acme -> renewal_status -> green    (valid)
  Feb 2025: Acme -> renewal_status -> green    (invalidated)
  Feb 2025: Acme -> renewal_status -> at_risk  (valid)
```

Mnemo tracks *when* facts became true and *when* they were superseded:

```
Aug 2024: "Acme legal and procurement are aligned; renewal looks green."
  → Acme ──renewal_status──▶ green  (valid_at: Aug 2024)

Feb 2025: "Procurement blocked signature pending SOC 2 report. Renewal is now at risk."
  → Acme ──renewal_status──▶ green    (invalid_at: Feb 2025)  ← superseded
  → Acme ──renewal_status──▶ at_risk  (valid_at: Feb 2025)    ← current
```

Old facts aren't deleted. This enables point-in-time queries and change tracking.

### Real API Example

```bash
# 1) Initial preference
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"acct_mgr_jordan","text":"Acme renewal status is green and legal has no open issues."}'

# 2) Later correction
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"acct_mgr_jordan","text":"Acme renewal is now at risk because procurement requires SOC 2 evidence before signature."}'

# 3) Ask for current truth
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What is Acme renewal status now?"}'

# 4) Ask for historical truth
curl -X POST http://localhost:8080/api/v1/memory/acct_mgr_jordan/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What was Acme renewal status before procurement blocked signature?"}'
```

## Deployment

Production deployment artifacts are in `deploy/`. Each target is fully falsified (5-gate test: health, write, context, restart-persistence, service status).

| Target | Tooling | Status | Guide |
|--------|---------|--------|-------|
| Docker (all-in-one) | `docker compose` | ✅ Falsified | [deploy/docker/DEPLOY.md](deploy/docker/DEPLOY.md) |
| Bare Metal / VPS | systemd + nginx | ✅ Falsified | [deploy/bare-metal/DEPLOY.md](deploy/bare-metal/DEPLOY.md) |
| AWS EC2 | CloudFormation | ✅ Falsified | [deploy/aws/cloudformation/DEPLOY.md](deploy/aws/cloudformation/DEPLOY.md) |
| GCP Compute Engine | Terraform | ✅ Falsified | [deploy/gcp/DEPLOY.md](deploy/gcp/DEPLOY.md) |
| DigitalOcean | Terraform | ✅ Artifacts ready | [deploy/digitalocean/DEPLOY.md](deploy/digitalocean/DEPLOY.md) |
| Render | `render.yaml` | ✅ Artifacts ready | [deploy/render/DEPLOY.md](deploy/render/DEPLOY.md) |
| Railway | Railway template | ✅ Artifacts ready | [deploy/railway/DEPLOY.md](deploy/railway/DEPLOY.md) |
| Elestio | Managed hosting | ✅ Artifacts ready | [deploy/elestio/DEPLOY.md](deploy/elestio/DEPLOY.md) |
| Northflank | Stack definition | ✅ Artifacts ready | [deploy/northflank/DEPLOY.md](deploy/northflank/DEPLOY.md) |
| Linode / Akamai | Terraform | ✅ Falsified | [deploy/linode/DEPLOY.md](deploy/linode/DEPLOY.md) |

### Quickest path to production

```bash
# Clone and configure
git clone https://github.com/anjaustin/mnemo.git
cd mnemo/deploy/docker
cp .env.example .env
# Edit .env — set LLM key, auth keys, etc.

# Start
docker compose -f docker-compose.prod.yml up -d

# Verify
curl http://localhost:8080/health
# {"status":"ok","version":"0.3.2"}
```

See [deploy/docker/DEPLOY.md](deploy/docker/DEPLOY.md) for full options including the managed-services variant (external Redis + Qdrant Cloud).

## Production Readiness Checklist

- Enable API auth and provision keys before exposing Mnemo externally.
- Run Redis and Qdrant with persistent volumes and backup policy.
- Pin release versions (`v*.*.*`) for server binaries or container tags.
- Run the full quality gate stack in CI on every merge.
- Track evaluation drift with the temporal harness on a fixed dataset cadence.
- Keep `CHANGELOG.md`, `docs/OPERATOR_UX_EXECUTION_BACKLOG.md`, and integration READMEs updated with shipped behavior.

## Documentation

| Document | Description |
|----------|-------------|
| [Deployment PRD](docs/PRD_DEPLOY.md) | T1–T10 deployment targets, falsification gates, rollout phasing |
| [API Reference](docs/API.md) | Every endpoint with request/response examples |
| [Architecture](docs/ARCHITECTURE.md) | Data model, temporal reasoning, pipeline internals |
| [Phase 2 PRD](docs/PHASE_2_PRD.md) | Productization plan for temporal memory and proof gates |
| [Evaluation Playbook](docs/EVALUATION.md) | Reproducible temporal quality and latency measurements |
| [Competitive Plan](docs/COMPETITIVE.md) | Cross-system benchmark methodology and scorecard |
| [Chat Import Guide](docs/IMPORTING_CHAT_HISTORY.md) | Import formats, idempotency, dry run, and migration examples |
| [Webhook Delivery Guide](docs/WEBHOOKS.md) | Event types, retry semantics, and signature verification examples |
| [P0 Ops Control Plane PRD](docs/P0_OPS_CONTROL_PLANE_PRD.md) | Cloud-grade ops goals, scope, falsification matrix, and rollout criteria |
| [Operator UX PRD](docs/OPERATOR_UX_PRD.md) | Control-plane UX strategy, screens, metrics, and phased rollout |
| [Operator UX Backlog](docs/OPERATOR_UX_EXECUTION_BACKLOG.md) | Ticketized execution plan for the two hero operator lanes |
| [SDK Integrations PRD](docs/SDK_INTEGRATIONS_PRD.md) | Python SDK rebuild, LangChain adapter, LlamaIndex adapter |
| [Operator Dashboard PRD](docs/OPERATOR_DASHBOARD_PRD.md) | Embedded zero-deployment operator dashboard |
| [AnythingLLM Integration](integrations/anythingllm/README.md) | Drop-in vector DB provider for AnythingLLM |
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
| `MNEMO_WEBHOOKS_ENABLED` | Enable outbound webhook delivery | `true` |
| `MNEMO_WEBHOOKS_MAX_ATTEMPTS` | Retry attempts before dead-lettering | `3` |
| `MNEMO_WEBHOOKS_BASE_BACKOFF_MS` | Base backoff duration for retries | `200` |
| `MNEMO_WEBHOOKS_TIMEOUT_MS` | Per-attempt request timeout | `3000` |
| `MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK` | Max retained event rows per webhook | `1000` |
| `MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE` | Max outbound sends per webhook per minute | `120` |
| `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD` | Consecutive failures before opening circuit | `5` |
| `MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS` | Circuit cooldown before retrying sends | `60000` |
| `MNEMO_WEBHOOKS_PERSISTENCE_ENABLED` | Persist webhook subscriptions/events in Redis | `true` |
| `MNEMO_WEBHOOKS_PERSISTENCE_PREFIX` | Redis key suffix for webhook state | `webhooks` |
| `MNEMO_SERVER_PORT` | Server port | `8080` |

Webhook outbound delivery defaults are configured in `config/default.toml` and can be overridden with env vars:

- `max_attempts=3`
- `base_backoff_ms=200`
- `request_timeout_ms=3000`

## Project Status

**Phase 1.5 — Production Hardening** ✅ complete

- compilation + integration coverage
- auth middleware
- full-text + hybrid retrieval
- memory API + falsification CI gate

**Phase 2 — Temporal Productization** ✅ complete

- M1 Thread HEAD completion ✅
- M2 Temporal retrieval v2 diagnostics ✅
- M3 Metadata index layer ✅
- M4 Competitive publication v1 ✅
- M5 Agent Identity Substrate P0 ✅

See `docs/PHASE_2_PRD.md` for milestones.

**Phase 2 Deployment — Cloud IaC** 🚧 in progress

- T1 Docker production compose ✅
- T2 Bare Metal systemd + nginx ✅
- T3 AWS CloudFormation — all 5 gates passed ✅
- T4 GCP Terraform — all 5 gates passed ✅
- T5 DigitalOcean Terraform — artifacts written ✅
- T6 Render — artifacts written ✅
- T7 Railway — artifacts written ✅
- T8 Elestio — artifacts written ✅
- T9 Northflank — artifacts written ✅
- T10 Linode — all 5 gates passed ✅

See `docs/PRD_DEPLOY.md` for full deployment PRD and falsification gate contract.

**Phase 3 — Operator UX & Control Plane** 🚧 in progress

- Governance policy APIs (retention, allowlists, audit) ✅
- Read/write retention enforcement ✅
- Operator hero-lane backend (summary, trace, preview, violations) ✅
- Webhook ops endpoints (dead-letter, replay, retry, stats) ✅
- Falsification suite: 56 integration tests including 4×4 contract/policy matrix ✅
- Raw Vector API (6 endpoints — upsert, search, delete, count, namespace lifecycle) ✅
- Session Messages API (list, clear, delete-by-index) ✅
- AnythingLLM vector DB provider (`integrations/anythingllm/`) ✅
- Python SDK full rebuild: sync + async clients, 27 methods, full API coverage ✅
- LangChain `MnemoChatMessageHistory` drop-in adapter ✅
- LlamaIndex `MnemoChatStore` drop-in adapter (all 7 abstract methods) ✅
- SDK falsification test suite: 65/65 assertions pass ✅
- Operator-facing frontend surfaces 🚧 (`docs/OPERATOR_DASHBOARD_PRD.md`)
- p95 latency evidence capture 🚧

See `docs/OPERATOR_UX_PRD.md`, `docs/SDK_INTEGRATIONS_PRD.md`, and `docs/OPERATOR_DASHBOARD_PRD.md` for current scope.

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions and guidelines.

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

*Named after Mnemosyne (Μνημοσύνη), the Greek Titaness of memory and mother of the Muses.*
