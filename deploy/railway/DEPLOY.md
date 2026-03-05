# Mnemo — Railway Deployment

[Railway](https://railway.app) supports multi-service Docker deployments. Mnemo runs as three services: mnemo-server, Redis, and Qdrant.

---

## What Gets Created

| Service | Image | Notes |
|---|---|---|
| `mnemo` | `ghcr.io/anjaustin/mnemo/mnemo-server:latest` | Main API server |
| `redis` | Railway Redis plugin | Managed, persistent |
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
2. Add a **Redis** plugin (Railway managed Redis)
3. Add a new service → **Docker Image** → `ghcr.io/anjaustin/mnemo/mnemo-server:latest`
4. Add a new service → **Docker Image** → `qdrant/qdrant:v1.12.4`
5. Wire env vars between services (see below)

---

## Environment Variables for `mnemo` service

```bash
MNEMO_REDIS_URL=redis://${{Redis.RAILWAY_TCP_PROXY_DOMAIN}}:${{Redis.RAILWAY_TCP_PROXY_PORT}}
MNEMO_QDRANT_URL=http://${{qdrant.RAILWAY_PRIVATE_DOMAIN}}:6334
MNEMO_SERVER_HOST=0.0.0.0
MNEMO_SERVER_PORT=8080
MNEMO_WEBHOOKS_ENABLED=true
RUST_LOG=mnemo=info

# LLM (optional)
MNEMO_LLM_PROVIDER=openai
MNEMO_LLM_API_KEY=sk-...
MNEMO_LLM_MODEL=gpt-4o-mini

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
# Expected: {"status":"ok","version":"0.3.3"}
```

---

## Notes

- Railway's managed Redis supports the modules Mnemo needs (RedisSearch, RedisJSON are part of Redis Stack). If using the basic Redis plugin, verify compatibility or use a Redis Stack image instead.
- Qdrant on Railway requires a persistent volume attached to `/qdrant/storage` to survive deploys.
- Railway private networking (`RAILWAY_PRIVATE_DOMAIN`) keeps Redis and Qdrant off the public internet.
