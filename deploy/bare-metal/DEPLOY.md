# Mnemo — Bare Metal / VPS Deployment

Deploy Mnemo on any Linux VPS or bare-metal server using systemd. No Docker required for `mnemo-server` — it is a single Rust binary with no runtime dependencies. Redis and Qdrant run as Docker containers (simplest, most reliable).

---

## Prerequisites

- Ubuntu 22.04+ / Debian 12+ / RHEL 9+ (or equivalent)
- 2 GB RAM minimum (4 GB recommended)
- 20 GB disk space
- `curl`, `docker`, `nginx` installed
- Root or sudo access

---

## Step 1 — Install Docker

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
# Log out and back in for the group change to take effect
```

---

## Step 2 — Start Redis and Qdrant

Redis and Qdrant run as Docker containers with named volumes for persistence.

```bash
# Redis (with RedisSearch + RedisJSON via Redis Stack)
docker run -d \
  --name mnemo-redis \
  --restart always \
  -p 127.0.0.1:6379:6379 \
  -v mnemo-redis-data:/data \
  -e REDIS_ARGS="--save 60 1 --loglevel warning" \
  redis/redis-stack:7.4.0-v1

# Qdrant (vector store)
docker run -d \
  --name mnemo-qdrant \
  --restart always \
  -p 127.0.0.1:6334:6334 \
  -v mnemo-qdrant-data:/qdrant/storage \
  -e QDRANT__SERVICE__GRPC_PORT=6334 \
  -e QDRANT__LOG_LEVEL=WARN \
  qdrant/qdrant:v1.12.4

# Verify both are running
docker ps --filter name=mnemo
```

---

## Step 3 — Download the mnemo-server binary

```bash
# Option A: use the update script (recommended — handles checksum verification)
chmod +x update.sh
sudo ./update.sh           # latest release
# sudo ./update.sh 0.3.1  # specific version

# Option B: manual download
VERSION=0.3.1
ARCH=x86_64-unknown-linux-gnu  # or aarch64-unknown-linux-gnu
curl -fsSL -o /tmp/mnemo-server \
  "https://github.com/anjaustin/mnemo/releases/download/v${VERSION}/mnemo-server-${ARCH}"
chmod +x /tmp/mnemo-server
sudo mv /tmp/mnemo-server /usr/local/bin/mnemo-server
```

Verify the binary:

```bash
mnemo-server --version
```

---

## Step 4 — Create the mnemo user and directories

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin mnemo
sudo mkdir -p /var/lib/mnemo /var/log/mnemo /etc/mnemo
sudo chown mnemo:mnemo /var/lib/mnemo /var/log/mnemo
```

---

## Step 5 — Configure environment

```bash
sudo cp /path/to/deploy/docker/.env.example /etc/mnemo/mnemo.env
sudo $EDITOR /etc/mnemo/mnemo.env
```

Key variables to set in `/etc/mnemo/mnemo.env`:

```dotenv
# Storage — point at the local Docker containers
MNEMO_REDIS_URL=redis://127.0.0.1:6379
MNEMO_QDRANT_URL=http://127.0.0.1:6334

# Server — bind to loopback; nginx handles public exposure
MNEMO_SERVER_HOST=127.0.0.1
MNEMO_SERVER_PORT=8080

# LLM (optional)
MNEMO_LLM_PROVIDER=openai
MNEMO_LLM_API_KEY=sk-...
MNEMO_LLM_MODEL=gpt-4o-mini
MNEMO_EMBEDDING_API_KEY=sk-...
MNEMO_EMBEDDING_MODEL=text-embedding-3-small

# Auth — enable before going public
MNEMO_AUTH_ENABLED=false

# Logging
RUST_LOG=mnemo=info
```

Secure the file:

```bash
sudo chmod 600 /etc/mnemo/mnemo.env
sudo chown root:mnemo /etc/mnemo/mnemo.env
```

---

## Step 6 — Install the systemd unit

```bash
sudo cp mnemo.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now mnemo
```

Check status:

```bash
sudo systemctl status mnemo
journalctl -u mnemo -f
```

---

## Step 7 — Configure nginx

```bash
sudo apt-get install -y nginx

# Copy the reference config
sudo cp nginx.conf /etc/nginx/sites-available/mnemo

# Edit to set your domain
sudo $EDITOR /etc/nginx/sites-available/mnemo
# Replace 'your.domain.example' with your actual domain

sudo ln -s /etc/nginx/sites-available/mnemo /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

---

## Step 8 — TLS with Let's Encrypt

```bash
sudo apt-get install -y certbot python3-certbot-nginx
sudo certbot --nginx -d your.domain.example
# certbot will auto-update the nginx config and add a cron renewal job
```

---

## Step 9 — Verify

```bash
# Health via loopback
curl http://127.0.0.1:8080/health
# Expected: {"status":"ok","version":"0.3.1"}

# Health via domain (after nginx + TLS)
curl https://your.domain.example/health

# Write a memory
curl -s -X POST https://your.domain.example/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","content":"Bare metal deploy works"}'

# Read context
curl -s -X POST https://your.domain.example/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"deploy","limit":5}'

# Persistence test: restart and repeat context query
sudo systemctl restart mnemo
curl -s -X POST https://your.domain.example/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"deploy","limit":5}'
# Should return same result as before restart
```

---

## Updating

Use the update script:

```bash
sudo ./update.sh            # update to latest
sudo ./update.sh 0.4.0     # update to specific version
```

Or manually:

```bash
sudo systemctl stop mnemo
sudo cp /tmp/mnemo-server-new /usr/local/bin/mnemo-server
sudo systemctl start mnemo
```

---

## Troubleshooting

| Symptom | Command | Notes |
|---|---|---|
| Service won't start | `journalctl -u mnemo -n 100` | Check for missing env vars or port conflicts |
| Redis unreachable | `docker logs mnemo-redis` | Ensure container is up; check `MNEMO_REDIS_URL` |
| Qdrant unreachable | `docker logs mnemo-qdrant` | Ensure container is up; check `MNEMO_QDRANT_URL` |
| Port 8080 conflict | `ss -tlnp \| grep 8080` | Another process using the port |
| nginx 502 Bad Gateway | `curl http://127.0.0.1:8080/health` | Mnemo is down; check service status |
| Permission denied on env file | `ls -la /etc/mnemo/mnemo.env` | Must be readable by mnemo user |
