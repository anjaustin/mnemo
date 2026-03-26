# Docker Deployment

Deploy Mnemo using Docker and Docker Compose.

For the canonical, maintained deployment docs, prefer `deploy/docker/DEPLOY.md`
and `docs/CONFIGURATION.md`. This wiki page is an overview and should follow
those source-of-truth docs.

---

## Quick Start

> **Security Note**: Always review scripts before running them.

```bash
# Review then run (recommended)
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh -o quickstart.sh
less quickstart.sh
bash quickstart.sh
```

This starts Mnemo with Redis and Qdrant, using local embeddings.

> **Warning**: The quickstart exposes ports on all interfaces. For production, see the security section below.

---

## Docker Compose

### Basic Setup

```yaml
# docker-compose.yml
version: '3.8'

services:
  mnemo:
    image: ghcr.io/anjaustin/mnemo/mnemo-server:latest
    ports:
      - "8080:8080"
    environment:
      - MNEMO_REDIS_URL=redis://redis:6379
      - MNEMO_QDRANT_URL=http://qdrant:6334
      - MNEMO_LLM_PROVIDER=none
      - MNEMO_EMBEDDING_PROVIDER=local
      - MNEMO_EMBEDDING_MODEL=AllMiniLML6V2
      - MNEMO_EMBEDDING_DIMENSIONS=384
    depends_on:
      - redis
      - qdrant

  redis:
    image: redis/redis-stack:7.4.0-v1
    ports:
      - "6379:6379"
    volumes:
      - redis_data:/data

  qdrant:
    image: qdrant/qdrant:v1.12.4
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant_data:/qdrant/storage

volumes:
  redis_data:
  qdrant_data:
```

### Start Services

```bash
docker compose up -d
```

### Verify

```bash
curl http://localhost:8080/health
# {"status":"ok","version":"0.9.0"}
```

---

## Production Configuration

### With External LLM

```yaml
services:
  mnemo:
    image: ghcr.io/anjaustin/mnemo/mnemo-server:latest
    ports:
      - "8080:8080"
    environment:
      - MNEMO_REDIS_URL=redis://redis:6379
      - MNEMO_QDRANT_URL=http://qdrant:6334
      - MNEMO_LLM_PROVIDER=anthropic
      - MNEMO_LLM_API_KEY=${MNEMO_LLM_API_KEY}
      - MNEMO_LLM_MODEL=claude-haiku-4-20250514
      - MNEMO_EMBEDDING_PROVIDER=openai
      - MNEMO_EMBEDDING_API_KEY=${MNEMO_EMBEDDING_API_KEY}
      - MNEMO_EMBEDDING_MODEL=text-embedding-3-small
      - MNEMO_AUTH_ENABLED=true
      - MNEMO_AUTH_API_KEYS=${MNEMO_AUTH_API_KEYS}
    depends_on:
      - redis
      - qdrant
    restart: unless-stopped
    deploy:
      resources:
        limits:
          memory: 512M

  redis:
    image: redis/redis-stack:7.4.0-v1
    volumes:
      - redis_data:/data
    restart: unless-stopped
    deploy:
      resources:
        limits:
          memory: 1G

  qdrant:
    image: qdrant/qdrant:v1.12.4
    volumes:
      - qdrant_data:/qdrant/storage
    restart: unless-stopped
    deploy:
      resources:
        limits:
          memory: 1G

volumes:
  redis_data:
  qdrant_data:
```

### Webhook Delivery Tuning

```yaml
services:
  mnemo:
    environment:
      - MNEMO_WEBHOOKS_ENABLED=true
      - MNEMO_WEBHOOKS_MAX_ATTEMPTS=5

# Webhook targets themselves are created via the Memory Webhooks API,
# not configured as a single global URL in compose.
```

### With S3 Blob Storage

```yaml
services:
  mnemo:
    environment:
      # ... other vars
      - BLOB_STORAGE_PROVIDER=s3
      - AWS_S3_BUCKET=mnemo-attachments
      - AWS_S3_REGION=us-east-1
      - AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID}
      - AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY}
```

---

## Resource Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| Mnemo (remote embeddings) | 256 MB | 512 MB |
| Mnemo (local embeddings) | 1.5 GB | 2 GB |
| Redis | 256 MB | 1 GB |
| Qdrant | 512 MB | 2 GB |

---

## Health Checks

Add health checks for reliability:

```yaml
services:
  # Mnemo runs in a distroless image, so probe /healthz from your orchestrator
  # or host instead of using an in-container curl healthcheck.

  redis:
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 30s
      timeout: 10s
      retries: 3

  qdrant:
    healthcheck:
      test: ["CMD-SHELL", "pidof qdrant || exit 1"]
      interval: 30s
      timeout: 10s
      retries: 3
```

---

## Persistence

### Redis Persistence

Redis Stack includes RDB and AOF persistence by default. Data is stored in `/data`.

