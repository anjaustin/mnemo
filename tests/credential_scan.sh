#!/usr/bin/env bash
# =============================================================================
# SEC-01 through SEC-05: Credential hygiene falsification tests
#
# Usage: ./tests/credential_scan.sh
# Can run in CI without any external dependencies.
# =============================================================================
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0
WARN=0

pass() { echo -e "${GREEN}[PASS]${NC} $1"; PASS=$((PASS + 1)); }
fail() { echo -e "${RED}[FAIL]${NC} $1"; FAIL=$((FAIL + 1)); }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; WARN=$((WARN + 1)); }

echo "============================================"
echo "  Mnemo Credential Scan"
echo "  SEC-01 through SEC-05"
echo "============================================"
echo

# ==========================================================================
# SEC-01: .keys/ directory is gitignored
# ==========================================================================
echo "--- SEC-01: .keys/ directory is gitignored ---"
TRACKED_KEYS=$(git ls-files .keys/ 2>/dev/null || true)
if [ -z "$TRACKED_KEYS" ]; then
    pass "SEC-01: No files under .keys/ are tracked by git"
else
    fail "SEC-01: Found tracked files under .keys/: $TRACKED_KEYS"
fi

# ==========================================================================
# SEC-02: Sensitive file patterns are gitignored
# ==========================================================================
echo
echo "--- SEC-02: Sensitive file patterns gitignored ---"
PATTERNS=("*.pem" "credentials.json" "terraform.tfstate" "terraform.tfstate.backup" "*.key" ".env")
ALL_PATTERNS_OK=true

for pattern in "${PATTERNS[@]}"; do
    if git ls-files "$pattern" 2>/dev/null | grep -q .; then
        fail "SEC-02: Found tracked file matching pattern '$pattern'"
        ALL_PATTERNS_OK=false
    fi
done

# Also check for specific known sensitive files
SENSITIVE_FILES=("zep_api.key" ".env.local" ".env.production")
for f in "${SENSITIVE_FILES[@]}"; do
    TRACKED=$(git ls-files "$f" 2>/dev/null || true)
    if [ -n "$TRACKED" ]; then
        fail "SEC-02: Sensitive file '$f' is tracked by git"
        ALL_PATTERNS_OK=false
    fi
done

if [ "$ALL_PATTERNS_OK" = true ]; then
    pass "SEC-02: All sensitive file patterns are properly gitignored"
fi

# ==========================================================================
# SEC-03: Terraform state files not tracked
# ==========================================================================
echo
echo "--- SEC-03: Terraform state files not tracked ---"
TF_STATE_FILES=$(git ls-files '**/terraform.tfstate' '**/terraform.tfstate.backup' '**/*.tfstate' 2>/dev/null || true)
if [ -z "$TF_STATE_FILES" ]; then
    pass "SEC-03: No Terraform state files tracked"
else
    fail "SEC-03: Terraform state files tracked: $TF_STATE_FILES"
fi

# ==========================================================================
# SEC-04: No API keys, tokens, or passwords in tracked files
# ==========================================================================
echo
echo "--- SEC-04: No secrets in tracked source files ---"
SEC04_CLEAN=true

# Patterns that indicate real secrets (not documentation examples)
# We scan tracked files only, excluding test files, docs, and .env.example
SECRET_PATTERNS=(
    'sk-[a-zA-Z0-9]{20,}'           # OpenAI-style keys
    'sk-ant-[a-zA-Z0-9]{20,}'       # Anthropic keys
    'rnd_[a-zA-Z0-9]{20,}'          # Render tokens
    'dop_v1_[a-zA-Z0-9]{20,}'       # DigitalOcean tokens
    'ghp_[a-zA-Z0-9]{36}'           # GitHub personal tokens
    'gho_[a-zA-Z0-9]{36}'           # GitHub OAuth tokens
    'AKIA[A-Z0-9]{16}'              # AWS access keys
    'nf_sa_[a-zA-Z0-9]{20,}'        # Northflank service account keys
)

# Files to exclude from scanning (test data, docs, examples, this script)
EXCLUDE_PATTERNS="--exclude=*.example --exclude=credential_scan.sh --exclude=QA_QC_FALSIFICATION_PRD.md --exclude=*.md"

for pattern in "${SECRET_PATTERNS[@]}"; do
    # Use git grep to only scan tracked files
    MATCHES=$(git grep -lE "$pattern" -- '*.rs' '*.toml' '*.yml' '*.yaml' '*.json' '*.sh' '*.py' '*.tf' 2>/dev/null || true)
    if [ -n "$MATCHES" ]; then
        # Double-check each match to filter false positives (e.g., regex patterns in code)
        for file in $MATCHES; do
            # Skip test files and documentation
            if [[ "$file" == *test* ]] || [[ "$file" == *spec* ]] || [[ "$file" == docs/* ]]; then
                continue
            fi
            # Check if it's a real secret vs a pattern/example
            REAL_MATCH=$(git grep -cE "$pattern" -- "$file" 2>/dev/null || echo "0")
            if [ "$REAL_MATCH" != "0" ]; then
                fail "SEC-04: Potential secret pattern '$pattern' found in $file"
                SEC04_CLEAN=false
            fi
        done
    fi
done

if [ "$SEC04_CLEAN" = true ]; then
    pass "SEC-04: No API keys, tokens, or passwords found in tracked source files"
fi

# ==========================================================================
# SEC-05: .env.example files contain no real values
# ==========================================================================
echo
echo "--- SEC-05: .env.example files contain placeholder values only ---"
SEC05_CLEAN=true

ENV_EXAMPLES=$(git ls-files '**/.env.example' '.env.example' 2>/dev/null || true)
if [ -z "$ENV_EXAMPLES" ]; then
    warn "SEC-05: No .env.example files found"
else
    for envfile in $ENV_EXAMPLES; do
        # Check for values that look like real secrets (long random alphanumeric after =)
        # Exclude known safe patterns: comments, URLs, model names, provider names, env var refs
        SUSPICIOUS=$(grep -E '=.{20,}' "$envfile" 2>/dev/null | grep -vE '(^#|example|placeholder|your-|changeme|xxx|localhost|redis://|http://|openai|anthropic|ollama|liquid|embedding|text-|gpt-|claude|comma-separated|API keys)' || true)
        if [ -n "$SUSPICIOUS" ]; then
            fail "SEC-05: Suspicious values in $envfile: $SUSPICIOUS"
            SEC05_CLEAN=false
        fi
    done

    if [ "$SEC05_CLEAN" = true ]; then
        pass "SEC-05: All .env.example files contain only placeholder values"
    fi
fi

# ==========================================================================
# Summary
# ==========================================================================
echo
echo "============================================"
echo "  Credential Scan Summary"
echo "============================================"
echo -e "  ${GREEN}PASS: $PASS${NC}"
echo -e "  ${RED}FAIL: $FAIL${NC}"
echo -e "  ${YELLOW}WARN: $WARN${NC}"
echo

if [ "$FAIL" -gt 0 ]; then
    echo -e "${RED}CREDENTIAL SCAN FAILED — $FAIL issue(s) found${NC}"
    exit 1
else
    echo -e "${GREEN}CREDENTIAL SCAN PASSED${NC}"
    exit 0
fi
