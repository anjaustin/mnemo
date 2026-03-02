#!/usr/bin/env bash
# ============================================================================
# Mnemo End-to-End Smoke Test
#
# Tests the full workflow: create user → session → episode → get context
#
# Prerequisites:
#   - Mnemo server running on localhost:8080
#   - Redis + Qdrant running (via docker compose up)
#
# Usage:
#   ./tests/e2e.sh
#   ./tests/e2e.sh http://localhost:9090  # custom base URL
# ============================================================================

set -euo pipefail

BASE_URL="${1:-http://localhost:8080}"
PASS=0
FAIL=0

green() { printf "\033[32m%s\033[0m\n" "$1"; }
red() { printf "\033[31m%s\033[0m\n" "$1"; }
bold() { printf "\033[1m%s\033[0m\n" "$1"; }

assert_status() {
    local expected="$1"
    local actual="$2"
    local label="$3"
    if [ "$actual" -eq "$expected" ]; then
        green "  ✓ $label (HTTP $actual)"
        PASS=$((PASS + 1))
    else
        red "  ✗ $label (expected HTTP $expected, got $actual)"
        FAIL=$((FAIL + 1))
    fi
}

assert_json_field() {
    local json="$1"
    local field="$2"
    local label="$3"
    if echo "$json" | python3 -c "import sys,json; d=json.load(sys.stdin); assert '$field' in d" 2>/dev/null; then
        green "  ✓ $label (field '$field' present)"
        PASS=$((PASS + 1))
    else
        red "  ✗ $label (field '$field' missing)"
        FAIL=$((FAIL + 1))
    fi
}

extract() {
    echo "$1" | python3 -c "import sys,json; print(json.load(sys.stdin)$2)"
}

# ── Health check ────────────────────────────────────────────────────
bold "=== Health Check ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/health")
assert_status 200 "$STATUS" "Health endpoint"

# ── Create user ─────────────────────────────────────────────────────
bold "=== Create User ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/users" \
    -H "Content-Type: application/json" \
    -d '{"name": "E2E Test User", "email": "e2e@test.com", "external_id": "e2e_test_'$$'"}')
BODY=$(echo "$RESPONSE" | head -n -1)
STATUS=$(echo "$RESPONSE" | tail -1)
assert_status 201 "$STATUS" "Create user"
USER_ID=$(extract "$BODY" "['id']")
bold "  User ID: $USER_ID"

# ── Get user ────────────────────────────────────────────────────────
bold "=== Get User ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/api/v1/users/$USER_ID")
assert_status 200 "$STATUS" "Get user by ID"

# ── Create session ──────────────────────────────────────────────────
bold "=== Create Session ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/sessions" \
    -H "Content-Type: application/json" \
    -d "{\"user_id\": \"$USER_ID\", \"name\": \"E2E Test Session\"}")
BODY=$(echo "$RESPONSE" | head -n -1)
STATUS=$(echo "$RESPONSE" | tail -1)
assert_status 201 "$STATUS" "Create session"
SESSION_ID=$(extract "$BODY" "['id']")
bold "  Session ID: $SESSION_ID"

# ── Add episodes ────────────────────────────────────────────────────
bold "=== Add Episodes ==="
for MSG in \
    '{"type":"message","role":"user","name":"Kendra","content":"I just switched from Adidas to Nike running shoes! Training for the Boston Marathon."}' \
    '{"type":"message","role":"assistant","content":"Great choice! Nike has excellent marathon shoes. How is your training going?"}' \
    '{"type":"message","role":"user","name":"Kendra","content":"Going well! I run 40 miles a week now. My coach Mike suggested I also try Brooks for longer runs."}'; do

    STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
        "$BASE_URL/api/v1/sessions/$SESSION_ID/episodes" \
        -H "Content-Type: application/json" \
        -d "$MSG")
    assert_status 201 "$STATUS" "Add episode"
done

# ── Wait for processing ─────────────────────────────────────────────
bold "=== Waiting for ingestion (up to 30s) ==="
ATTEMPTS=0
MAX_ATTEMPTS=30
while [ $ATTEMPTS -lt $MAX_ATTEMPTS ]; do
    EPISODES=$(curl -s "$BASE_URL/api/v1/sessions/$SESSION_ID/episodes?limit=10")
    PENDING=$(echo "$EPISODES" | python3 -c "
import sys, json
data = json.load(sys.stdin)
pending = sum(1 for e in data['data'] if e['processing_status'] == 'pending')
print(pending)
" 2>/dev/null || echo "?")

    if [ "$PENDING" = "0" ]; then
        green "  ✓ All episodes processed"
        PASS=$((PASS + 1))
        break
    fi
    printf "  ... %s episodes still pending\n" "$PENDING"
    sleep 1
    ATTEMPTS=$((ATTEMPTS + 1))
done

if [ $ATTEMPTS -eq $MAX_ATTEMPTS ]; then
    red "  ✗ Episodes not processed within 30s (is LLM configured?)"
    FAIL=$((FAIL + 1))
fi

# ── List entities ───────────────────────────────────────────────────
bold "=== List Entities ==="
RESPONSE=$(curl -s "$BASE_URL/api/v1/users/$USER_ID/entities?limit=20")
ENTITY_COUNT=$(extract "$RESPONSE" "['count']" 2>/dev/null || echo "0")
bold "  Entities found: $ENTITY_COUNT"
if [ "$ENTITY_COUNT" -gt 0 ]; then
    green "  ✓ Entities extracted"
    PASS=$((PASS + 1))
else
    red "  ✗ No entities found (may need LLM API key)"
    FAIL=$((FAIL + 1))
fi

# ── Get context ─────────────────────────────────────────────────────
bold "=== Get Context ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
    "$BASE_URL/api/v1/users/$USER_ID/context" \
    -H "Content-Type: application/json" \
    -d '{"messages": [{"role": "user", "content": "What running shoes does Kendra prefer?"}], "max_tokens": 500}')
BODY=$(echo "$RESPONSE" | head -n -1)
STATUS=$(echo "$RESPONSE" | tail -1)
assert_status 200 "$STATUS" "Get context"
assert_json_field "$BODY" "context" "Context string present"
assert_json_field "$BODY" "latency_ms" "Latency tracked"

CONTEXT=$(extract "$BODY" "['context']" 2>/dev/null || echo "")
if [ -n "$CONTEXT" ] && [ "$CONTEXT" != "" ]; then
    green "  ✓ Context string is non-empty"
    PASS=$((PASS + 1))
else
    red "  ✗ Context string is empty"
    FAIL=$((FAIL + 1))
fi

# ── Cleanup ─────────────────────────────────────────────────────────
bold "=== Cleanup ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE "$BASE_URL/api/v1/users/$USER_ID")
assert_status 200 "$STATUS" "Delete user"

# ── Results ─────────────────────────────────────────────────────────
echo ""
bold "============================================"
bold "Results: $PASS passed, $FAIL failed"
bold "============================================"

if [ $FAIL -gt 0 ]; then
    exit 1
fi
exit 0
