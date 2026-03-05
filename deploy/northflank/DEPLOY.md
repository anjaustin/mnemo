# Mnemo — Northflank Deployment

[Northflank](https://northflank.com) supports multi-service Docker stacks with persistent volumes. Mnemo deploys as a three-service stack via the Northflank stack definition.

---

## What Gets Created

| Service | Image | Notes |
|---|---|---|
| `mnemo-redis` | `redis/redis-stack:7.4.0-v1` | 10 GB SSD volume at `/data` |
| `mnemo-qdrant` | `qdrant/qdrant:v1.12.4` | 20 GB SSD volume at `/qdrant/storage` |
| `mnemo-server` | `ghcr.io/anjaustin/mnemo/mnemo-server:latest` | Public HTTP port 8080 |

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
# LLM (optional)
MNEMO_LLM_PROVIDER=openai
MNEMO_LLM_API_KEY=sk-...
MNEMO_LLM_MODEL=gpt-4o-mini
MNEMO_EMBEDDING_API_KEY=sk-...

# Auth (recommended)
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=your-key-here
```

---

## Verify

Northflank provides a public URL for the `mnemo-server` service. Test it:

```bash
curl https://mnemo-server-xxxx.northflank.app/health
# Expected: {"status":"ok","version":"0.3.2"}

curl -s -X POST https://mnemo-server-xxxx.northflank.app/api/v1/memory \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-key-here" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on Northflank"}'
```

---

## Notes

- Internal service hostnames follow Northflank's pattern (service name within a project). The stack definition uses `mnemo-redis` and `mnemo-qdrant` as internal hostnames.
- Persistent volumes are managed by Northflank and survive service restarts and redeployments.
- The `nf-compute-10` plan is the entry-level Northflank compute tier. Upgrade if you need more RAM for high-volume workloads.
