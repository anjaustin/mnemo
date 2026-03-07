#!/bin/bash
exec > /var/log/mnemo-init.log 2>&1
set -eux

# ── Wait for apt lock (cloud-init may still be running) ─────────────────────
for i in $(seq 1 30); do
  flock -n /var/lib/dpkg/lock-frontend true 2>/dev/null && break || sleep 5
done

# ── Install Docker ───────────────────────────────────────────────────────────
apt-get update -qq
apt-get install -y -qq ca-certificates curl gnupg lsb-release

install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
chmod a+r /etc/apt/keyrings/docker.gpg

echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] \
  https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" \
  > /etc/apt/sources.list.d/docker.list

apt-get update -qq
apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin

systemctl enable --now docker

# ── App directories ──────────────────────────────────────────────────────────
mkdir -p /data/redis /data/qdrant /opt/mnemo

# ── Environment file ─────────────────────────────────────────────────────────
cat > /opt/mnemo/.env <<'ENVEOF'
MNEMO_VERSION=${mnemo_version}
MNEMO_IMAGE=${mnemo_image}
MNEMO_SERVER_PORT=8080
MNEMO_LLM_PROVIDER=${mnemo_llm_provider}
MNEMO_LLM_API_KEY=${mnemo_llm_api_key}
MNEMO_LLM_MODEL=${mnemo_llm_model}
MNEMO_EMBEDDING_PROVIDER=${mnemo_embedding_provider}
MNEMO_EMBEDDING_API_KEY=${mnemo_embedding_api_key}
MNEMO_EMBEDDING_MODEL=${mnemo_embedding_model}
MNEMO_EMBEDDING_DIMENSIONS=${mnemo_embedding_dimensions}
MNEMO_QDRANT_PREFIX=${mnemo_qdrant_prefix}
MNEMO_SESSION_SUMMARY_THRESHOLD=${mnemo_session_summary_threshold}
MNEMO_AUTH_ENABLED=${mnemo_auth_enabled}
MNEMO_AUTH_API_KEYS=${mnemo_auth_api_keys}
MNEMO_WEBHOOKS_ENABLED=true
MNEMO_WEBHOOKS_MAX_ATTEMPTS=3
RUST_LOG=mnemo=info
ENVEOF
chmod 600 /opt/mnemo/.env

# ── Docker Compose file ──────────────────────────────────────────────────────
MNEMO_IMAGE=$(grep '^MNEMO_IMAGE=' /opt/mnemo/.env | cut -d= -f2-)
cat > /opt/mnemo/docker-compose.yml <<COMPEOF
services:
  redis:
    image: redis/redis-stack-server:7.4.0-v1
    container_name: mnemo-redis
    restart: always
    ports:
      - "127.0.0.1:6379:6379"
    volumes:
      - /data/redis:/data
    environment:
      - REDIS_ARGS=--save 60 1 --appendonly yes --loglevel warning
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 10s
  qdrant:
    image: qdrant/qdrant:v1.12.4
    container_name: mnemo-qdrant
    restart: always
    ports:
      - "127.0.0.1:6334:6334"
    volumes:
      - /data/qdrant:/qdrant/storage
    environment:
      - QDRANT__SERVICE__GRPC_PORT=6334
      - QDRANT__LOG_LEVEL=WARN
    healthcheck:
      test: ["CMD-SHELL", "pidof qdrant || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 15s
  mnemo:
    image: $MNEMO_IMAGE
    container_name: mnemo-server
    restart: always
    ports:
      - "8080:8080"
    env_file:
      - /opt/mnemo/.env
    environment:
      - MNEMO_SERVER_HOST=0.0.0.0
      - MNEMO_SERVER_PORT=8080
      - MNEMO_REDIS_URL=redis://redis:6379
      - MNEMO_QDRANT_URL=http://qdrant:6334
    depends_on:
      redis:
        condition: service_healthy
      qdrant:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 15s
      timeout: 5s
      retries: 3
      start_period: 20s
COMPEOF

# ── Start the stack ───────────────────────────────────────────────────────────
cd /opt/mnemo
docker compose up -d

echo "Mnemo init complete."
