#!/usr/bin/env bash
# ============================================================================
# Mnemo Quickstart — Zero to Memory in 30 Seconds
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
#
# Requirements: docker, docker compose (v2), curl
# ============================================================================
set -euo pipefail

COMPOSE_URL="https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/docker-compose.quickstart.yml"
COMPOSE_FILE="docker-compose.quickstart.yml"

echo ""
echo "  ╔══════════════════════════════════════╗"
echo "  ║         Mnemo Quickstart             ║"
echo "  ║   Memory infrastructure for agents   ║"
echo "  ╚══════════════════════════════════════╝"
echo ""

# ── Preflight checks ──────────────────────────────────────────────
command -v docker >/dev/null 2>&1 || { echo "Error: docker is not installed. See https://docs.docker.com/get-docker/"; exit 1; }
docker compose version >/dev/null 2>&1 || { echo "Error: docker compose v2 is required. See https://docs.docker.com/compose/install/"; exit 1; }

# ── Download compose file ─────────────────────────────────────────
echo "[1/3] Downloading compose file..."
curl -fsSL "$COMPOSE_URL" -o "$COMPOSE_FILE"

# ── Start services ────────────────────────────────────────────────
echo "[2/3] Starting Mnemo stack (Redis + Qdrant + Mnemo Server)..."
docker compose -f "$COMPOSE_FILE" up -d

# ── Wait for health ───────────────────────────────────────────────
echo "[3/3] Waiting for Mnemo to become healthy..."
for i in $(seq 1 30); do
  if curl -sf http://localhost:8080/health >/dev/null 2>&1; then
    echo ""
    echo "  Mnemo is running at http://localhost:8080"
    echo ""
    echo "  Quick test:"
    echo "    curl -X POST http://localhost:8080/api/v1/memory \\"
    echo "      -H 'Content-Type: application/json' \\"
    echo "      -d '{\"user\":\"alice\",\"text\":\"I love hiking\",\"role\":\"user\"}'"
    echo ""
    echo "  Dashboard:  http://localhost:8080/_/"
    echo ""
    echo "  MCP integration (Claude Code, Cursor):"
    echo "    Add to your MCP config:"
    echo "    {"
    echo "      \"mcpServers\": {"
    echo "        \"mnemo\": {"
    echo "          \"command\": \"docker\","
    echo "          \"args\": [\"exec\", \"-i\", \"mnemo-server\", \"mnemo-mcp-server\"]"
    echo "        }"
    echo "      }"
    echo "    }"
    echo ""
    echo "  To stop:  docker compose -f $COMPOSE_FILE down"
    echo ""
    exit 0
  fi
  sleep 2
done

echo "Warning: Mnemo did not become healthy within 60 seconds."
echo "Check logs: docker compose -f $COMPOSE_FILE logs mnemo"
exit 1
