#!/usr/bin/env bash
# Mnemo quickstart — pulls the pre-built image and starts the full stack.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/anomalyco/mnemo/main/scripts/quickstart.sh | bash
# or:
#   MNEMO_LLM_API_KEY=sk-... bash scripts/quickstart.sh
set -euo pipefail

MNEMO_PORT="${MNEMO_PORT:-8080}"
MNEMO_LLM_PROVIDER="${MNEMO_LLM_PROVIDER:-anthropic}"
MNEMO_LLM_MODEL="${MNEMO_LLM_MODEL:-claude-haiku-4-20250514}"

echo "==> Mnemo quickstart"

# Check dependencies
for cmd in docker curl; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "ERROR: '$cmd' not found. Install it and retry."
    exit 1
  fi
done

if ! docker compose version &>/dev/null; then
  echo "ERROR: 'docker compose' (v2) not found. Install Docker Desktop >= 4.x."
  exit 1
fi

# Warn if no LLM key (optional — local embedder works without it)
if [[ -z "${MNEMO_LLM_API_KEY:-}" ]]; then
  echo "WARN: MNEMO_LLM_API_KEY is not set."
  echo "      Entity extraction and memory digests require an LLM key."
  echo "      Set it: export MNEMO_LLM_API_KEY=sk-..."
  echo ""
fi

# Determine compose file location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
COMPOSE_FILE="${REPO_ROOT}/docker-compose.yml"

if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "ERROR: docker-compose.yml not found at $COMPOSE_FILE"
  exit 1
fi

echo "==> Pulling latest Mnemo images..."
MNEMO_LLM_API_KEY="${MNEMO_LLM_API_KEY:-}" \
MNEMO_LLM_PROVIDER="$MNEMO_LLM_PROVIDER" \
MNEMO_LLM_MODEL="$MNEMO_LLM_MODEL" \
MNEMO_PORT="$MNEMO_PORT" \
  docker compose -f "$COMPOSE_FILE" pull --quiet

echo "==> Starting stack..."
MNEMO_LLM_API_KEY="${MNEMO_LLM_API_KEY:-}" \
MNEMO_LLM_PROVIDER="$MNEMO_LLM_PROVIDER" \
MNEMO_LLM_MODEL="$MNEMO_LLM_MODEL" \
MNEMO_PORT="$MNEMO_PORT" \
  docker compose -f "$COMPOSE_FILE" up -d

echo ""
echo "==> Waiting for Mnemo to be healthy..."
for i in $(seq 1 30); do
  if curl -sf "http://localhost:${MNEMO_PORT}/health" &>/dev/null; then
    echo "==> Mnemo is up!"
    break
  fi
  if [[ $i -eq 30 ]]; then
    echo "ERROR: Mnemo did not become healthy in 30s."
    echo "       Check logs: docker compose logs mnemo"
    exit 1
  fi
  sleep 1
done

echo ""
echo "  Mnemo API  : http://localhost:${MNEMO_PORT}"
echo "  Dashboard  : http://localhost:${MNEMO_PORT}/_/"
echo ""
echo "Quick test:"
echo "  curl http://localhost:${MNEMO_PORT}/health"
echo ""
echo "Remember a fact:"
cat <<'EXAMPLE'
  curl -X POST http://localhost:${MNEMO_PORT}/api/v1/memory \
    -H 'Content-Type: application/json' \
    -d '{"user":"alice","text":"I love hiking in Colorado","role":"user"}'
EXAMPLE
echo ""
echo "Retrieve context:"
cat <<'EXAMPLE'
  curl -X POST http://localhost:${MNEMO_PORT}/api/v1/memory/alice/context \
    -H 'Content-Type: application/json' \
    -d '{"query":"What are my hobbies?"}'
EXAMPLE
echo ""
echo "Stop: docker compose down"
