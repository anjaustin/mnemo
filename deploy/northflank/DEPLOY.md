# Mnemo — Northflank Deployment

[Northflank](https://northflank.com) supports multi-service Docker stacks with persistent volumes. Mnemo deploys as a three-service stack via the Northflank stack definition.

---

## What Gets Created

| Service | Image | Notes |
|---|---|---|
| `mnemo-redis` | `redis/redis-stack-server:7.4.0-v1` | 10 GB SSD volume at `/data`; uses `REDIS_ARGS` env var |
| `mnemo-qdrant` | `qdrant/qdrant:v1.12.4` | 20 GB SSD volume at `/qdrant/storage` |
| `mnemo-server` | `ghcr.io/anjaustin/mnemo/mnemo-server:latest` | Public HTTP port 8080; local embeddings enabled |

**Cost estimate:** ~$20–$40/month depending on compute plan.

---

## Deploy via Northflank CLI

```bash
# Install Northflank CLI
npm install -g @northflank/cli

# Log in
northflank login

# Create a project
northflank create project --name mnemo

# Deploy the stack
northflank create stack \
  --project mnemo \
  --file deploy/northflank/stack.json
```

---

## Deploy via Dashboard

1. Log in to [app.northflank.com](https://app.northflank.com)
2. Create a new project → **Mnemo**
3. Go to **Stacks** → **New Stack** → paste contents of `deploy/northflank/stack.json`
4. Add secret env vars for `mnemo-server` (LLM key, auth keys)
5. Click **Deploy**

---

## Add Secret Environment Variables

After stack creation, add these to `mnemo-server`:

```bash
# LLM
MNEMO_LLM_PROVIDER=anthropic
MNEMO_LLM_API_KEY=sk-ant-...
MNEMO_LLM_MODEL=claude-haiku-4-20250514

# Embedding
MNEMO_EMBEDDING_PROVIDER=local
MNEMO_EMBEDDING_MODEL=AllMiniLML6V2
MNEMO_EMBEDDING_DIMENSIONS=384
MNEMO_QDRANT_PREFIX=mnemo_nf_384_
MNEMO_SESSION_SUMMARY_THRESHOLD=10

# Auth (recommended)
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=your-key-here
```

---

## Verify

Northflank provides a public URL for the `mnemo-server` service. Test it:

```bash
curl https://mnemo-server-xxxx.northflank.app/health
# Expected: {"status":"ok","version":"0.3.7"}

curl -s -X POST https://mnemo-server-xxxx.northflank.app/api/v1/memory \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-key-here" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on Northflank"}'
```

---

## Notes

- Internal service hostnames use short names within the same project: `mnemo-redis` and `mnemo-qdrant`. External DNS follows the pattern `<portname>--<servicename>--<namespace>.code.run`.
- **Must use `redis/redis-stack-server`** (not `redis/redis-stack` with a custom command). The `-stack` image's custom entrypoint loads RedisSearch/RedisJSON modules; overriding the command with `redis-server ...` bypasses module loading. Pass persistence args via the `REDIS_ARGS` env var instead.
- Persistent volumes are managed by Northflank and survive service restarts and redeployments.
- The `nf-compute-10` plan is the entry-level Northflank compute tier. Upgrade if you need more RAM for high-volume workloads.
- Cluster used during falsification: `nf-us-east-ohio`, namespace `ns-blcxq2rhfzbr`.
