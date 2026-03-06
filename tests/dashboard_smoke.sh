#!/usr/bin/env bash
# Dashboard smoke test — verifies embedded assets are served correctly.
# Requires a running Mnemo server on http://localhost:8080.
set -euo pipefail

BASE="${MNEMO_URL:-http://localhost:8080}"
PASS=0
FAIL=0

check() {
  local label="$1" url="$2" expected_status="$3" body_grep="${4:-}"
  local status body
  status=$(curl -s -o /tmp/_mnemo_smoke_body -w "%{http_code}" "$url")
  body=$(cat /tmp/_mnemo_smoke_body)
  if [[ "$status" != "$expected_status" ]]; then
    echo "FAIL  $label — expected $expected_status, got $status"
    FAIL=$((FAIL + 1))
    return
  fi
  if [[ -n "$body_grep" ]] && ! echo "$body" | grep -q "$body_grep"; then
    echo "FAIL  $label — body missing '$body_grep'"
    FAIL=$((FAIL + 1))
    return
  fi
  echo "OK    $label"
  PASS=$((PASS + 1))
}

echo "=== Mnemo Dashboard Smoke Test ==="
echo "Target: $BASE"
echo ""

# Health (baseline)
check "GET /health"            "$BASE/health"            200 '"status":"ok"'

# Dashboard index
check "GET /_/"                "$BASE/_/"                200 '<!DOCTYPE html>'

# Bare /_ redirects to /_/
check "GET /_ (redirect)"     "$BASE/_"                 308 ''

# Static assets
check "GET /_/static/style.css"  "$BASE/_/static/style.css"  200 ':root'
check "GET /_/static/app.js"     "$BASE/_/static/app.js"     200 'use strict'

# SPA routes serve index.html
check "GET /_/webhooks"        "$BASE/_/webhooks"        200 '<!DOCTYPE html>'
check "GET /_/rca"             "$BASE/_/rca"             200 '<!DOCTYPE html>'
check "GET /_/governance"      "$BASE/_/governance"      200 '<!DOCTYPE html>'
check "GET /_/traces"          "$BASE/_/traces"          200 '<!DOCTYPE html>'
check "GET /_/explorer"        "$BASE/_/explorer"        200 '<!DOCTYPE html>'

# 404 for non-existent static
check "GET /_/static/nope.xyz" "$BASE/_/static/nope.xyz" 404 ''

# List webhooks endpoint
check "GET /api/v1/memory/webhooks" "$BASE/api/v1/memory/webhooks" 200 '"count"'

echo ""
echo "Results: $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]] && exit 0 || exit 1
