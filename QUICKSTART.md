# Mnemo Quickstart

Get from zero to "it remembers" in under 5 minutes.

## One-line start (Docker — no Rust toolchain needed)

```bash
# Set your LLM key (Anthropic or OpenAI)
export MNEMO_LLM_API_KEY=sk-...

# Start the full stack (Mnemo + Redis + Qdrant)
docker compose up -d
```

Then open **http://localhost:8080/_/** for the operator dashboard.

Or use the bootstrap script:

```bash
curl -fsSL https://raw.githubusercontent.com/anomalyco/mnemo/main/scripts/quickstart.sh \
  | MNEMO_LLM_API_KEY=sk-... bash
```

---

## Quick smoke test

```bash
# Health check
curl http://localhost:8080/health

# Add a memory
curl -X POST http://localhost:8080/api/v1/memory \
  -H 'Content-Type: application/json' \
  -d '{"user":"alice","text":"I love hiking in Colorado and my dog is named Bear","role":"user"}'

# Retrieve context
curl -X POST http://localhost:8080/api/v1/memory/alice/context \
  -H 'Content-Type: application/json' \
  -d '{"query":"What are my hobbies?"}'
```

---

## Python SDK

```bash
pip install mnemo-client
```

```python
from mnemo import Mnemo

m = Mnemo("http://localhost:8080")

m.add("alice", "I love hiking in Colorado and my dog is named Bear")
m.add("alice", "I just got back from camping near Breckenridge with Sarah")

ctx = m.context("alice", "What are my recent trips and hobbies?")
print(ctx.text)
```

---

## Development (build from source)

```bash
# Start infra only
docker compose up -d redis qdrant

# Run server from source
MNEMO_LLM_API_KEY=sk-... cargo run --bin mnemo-server
```

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MNEMO_LLM_API_KEY` | — | Required for entity extraction |
| `MNEMO_LLM_PROVIDER` | `anthropic` | `anthropic` or `openai` |
| `MNEMO_LLM_MODEL` | `claude-haiku-4-20250514` | Model name |
| `MNEMO_EMBEDDING_PROVIDER` | `local` | `local` (fastembed) or `openai` |
| `MNEMO_PORT` | `8080` | Host port to expose |
| `MNEMO_AUTH_ENABLED` | `false` | Enable API key auth |
| `MNEMO_API_KEY` | — | API key(s) when auth enabled |

## Stop

```bash
docker compose down          # stop, keep data
docker compose down -v       # stop, delete volumes
```
