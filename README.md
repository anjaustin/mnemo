# 🧠 Mnemo

[![memory-falsification](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml)

**Memory that evolves. Context that matters.**

Mnemo is a free, open-source, self-hosted memory and context engine for AI agents. Built in Rust for raw performance, backed by Redis and Qdrant for sub-50ms retrieval latency.

> **Why Mnemo?** Zep deprecated their open-source Community Edition. Mem0 is SaaS-first. Letta isn't production-ready. Mnemo is the fully open-source, high-performance alternative the community needs.

## Features

- **Temporal Knowledge Graph** — Entities and relationships extracted automatically, tracking how facts change over time
- **Sub-50ms Retrieval** — Pre-assembled context blocks ready for LLM injection
- **Bi-temporal Model** — Query what you knew at any point in time, not just what's current
- **Multi-tenant** — Complete data isolation per user
- **LLM Agnostic** — Anthropic, OpenAI, Ollama, Liquid AI, or no LLM at all
- **Self-hosted First** — Docker Compose for dev, your data stays yours

## Quality Gates

- `cargo test --workspace`
- `./tests/e2e.sh` (server running)
- `cargo test -p mnemo-server --test memory_api -- --test-threads=1` (memory falsification)

## Why Mnemo (Measured)

- Temporal eval harness (`eval/temporal_eval.py`) currently shows better accuracy on time-sensitive recall than baseline mode in local runs.
- Stale-fact rate is explicitly tracked and reported in `docs/EVALUATION.md`.
- Memory API behavior is guarded by falsification tests in CI (`memory-falsification` workflow).
- Competitive benchmarking publication format is defined in `docs/COMPETITIVE.md`.

Quick benchmark commands:

- Mnemo only: `python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080`
- Mnemo vs Zep: `python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key`

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

All interaction is via REST API. Here's a complete workflow:

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

### High-Level Memory API

You can also use the streamlined memory endpoints:

```bash
# Remember
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"kendra","text":"I love hiking in Colorado and my dog is named Bear"}'

# Recall
curl -X POST http://localhost:8080/api/v1/memory/kendra/context \
  -H "Content-Type: application/json" \
  -d '{"query":"What are my hobbies?"}'
```

## Architecture

```
Your Agent ──▶ REST API ──▶ Mnemo Server
                                │
                    ┌───────────┴───────────┐
                    ▼                       ▼
                  Redis                  Qdrant
               (state, graph           (vectors,
               adjacency)              search)
```

Mnemo is a single Rust binary. No Neo4j. No JVM. No garbage collector.

You send messages → Mnemo extracts entities and relationships via LLM → builds a temporal knowledge graph per user → on retrieval, runs hybrid search (semantic + graph traversal) → assembles a token-budgeted context string for your agent.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full deep dive.

## How Temporal Memory Works

Most memory systems store facts as static key-value pairs. Mnemo tracks *when* facts became true and *when* they were superseded:

```
Aug 2024: "I love my Adidas shoes!"
  → Kendra ──prefers──▶ Adidas  (valid_at: Aug 2024)

Feb 2025: "Adidas fell apart. Nike is my new favorite."
  → Kendra ──prefers──▶ Adidas  (invalid_at: Feb 2025)  ← superseded
  → Kendra ──prefers──▶ Nike    (valid_at: Feb 2025)    ← current
```

Old facts aren't deleted. This enables point-in-time queries and change tracking.

## Documentation

| Document | Description |
|----------|-------------|
| [API Reference](docs/API.md) | Every endpoint with request/response examples |
| [Architecture](docs/ARCHITECTURE.md) | Data model, temporal reasoning, pipeline internals |
| [Phase 2 PRD](docs/PHASE_2_PRD.md) | Productization plan for temporal memory and proof gates |
| [Evaluation Playbook](docs/EVALUATION.md) | Reproducible temporal quality and latency measurements |
| [Competitive Plan](docs/COMPETITIVE.md) | Cross-system benchmark methodology and scorecard |
| [Agent Identity Substrate](docs/AGENT_IDENTITY_SUBSTRATE.md) | P0 spec for stable agent identity + adaptive experience |
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
| `MNEMO_REDIS_URL` | Redis connection | `redis://localhost:6379` |
| `MNEMO_QDRANT_URL` | Qdrant connection | `http://localhost:6334` |
| `MNEMO_SERVER_PORT` | Server port | `8080` |

## Project Status

**Phase 1.5 — Production Hardening** ✅ complete

- compilation + integration coverage
- auth middleware
- full-text + hybrid retrieval
- memory API + falsification CI gate

**Phase 2 — Temporal Productization** 🚧 in progress

- M1 Thread HEAD completion ✅
- M2 Temporal retrieval v2 diagnostics ✅
- M3 Metadata index layer ⏳
- M4 Competitive publication v1 🚧

See `docs/PHASE_2_PRD.md` for current milestones.

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions and guidelines.

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

*Named after Mnemosyne (Μνημοσύνη), the Greek Titaness of memory and mother of the Muses.*
