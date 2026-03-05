# Mnemo — Render Deployment

[Render](https://render.com) supports Docker-based services with managed Redis. Mnemo runs as three services: mnemo-server, Redis (Render managed), and Qdrant (self-hosted on Render).

---

## What Gets Created

| Service | Type | Notes |
|---|---|---|
| `mnemo` | Web service (Docker image) | Main API server |
| `mnemo-redis` | Redis (Render managed) | Persistent, managed |
| `mnemo-qdrant` | Web service (Docker image) | Qdrant with persistent disk |

**Cost estimate:** Starter plan ~$14/month (mnemo-server) + ~$10/month (Redis) + ~$7/month (qdrant) = ~$31/month minimum. Free tier is not suitable (Redis and persistent disk are paid).

---

## Deploy via Blueprint

1. Fork [github.com/anjaustin/mnemo](https://github.com/anjaustin/mnemo)
2. In Render dashboard, click **New** → **Blueprint**
3. Connect your forked repo
4. Render detects `deploy/render/render.yaml`
5. Set secret env vars when prompted (LLM key, auth keys)
6. Click **Apply**

---

## Manual Deploy

If not using Blueprint:

1. **Create Redis**: New → Redis → Starter plan → note the connection string
2. **Create Qdrant**: New → Web Service → Docker image `qdrant/qdrant:v1.12.4` → add persistent disk at `/qdrant/storage`
3. **Create Mnemo**: New → Web Service → Docker image `ghcr.io/anjaustin/mnemo/mnemo-server:latest`
   - Set env vars (see below)
   - Health check path: `/health`

---

## Environment Variables for `mnemo` service

```bash
MNEMO_SERVER_HOST=0.0.0.0
MNEMO_SERVER_PORT=8080
MNEMO_REDIS_URL=<from Render Redis connection string>
MNEMO_QDRANT_URL=http://mnemo-qdrant:6334  # Render internal hostname
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

## Verify

Render provides a public HTTPS URL. Test it:

```bash
curl https://mnemo-xxxx.onrender.com/health
# Expected: {"status":"ok","version":"0.3.2"}
```

---

## Notes

- Render's managed Redis is a standard Redis instance. Mnemo requires **Redis Stack** modules (RedisSearch, RedisJSON). If Render's managed Redis doesn't include Stack modules, use a Redis Stack Docker image as a separate web service instead.
- Qdrant requires a persistent disk on Render (not included in free tier).
- Render free tier web services spin down after inactivity — use paid plans for production.
- The `render.yaml` blueprint uses `sync: false` for secret vars — you'll be prompted to set them on deploy.