For production, configure persistence:

```yaml
redis:
  command: >
    redis-server
    --save 60 1
    --appendonly yes
    --appendfsync everysec
  volumes:
    - redis_data:/data
```

### Qdrant Persistence

Qdrant stores data in `/qdrant/storage`. Mount a volume to persist.

### Blob Storage

For local blob storage, mount a volume:

```yaml
mnemo:
  volumes:
    - mnemo_blobs:/data/blobs
  environment:
    - BLOB_STORAGE_PATH=/data/blobs
```

---

## Networking

### Internal Network

Services communicate on an internal Docker network:

```yaml
networks:
  mnemo-net:
    driver: bridge

services:
  mnemo:
    networks:
      - mnemo-net
  redis:
    networks:
      - mnemo-net
  qdrant:
    networks:
      - mnemo-net
```

### Exposing Only Mnemo

Only expose Mnemo externally:

```yaml
services:
  mnemo:
    ports:
      - "8080:8080"

  redis:
    # No ports exposed externally
    expose:
      - "6379"

  qdrant:
    # No ports exposed externally
    expose:
      - "6333"
```

---

## Reverse Proxy

### With Traefik

```yaml
services:
  mnemo:
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.mnemo.rule=Host(`mnemo.example.com`)"
      - "traefik.http.routers.mnemo.entrypoints=websecure"
      - "traefik.http.routers.mnemo.tls.certresolver=letsencrypt"
      - "traefik.http.services.mnemo.loadbalancer.server.port=8080"
```

### With Nginx

```nginx
upstream mnemo {
    server mnemo:8080;
}

server {
    listen 443 ssl http2;
    server_name mnemo.example.com;

    ssl_certificate /etc/ssl/certs/mnemo.crt;
    ssl_certificate_key /etc/ssl/private/mnemo.key;

    location / {
        proxy_pass http://mnemo;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Do not combine `proxy_pass` and `grpc_pass` in the same `location` block. If
you expose Mnemo gRPC separately, route it with a dedicated gRPC-aware virtual
host or location.

---

## Logging

### View Logs

```bash
# All services
docker compose logs -f

# Just Mnemo
docker compose logs -f mnemo

# Last 100 lines
docker compose logs --tail=100 mnemo
```

### Log Configuration

```yaml
services:
  mnemo:
    environment:
      - RUST_LOG=info,mnemo_server=debug
      - MNEMO_LOG_FORMAT=json
    logging:
      driver: json-file
      options:
        max-size: "100m"
        max-file: "3"
```

---

## Scaling

### Multiple Mnemo Instances

```yaml
services:
  mnemo:
    deploy:
      replicas: 3

  nginx:
    image: nginx:latest
    ports:
      - "80:80"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf
    depends_on:
      - mnemo
```

---

## Backup & Restore

### Backup Redis

```bash
# Create backup
docker compose exec redis redis-cli BGSAVE
docker compose exec redis cat /data/dump.rdb > backup.rdb

# Restore
docker compose stop mnemo
docker cp backup.rdb $(docker compose ps -q redis):/data/dump.rdb
docker compose restart redis
docker compose start mnemo
```

### Backup Qdrant

```bash
# Snapshot
curl -X POST "http://localhost:6333/collections/mnemo_episodes/snapshots"

# List snapshots
curl "http://localhost:6333/collections/mnemo_episodes/snapshots"
```

---

## Upgrading

```bash
# Pull latest images
docker compose pull

# Recreate containers
docker compose up -d

# Verify
curl http://localhost:8080/health
```

---

## Troubleshooting

### Container Won't Start

```bash
# Check logs
docker compose logs mnemo

# Common issues:
# - Redis not ready: increase depends_on wait
# - Port conflict: change host port
# - Memory limit: increase limits
```

### Connection Refused

```bash
# Mnemo runs in a distroless image, so debug from the host or sibling services
docker compose logs mnemo
docker compose exec redis redis-cli ping
curl http://localhost:6333/healthz
```

### Out of Memory

```bash
# Check memory usage
docker stats

# Increase limits in docker-compose.yml
# Or switch to remote embeddings
```

---

## Next Steps

- **[Kubernetes](../../DEPLOY.md)** - Production Helm deployment
- **[Configuration](../reference/configuration.md)** - All settings
- **[Troubleshooting](../../TROUBLESHOOTING.md)** - Common issues

---

## Security Checklist

Before exposing to the internet:

- [ ] Enable authentication (`MNEMO_AUTH_ENABLED=true`)
- [ ] Use strong API keys (generate with `openssl rand -base64 32`)
- [ ] Put behind reverse proxy with TLS (nginx, traefik, caddy)
- [ ] Don't expose Redis (6379) or Qdrant (6333) ports publicly
- [ ] Use Docker secrets or env file for sensitive values
- [ ] Enable rate limiting
- [ ] Set up monitoring and alerting
