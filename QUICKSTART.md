# Mnemo Quickstart

Go from nothing to a working memory-enabled agent in under 5 minutes.

**Requirements:** [Docker](https://docs.docker.com/get-docker/) with [Compose v2](https://docs.docker.com/compose/install/)

## 1. Start Mnemo (30 seconds)

```bash
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
```

This starts three containers:

| Service | Purpose | Port |
|---------|---------|------|
| mnemo-server | Memory API + MCP server + Dashboard | 8080 |
| redis | Session and episode storage | 6379 |
| qdrant | Vector search | 6333/6334 |

No API keys are required. Embeddings run locally using AllMiniLML6V2.

Or start manually:

```bash
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/docker-compose.quickstart.yml -o docker-compose.quickstart.yml
docker compose -f docker-compose.quickstart.yml up -d
```

## 2. Verify it works

```bash
# Health check
curl http://localhost:8080/health

# Store a memory
curl -X POST http://localhost:8080/api/v1/memory \
  -H 'Content-Type: application/json' \
  -d '{"user":"alice","text":"I love hiking in Colorado","role":"user"}'

# Retrieve context
curl -X POST http://localhost:8080/api/v1/memory/alice/context \
  -H 'Content-Type: application/json' \
  -d '{"query":"What does Alice enjoy?"}'
```

## 3. Connect an MCP-compatible agent (30 seconds)

The Docker image includes `mnemo-mcp-server`, a stdio-based MCP server that
any MCP-compatible client can use directly.

### Claude Code

Add to `~/.claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "mnemo": {
      "command": "docker",
      "args": ["exec", "-i", "mnemo-server", "mnemo-mcp-server"],
      "env": {
        "MNEMO_MCP_DEFAULT_USER": "alice"
      }
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "mnemo": {
      "command": "docker",
      "args": ["exec", "-i", "mnemo-server", "mnemo-mcp-server"],
      "env": {
        "MNEMO_MCP_DEFAULT_USER": "alice"
      }
    }
  }
}
```

### Available MCP tools

| Tool | Description |
|------|-------------|
| `remember` | Store a memory (text, role, optional session) |
| `recall` | Retrieve context for a query |
| `graph` | Query the knowledge graph |
| `identity` | Get/update agent identity |
| `digest` | Get memory digest summary |
| `coherence` | Check memory coherence |
| `health` | Server health check |
| `delegate` | Grant another agent read access |
| `revoke` | Revoke delegated access |
| `scopes` | List active memory scopes |

## 4. Dashboard

Open [http://localhost:8080/_/](http://localhost:8080/_/) in your browser.

Pages: Home, Memory (episodes/facts/search/temporal diff), Webhooks, Time Travel, Governance, Traces, Explorer, LLM Spans.

## 5. Python SDK

```bash
pip install mnemo-client
```

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080")
client.add("alice", "I love hiking in Colorado")
client.add("alice", "My favorite trail is Hanging Lake")

ctx = client.context("alice", "What outdoor activities does Alice enjoy?")
print(ctx.text)
```

## Adding LLM extraction (optional)

By default the quickstart runs without an LLM. Memory storage and retrieval
work, but entity/relationship extraction and summarization are disabled.

To enable extraction, pass your LLM provider as environment variables:

```bash
docker compose -f docker-compose.quickstart.yml down

MNEMO_LLM_PROVIDER=anthropic \
MNEMO_LLM_API_KEY=sk-ant-your-key \
MNEMO_LLM_MODEL=claude-haiku-4-20250514 \
docker compose -f docker-compose.quickstart.yml up -d
```

Supported providers: `anthropic`, `openai`, `ollama`, `liquid`

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MNEMO_LLM_PROVIDER` | `none` | `anthropic`, `openai`, `ollama`, `liquid`, or `none` |
| `MNEMO_LLM_API_KEY` | — | Required when LLM provider is not `none` |
| `MNEMO_LLM_MODEL` | — | Model name for extraction |
| `MNEMO_EMBEDDING_PROVIDER` | `local` | `local` (fastembed) or `openai` |
| `MNEMO_EMBEDDING_MODEL` | `AllMiniLML6V2` | Embedding model |
| `MNEMO_SERVER_PORT` | `8080` | Server listen port |
| `MNEMO_AUTH_ENABLED` | `false` | Enable API key auth |
| `MNEMO_AUTH_API_KEYS` | — | Comma-separated API keys when auth enabled |

See [`config/default.toml`](config/default.toml) for the full configuration reference.

## Stop

```bash
docker compose -f docker-compose.quickstart.yml down        # stop, keep data
docker compose -f docker-compose.quickstart.yml down -v     # stop, delete volumes
```

## Next steps

- [Production deployment](deploy/docker/DEPLOY.md) — TLS, auth, managed services
- [Python SDK](sdk/python/) — Sync and async clients with typed results
- [TypeScript SDK](sdk/typescript/) — Full-featured TypeScript/Node.js client
- [Deployment guides](deploy/) — AWS, GCP, DigitalOcean, Render, Railway, Kubernetes
