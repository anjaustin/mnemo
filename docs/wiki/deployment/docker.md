# Docker Deployment

Deploy Mnemo using Docker and Docker Compose.

---

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
```

This starts Mnemo with Redis and Qdrant, using local embeddings.

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
      - REDIS_URL=redis://redis:6379
      - QDRANT_URL=http://qdrant:6333
      - EMBEDDING_PROVIDER=fastembed
    depends_on:
      - redis
      - qdrant

  redis:
    image: redis/redis-stack:latest
    ports:
      - "6379:6379"
    volumes:
      - redis_data:/data

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
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
# {"status":"healthy","version":"0.9.0"}
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
      - REDIS_URL=redis://redis:6379
      - QDRANT_URL=http://qdrant:6333
      - LLM_PROVIDER=anthropic
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      - EMBEDDING_PROVIDER=openai
      - OPENAI_API_KEY=${OPENAI_API_KEY}
      - MNEMO_AUTH_ENABLED=true
      - MNEMO_AUTH_ADMIN_KEY=${MNEMO_ADMIN_KEY}
    depends_on:
      - redis
      - qdrant
    restart: unless-stopped
    deploy:
      resources:
        limits:
          memory: 512M

  redis:
    image: redis/redis-stack:latest
    volumes:
      - redis_data:/data
    restart: unless-stopped
    deploy:
      resources:
        limits:
          memory: 1G

  qdrant:
    image: qdrant/qdrant:latest
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

### With Webhooks

```yaml
services:
  mnemo:
    environment:
      # ... other vars
      - MNEMO_WEBHOOK_ENABLED=true
      - MNEMO_WEBHOOK_URL=https://hooks.example.com/mnemo
      - MNEMO_WEBHOOK_SECRET=${WEBHOOK_SECRET}
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
  mnemo:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 10s

  redis:
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 30s
      timeout: 10s
      retries: 3

  qdrant:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:6333/healthz"]
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
        
        # For gRPC
        grpc_pass grpc://mnemo;
    }
}
```

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
# Check network
docker compose exec mnemo ping redis
docker compose exec mnemo ping qdrant

# Check ports
docker compose exec mnemo curl http://redis:6379
docker compose exec mnemo curl http://qdrant:6333/healthz
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

- **[Kubernetes](kubernetes.md)** - Production Helm deployment
- **[Configuration](../reference/configuration.md)** - All settings
- **[Troubleshooting](../reference/troubleshooting.md)** - Common issues
