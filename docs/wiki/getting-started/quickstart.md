# Quickstart

Get Mnemo running in 5 minutes.

---

## Prerequisites

- Docker and Docker Compose
- 2GB RAM minimum
- curl (for testing)

---

## One-Command Install

```bash
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
```

This starts:
- **mnemo-server** on port 8080
- **Redis** (state storage) on port 6379
- **Qdrant** (vector storage) on port 6333

No API keys or external services required. Uses local embeddings (FastEmbed).

---

## Manual Docker Compose

If you prefer to run manually:

```bash
# Clone the repository
git clone https://github.com/anjaustin/mnemo.git
cd mnemo

# Start services
docker compose -f deploy/docker/docker-compose.yml up -d

# Check health
curl http://localhost:8080/health
```

Expected response:
```json
{"status":"healthy","version":"0.9.0"}
```

---

## Verify Installation

### Store a memory

```bash
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{
    "user": "demo",
    "text": "The project deadline is March 15th."
  }'
```

Response:
```json
{
  "ok": true,
  "user_id": "01234567-...",
  "session_id": "01234567-...",
  "episode_id": "01234567-..."
}
```

### Recall the memory

```bash
curl -X POST http://localhost:8080/api/v1/memory/demo/context \
  -H "Content-Type: application/json" \
  -d '{"query": "When is the deadline?"}'
```

Response includes the retrieved context:
```json
{
  "text": "## Memory Context\n\nThe project deadline is March 15th.",
  "token_count": 42,
  "entities": [...],
  "facts": [...],
  ...
}
```

---

## What's Happening

When you store a memory, Mnemo:

1. **Creates a user** (if `demo` doesn't exist)
2. **Creates a session** (conversation thread)
3. **Creates an episode** (the memory unit)
4. **Extracts entities** (e.g., "project", "March 15th")
5. **Creates edges** (e.g., "project → has_deadline → March 15th")
6. **Generates embeddings** for semantic search

When you recall:

1. **Hybrid search** - semantic vectors + full-text + graph traversal
2. **Reranking** - scores and orders results
3. **Context assembly** - builds a token-budgeted prompt

---

## Next Steps

- **[First Memory](first-memory.md)** - Detailed walkthrough
- **[SDK Setup](sdk-setup.md)** - Install Python or TypeScript SDK
- **[Core Concepts](../concepts/overview.md)** - Understand the data model
- **[Configuration](../reference/configuration.md)** - Customize settings

---

## Stopping Mnemo

```bash
# Stop containers
docker compose -f deploy/docker/docker-compose.yml down

# Stop and remove volumes (deletes all data)
docker compose -f deploy/docker/docker-compose.yml down -v
```

---

## Troubleshooting

### Port 8080 in use

```bash
# Use a different port
MNEMO_PORT=9090 docker compose -f deploy/docker/docker-compose.yml up -d
```

### Not enough memory

Mnemo with local embeddings needs ~1.5GB RAM. If constrained:

```bash
# Use remote embeddings (requires API key)
OPENAI_API_KEY=sk-... docker compose -f deploy/docker/docker-compose.yml up -d
```

### Connection refused

Ensure Docker is running:
```bash
docker ps
```

See **[Troubleshooting](../reference/troubleshooting.md)** for more.
