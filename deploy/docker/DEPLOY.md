# Mnemo — Docker Deployment

Deploy the full Mnemo stack (mnemo-server + Redis + Qdrant) on any host with Docker installed.

---

## Prerequisites

- Docker Engine 24+ with Compose v2 (`docker compose version`)
- 2 GB RAM minimum (4 GB recommended for production)
- 20 GB disk space

---

## Option A — All-in-One (self-hosted Redis + Qdrant)

Runs all three services on the same host. Volumes persist across restarts.

### 1. Configure

```bash
cp .env.example .env
$EDITOR .env
```

Required variables to review:

| Variable | Default | Notes |
|---|---|---|
| `MNEMO_VERSION` | `latest` | Pin to `0.3.7` (or a newer release) for reproducibility |
| `MNEMO_SERVER_PORT` | `8080` | Host port Mnemo listens on |
| `MNEMO_LLM_API_KEY` | _(empty)_ | OpenAI/Anthropic key; leave blank to skip enrichment |
| `MNEMO_AUTH_ENABLED` | `false` | Set `true` + `MNEMO_AUTH_API_KEYS` before public exposure |

### 2. Start

```bash
docker compose -f docker-compose.prod.yml up -d
```

### 3. Verify

```bash
# Health check
curl http://localhost:8080/health
# Expected: {"status":"ok","version":"0.3.7"}

# Write a memory
curl -s -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","text":"Mnemo deployed successfully"}'

# Read context
curl -s -X POST http://localhost:8080/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"deployment","limit":5}'
```

### 4. Logs

```bash
docker compose -f docker-compose.prod.yml logs -f mnemo
```

### 5. Stop / restart

```bash
docker compose -f docker-compose.prod.yml restart mnemo   # rolling restart
docker compose -f docker-compose.prod.yml down            # stop (volumes preserved)
docker compose -f docker-compose.prod.yml down -v         # stop + DELETE volumes
```

---

## Option B — Managed Services (external Redis + Qdrant)

Run only `mnemo-server`. Redis and Qdrant are hosted externally (Upstash, Redis Cloud, Qdrant Cloud, etc.).

### Managed service recommendations

| Service | Provider | Notes |
|---|---|---|
| Redis | [Upstash](https://upstash.com) | Redis Stack; free tier available |
| Redis | [Redis Cloud](https://redis.io/cloud) | Redis Stack; free 30 MB |
| Qdrant | [Qdrant Cloud](https://cloud.qdrant.io) | Free 1 GB cluster |

> Redis must support **RedisSearch** and **RedisJSON** modules (Redis Stack). Standard Redis without modules will not work.

### 1. Configure

```bash
cp .env.example .env
$EDITOR .env
```

Set:

```dotenv
MNEMO_REDIS_URL=redis://:password@your-redis-host:6379
MNEMO_QDRANT_URL=https://your-cluster.cloud.qdrant.io:6334
```

### 2. Start

```bash
docker compose -f docker-compose.managed.yml up -d
```

### 3. Verify (same as Option A)

```bash
curl http://localhost:8080/health
```

---

## Reverse Proxy (nginx + TLS)

For production, put nginx in front of Mnemo for TLS termination.

### Install nginx and certbot

```bash
apt-get install -y nginx certbot python3-certbot-nginx
```

### nginx site config

Create `/etc/nginx/sites-available/mnemo`:

```nginx
server {
    listen 80;
    server_name your.domain.example;

    proxy_connect_timeout 60s;
    proxy_send_timeout    120s;
    proxy_read_timeout    120s;

    location / {
        proxy_pass         http://127.0.0.1:8080;
        proxy_set_header   Host $host;
        proxy_set_header   X-Real-IP $remote_addr;
        proxy_set_header   X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
    }
}
```

```bash
ln -s /etc/nginx/sites-available/mnemo /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx

# Get TLS certificate
certbot --nginx -d your.domain.example
```

After certbot, access Mnemo at `https://your.domain.example/health`.

---

## Upgrading

```bash
# Pull new image
docker compose -f docker-compose.prod.yml pull mnemo

# Restart with new image (zero-downtime if behind a load balancer)
docker compose -f docker-compose.prod.yml up -d mnemo
```

---

## Persistence and Backup

Data lives in Docker named volumes:

```bash
docker volume ls | grep mnemo
# mnemo_redis-data
# mnemo_qdrant-data
```

To back up:

```bash
# Redis: trigger an RDB snapshot, then copy
docker exec mnemo-redis redis-cli BGSAVE
docker run --rm -v mnemo_redis-data:/data -v $(pwd):/backup \
  alpine tar czf /backup/redis-backup-$(date +%Y%m%d).tar.gz /data

# Qdrant: snapshot via API, then copy
curl -X POST http://localhost:6333/collections/your_collection/snapshots
docker run --rm -v mnemo_qdrant-data:/qdrant/storage -v $(pwd):/backup \
  alpine tar czf /backup/qdrant-backup-$(date +%Y%m%d).tar.gz /qdrant/storage
```

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `Connection refused` on port 8080 | Container not started | `docker compose ps` — check state |
| `{"error":...}` on memory write | Redis or Qdrant not healthy | `docker compose logs redis qdrant` |
| Mnemo exits immediately | Missing required env var | Check `MNEMO_REDIS_URL`, `MNEMO_QDRANT_URL` |
| Redis auth failure | `REDIS_PASSWORD` mismatch | Ensure same value in `.env` as when volumes were created |
| Out of memory | Resource limits too tight | Increase `deploy.resources.limits.memory` or host RAM |
