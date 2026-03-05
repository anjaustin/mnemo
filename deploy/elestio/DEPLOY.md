# Mnemo — Elestio Deployment

[Elestio](https://elestio.app) is a managed open-source hosting platform. You can deploy any Docker Compose application without managing infrastructure.

---

## What Gets Created

Elestio manages the VM, TLS, backups, and monitoring. You provide the compose file and env vars.

| Resource | Notes |
|---|---|
| VM | Elestio selects based on service requirements |
| TLS | Automatic via Elestio |
| Backups | Configurable in Elestio dashboard |
| Cost | Starts ~$10–$20/month depending on VM tier |

---

## Deploy via Elestio Dashboard

1. Log in to [elestio.app](https://elestio.app)
2. Click **Deploy a new service** → **Custom Docker Compose**
3. Paste the contents of `deploy/elestio/docker-compose.yml`
4. Set environment variables (see below)
5. Click **Deploy**

---

## Environment Variables

Set these in the Elestio service environment panel:

```bash
# Required
MNEMO_REDIS_URL=redis://redis:6379
MNEMO_QDRANT_URL=http://qdrant:6334

# LLM (optional)
MNEMO_LLM_PROVIDER=openai
MNEMO_LLM_API_KEY=sk-...
MNEMO_LLM_MODEL=gpt-4o-mini
MNEMO_EMBEDDING_API_KEY=sk-...
MNEMO_EMBEDDING_MODEL=text-embedding-3-small

# Auth (recommended before public exposure)
MNEMO_AUTH_ENABLED=true
MNEMO_AUTH_API_KEYS=your-key-here

# Server
MNEMO_SERVER_HOST=0.0.0.0
MNEMO_SERVER_PORT=8080
MNEMO_WEBHOOKS_ENABLED=true
RUST_LOG=mnemo=info
```

---

## Verify

Once deployed, Elestio provides a public URL. Test it:

```bash
curl https://your-elestio-url/health
# Expected: {"status":"ok","version":"0.3.2"}

curl -s -X POST https://your-elestio-url/api/v1/memory \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-key-here" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on Elestio"}'
```

---

## Notes

- Elestio maps external HTTPS to container port 9000 in the compose file by default. Adjust `0.0.0.0:9000:8080` to match Elestio's port forwarding convention for your deployment.
- Elestio handles TLS termination — Mnemo runs HTTP internally.
- Persistent volumes are managed by Elestio; data survives restarts.
