# Mnemo — Railway Deployment

[Railway](https://railway.app) supports multi-service Docker deployments. Mnemo runs as three services: mnemo-server, Redis, and Qdrant.

---

## What Gets Created

| Service | Image | Notes |
|---|---|---|
| `mnemo` | `ghcr.io/anjaustin/mnemo/mnemo-server:latest` | Main API server; local embeddings enabled |
| `redis` | `redis/redis-stack-server:7.4.0-v1` | Docker image service; Redis Stack modules required |
| `qdrant` | `qdrant/qdrant:v1.12.4` | Self-hosted on Railway |

**Cost estimate:** ~$5–$20/month depending on usage (Railway charges by resource consumption).

---

## Deploy

### Option A — From Template (recommended when available)

1. Go to [railway.app/new](https://railway.app/new)
2. Search for **Mnemo** in the template marketplace (once submitted)
3. Click **Deploy** and set env vars

### Option B — Manual

1. Create a new Railway project
2. Add a new service → **Docker Image** → `redis/redis-stack-server:7.4.0-v1`
3. Add a new service → **Docker Image** → `qdrant/qdrant:v1.12.4`
4. Add a new service → **Docker Image** → `ghcr.io/anjaustin/mnemo/mnemo-server:latest`
5. Wire env vars between services (see below)
6. Generate a Railway domain for the `mnemo` service (`*.up.railway.app`)

---

## Environment Variables for `mnemo` service

```bash
MNEMO_REDIS_URL=redis://${{Redis.RAILWAY_PRIVATE_DOMAIN}}:6379
MNEMO_QDRANT_URL=http://${{Qdrant.RAILWAY_PRIVATE_DOMAIN}}:6334
MNEMO_QDRANT_PREFIX=mnemo_rail_384_
MNEMO_SERVER_HOST=0.0.0.0
MNEMO_SERVER_PORT=8080
MNEMO_SESSION_SUMMARY_THRESHOLD=10
MNEMO_WEBHOOKS_ENABLED=true
RUST_LOG=mnemo=info

# LLM
MNEMO_LLM_PROVIDER=anthropic
MNEMO_LLM_API_KEY=sk-ant-...
MNEMO_LLM_MODEL=claude-haiku-4-20250514

# Embedding
MNEMO_EMBEDDING_PROVIDER=local
MNEMO_EMBEDDING_MODEL=AllMiniLML6V2
MNEMO_EMBEDDING_DIMENSIONS=384

# Auth (recommended)
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=your-key-here
```

---

## Environment Variables for `qdrant` service

```bash
QDRANT__SERVICE__GRPC_PORT=6334
QDRANT__LOG_LEVEL=WARN
```

Add a persistent volume at `/qdrant/storage` for the qdrant service.

---

## Verify

Railway provides a public URL for the `mnemo` service. Test it:

```bash
curl https://your-railway-url.up.railway.app/health
# Expected: {"status":"ok","version":"0.4.0"}
```

---

## Notes

- Railway project tokens use the `Project-Access-Token` header, not `Authorization: Bearer`, when automating deploys via GraphQL.
- Railway does not always auto-generate a public domain for image services; create one explicitly with `serviceDomainCreate` or in the dashboard.
- Use `redis/redis-stack-server` instead of the managed Redis plugin when you need guaranteed Redis Stack module availability.
- Qdrant on Railway requires a persistent volume attached to `/qdrant/storage` to survive deploys.
- Railway private networking (`RAILWAY_PRIVATE_DOMAIN`) keeps Redis and Qdrant off the public internet.
