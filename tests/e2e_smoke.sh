#!/usr/bin/env bash
# Mnemo deterministic E2E smoke test (no external LLM dependency).

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
        green "  OK $label (HTTP $actual)"
        PASS=$((PASS + 1))
    else
        red "  FAIL $label (expected HTTP $expected, got $actual)"
        FAIL=$((FAIL + 1))
    fi
}

assert_non_empty() {
    local value="$1"
    local label="$2"
    if [ -n "$value" ] && [ "$value" != "null" ]; then
        green "  OK $label"
        PASS=$((PASS + 1))
    else
        red "  FAIL $label"
        FAIL=$((FAIL + 1))
    fi
}

extract() {
    echo "$1" | python3 -c "import sys,json; print(json.load(sys.stdin)$2)"
}

bold "=== Health ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/health")
assert_status 200 "$STATUS" "health endpoint"

bold "=== Create User ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/users" \
    -H "Content-Type: application/json" \
    -d '{"name": "Smoke User", "email": "smoke@test.com", "external_id": "smoke_'$$'"}')
BODY=$(echo "$RESPONSE" | python3 -c 'import sys; print("\n".join(sys.stdin.read().splitlines()[:-1]))')
STATUS=$(echo "$RESPONSE" | python3 -c 'import sys; print(sys.stdin.read().splitlines()[-1])')
assert_status 201 "$STATUS" "create user"
USER_ID=$(extract "$BODY" "['id']")
assert_non_empty "$USER_ID" "user id returned"

bold "=== Create Session ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/sessions" \
    -H "Content-Type: application/json" \
    -d "{\"user_id\": \"$USER_ID\", \"name\": \"Smoke Session\"}")
BODY=$(echo "$RESPONSE" | python3 -c 'import sys; print("\n".join(sys.stdin.read().splitlines()[:-1]))')
STATUS=$(echo "$RESPONSE" | python3 -c 'import sys; print(sys.stdin.read().splitlines()[-1])')
assert_status 201 "$STATUS" "create session"
SESSION_ID=$(extract "$BODY" "['id']")
assert_non_empty "$SESSION_ID" "session id returned"

bold "=== Remember Memory ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
    "$BASE_URL/api/v1/memory" \
    -H "Content-Type: application/json" \
    -d "{\"user\":\"$USER_ID\",\"session\":\"Smoke Session\",\"text\":\"I like tea in the afternoon.\",\"role\":\"user\",\"name\":\"Smoke\"}")
BODY=$(echo "$RESPONSE" | python3 -c 'import sys; print("\n".join(sys.stdin.read().splitlines()[:-1]))')
STATUS=$(echo "$RESPONSE" | python3 -c 'import sys; print(sys.stdin.read().splitlines()[-1])')
assert_status 201 "$STATUS" "remember memory endpoint"

bold "=== Memory Context API ==="
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
    "$BASE_URL/api/v1/memory/$USER_ID/context" \
    -H "Content-Type: application/json" \
    -d '{"query": "What did I say about tea?", "max_tokens": 300}')
BODY=$(echo "$RESPONSE" | python3 -c 'import sys; print("\n".join(sys.stdin.read().splitlines()[:-1]))')
STATUS=$(echo "$RESPONSE" | python3 -c 'import sys; print(sys.stdin.read().splitlines()[-1])')
assert_status 200 "$STATUS" "memory context endpoint"
CONTEXT=$(extract "$BODY" "['context']" 2>/dev/null || true)
assert_non_empty "$CONTEXT" "non-empty context"

bold "=== Cleanup ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE "$BASE_URL/api/v1/users/$USER_ID")
assert_status 200 "$STATUS" "delete user"

echo ""
bold "============================================"
bold "Smoke results: $PASS passed, $FAIL failed"
bold "============================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
