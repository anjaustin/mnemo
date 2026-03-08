# Mnemo — Render Deployment

[Render](https://render.com) supports Docker-based services with private networking. Mnemo runs as three services: mnemo-server (public web service), Redis Stack (private service), and Qdrant (private service).

> **Why not Render's managed Key Value (Redis)?** Mnemo requires Redis Stack modules (RedisSearch, RedisJSON). Render's managed Key Value is plain Redis without Stack modules. We run `redis/redis-stack-server` as a private Docker service instead.

---

## What Gets Created

| Service | Type | Render Type | Notes |
|---|---|---|---|
| `mnemo` | API server | `web` (public) | Distroless image, port 8080, health check at `/health` |
| `mnemo-redis` | Redis Stack | `pserv` (private) | Persistent disk, AOF + RDB, not internet-reachable |
| `mnemo-qdrant` | Qdrant vector DB | `pserv` (private) | Persistent disk, gRPC on 6334, not internet-reachable |

**Cost estimate:** Starter plan ~$7/month per service x 3 + disks = ~$25–35/month. Free tier is not suitable (private services and persistent disks are paid).

---

## Deploy via Blueprint

1. Fork [github.com/anjaustin/mnemo](https://github.com/anjaustin/mnemo)
2. In Render dashboard, click **New** → **Blueprint**
3. Connect your forked repo
4. Set the **Blueprint path** to `deploy/render/render.yaml`
5. Set secret env vars when prompted:
   - `MNEMO_LLM_API_KEY` — your Anthropic (or other supported provider) API key
   - `MNEMO_AUTH_API_KEYS` — comma-separated API keys for auth (optional)
6. Click **Apply**

Render will create all three services, attach persistent disks, and wire up internal networking automatically.

---

## Manual Deploy

If not using Blueprint:

1. **Create Redis Stack (private service)**:
   - New → Private Service → Docker image `redis/redis-stack-server:7.4.0-v1`
   - Add persistent disk at `/data` (10 GB)
   - Set env: `REDIS_ARGS=--save 60 1 --appendonly yes --loglevel warning`

2. **Create Qdrant (private service)**:
   - New → Private Service → Docker image `qdrant/qdrant:v1.12.4`
   - Add persistent disk at `/qdrant/storage` (10 GB)
   - Set env: `QDRANT__SERVICE__GRPC_PORT=6334`, `QDRANT__LOG_LEVEL=WARN`

3. **Create Mnemo (web service)**:
   - New → Web Service → Docker image `ttl.sh/mnemo-local-embed-distroless-fixed-20260307:24h`
   - Health check path: `/health`
   - Set env vars (see below)

---

## Environment Variables for `mnemo` service

```bash
MNEMO_SERVER_HOST=0.0.0.0
MNEMO_SERVER_PORT=8080
MNEMO_REDIS_URL=redis://mnemo-redis:6379      # Render private network hostname
MNEMO_QDRANT_URL=http://mnemo-qdrant:6334      # Render private network hostname
MNEMO_QDRANT_PREFIX=mnemo_render_384_
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

# Auth (recommended for production)
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=your-key-here
```

---

## Verify

Render provides a public HTTPS URL for the `mnemo` web service:

```bash
curl https://mnemo-xxxx.onrender.com/health
# Expected: {"status":"ok","version":"0.3.7"}
```

---

## Networking

- Redis and Qdrant run as **private services** — they are only reachable from other Render services in the same region via the private network.
- Service hostnames on the private network match the service name (e.g., `mnemo-redis`, `mnemo-qdrant`).
- No public ports are exposed for Redis or Qdrant.

---

## Tear Down

To delete all resources:
1. Go to the Render dashboard
2. Delete each service (mnemo, mnemo-redis, mnemo-qdrant)
3. Persistent disks are deleted with their services

---

## Notes

- Render's Starter plan web services spin down after 15 minutes of inactivity on free tier. Use paid Starter plan or higher for production.
- Persistent disks disable zero-downtime deploys on Render. Deploys will have brief downtime.
- All three services must be in the **same region** (default: `oregon`) for private networking to work.
- The `render.yaml` blueprint uses `sync: false` for secret vars — you'll be prompted to set them on initial deploy only. For updates, set them manually in the dashboard.
