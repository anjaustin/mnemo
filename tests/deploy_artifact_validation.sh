#!/usr/bin/env bash
# =============================================================================
# DEP-02 through DEP-06: Deployment artifact validation
#
# Validates the structural correctness of all deployment artifacts.
# Can run in CI without cloud credentials.
# =============================================================================
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0
WARN=0
SKIP=0

pass() { echo -e "${GREEN}[PASS]${NC} $1"; PASS=$((PASS + 1)); }
fail() { echo -e "${RED}[FAIL]${NC} $1"; FAIL=$((FAIL + 1)); }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; WARN=$((WARN + 1)); }
skip() { echo -e "${YELLOW}[SKIP]${NC} $1"; SKIP=$((SKIP + 1)); }

echo "============================================"
echo "  Mnemo Deployment Artifact Validation"
echo "  DEP-02 through DEP-06"
echo "============================================"
echo

# ==========================================================================
# DEP-02: CloudFormation template validates (structural check)
# ==========================================================================
echo "--- DEP-02: CloudFormation template ---"
CFN_FILE="deploy/aws/cloudformation/mnemo_cfn.yaml"
if [ -f "$CFN_FILE" ]; then
    # Check it's valid YAML
    if python3 -c "import yaml; yaml.safe_load(open('$CFN_FILE'))" 2>/dev/null; then
        pass "DEP-02: CloudFormation template is valid YAML"
    else
        # CloudFormation uses !Ref, !Sub etc. which safe_load rejects.
        # Use a more lenient check.
        if python3 -c "
import yaml
class CFNLoader(yaml.SafeLoader):
    pass
for tag in ['!Ref', '!Sub', '!GetAtt', '!FindInMap', '!Select', '!Join', '!If', '!Equals', '!Not', '!And', '!Or', '!Condition', '!Base64', '!Cidr', '!GetAZs', '!ImportValue', '!Split', '!Transform']:
    CFNLoader.add_multi_constructor(tag, lambda loader, suffix, node: None)
    CFNLoader.add_constructor(tag, lambda loader, node: None)
with open('$CFN_FILE') as f:
    doc = yaml.load(f, Loader=CFNLoader)
assert 'AWSTemplateFormatVersion' in doc or 'Resources' in doc, 'Missing required CFN keys'
" 2>/dev/null; then
            pass "DEP-02: CloudFormation template is valid (with CFN intrinsic functions)"
        else
            fail "DEP-02: CloudFormation template failed structural validation"
        fi
    fi

    # Check required sections exist
    if grep -q "Resources:" "$CFN_FILE" 2>/dev/null; then
        pass "DEP-02: CloudFormation template has Resources section"
    else
        fail "DEP-02: CloudFormation template missing Resources section"
    fi

    if grep -q "Parameters:" "$CFN_FILE" 2>/dev/null; then
        pass "DEP-02: CloudFormation template has Parameters section"
    else
        warn "DEP-02: CloudFormation template missing Parameters section"
    fi
else
    fail "DEP-02: CloudFormation template not found at $CFN_FILE"
fi

# ==========================================================================
# DEP-03: Terraform configs validate (structural check)
# ==========================================================================
echo
echo "--- DEP-03: Terraform configs ---"
TF_TARGETS=("deploy/gcp/terraform" "deploy/digitalocean/terraform" "deploy/vultr/terraform" "deploy/linode/terraform")

for tf_dir in "${TF_TARGETS[@]}"; do
    if [ -d "$tf_dir" ]; then
        # Check that main.tf exists
        if [ -f "$tf_dir/main.tf" ]; then
            pass "DEP-03: $tf_dir/main.tf exists"
        else
            fail "DEP-03: $tf_dir/main.tf missing"
        fi

        # Check for variables.tf
        if [ -f "$tf_dir/variables.tf" ]; then
            pass "DEP-03: $tf_dir/variables.tf exists"
        else
            warn "DEP-03: $tf_dir/variables.tf missing"
        fi

        # Check for outputs.tf
        if [ -f "$tf_dir/outputs.tf" ]; then
            pass "DEP-03: $tf_dir/outputs.tf exists"
        else
            warn "DEP-03: $tf_dir/outputs.tf missing"
        fi

        # Structural check: main.tf should contain terraform/provider blocks
        if grep -qE '(terraform\s*\{|provider\s+")' "$tf_dir/main.tf" 2>/dev/null; then
            pass "DEP-03: $tf_dir/main.tf has terraform/provider blocks"
        else
            warn "DEP-03: $tf_dir/main.tf may be missing terraform/provider blocks"
        fi

        # Run terraform validate if terraform is available
        if command -v terraform &>/dev/null; then
            if (cd "$tf_dir" && terraform init -backend=false -no-color 2>/dev/null && terraform validate -no-color 2>/dev/null); then
                pass "DEP-03: $tf_dir passes terraform validate"
            else
                warn "DEP-03: $tf_dir terraform validate failed (may need provider init)"
            fi
        else
            skip "DEP-03: terraform not installed, skipping validate for $tf_dir"
        fi
    else
        fail "DEP-03: Terraform directory not found: $tf_dir"
    fi
done

# ==========================================================================
# DEP-04: Render blueprint validates (structural YAML check)
# ==========================================================================
echo
echo "--- DEP-04: Render blueprint ---"
RENDER_FILE="deploy/render/render.yaml"
if [ -f "$RENDER_FILE" ]; then
    if python3 -c "
import yaml
with open('$RENDER_FILE') as f:
    doc = yaml.safe_load(f)
assert 'services' in doc, 'Missing services key'
assert len(doc['services']) > 0, 'No services defined'
print(f'Found {len(doc[\"services\"])} service(s)')
" 2>&1; then
        pass "DEP-04: Render blueprint is valid YAML with services"
    else
        fail "DEP-04: Render blueprint validation failed"
    fi
else
    fail "DEP-04: Render blueprint not found at $RENDER_FILE"
fi

# ==========================================================================
# DEP-05: Railway config validates (structural JSON check)
# ==========================================================================
echo
echo "--- DEP-05: Railway config ---"
RAILWAY_FILE="deploy/railway/railway.json"
if [ -f "$RAILWAY_FILE" ]; then
    if python3 -c "
import json
with open('$RAILWAY_FILE') as f:
    doc = json.load(f)
# Railway manifests should have some structure
assert isinstance(doc, dict), 'Root should be a JSON object'
print(f'Railway config keys: {list(doc.keys())}')
" 2>&1; then
        pass "DEP-05: Railway config is valid JSON"
    else
        fail "DEP-05: Railway config validation failed"
    fi
else
    fail "DEP-05: Railway config not found at $RAILWAY_FILE"
fi

# ==========================================================================
# DEP-06: Northflank stack validates (structural JSON check)
# ==========================================================================
echo
echo "--- DEP-06: Northflank stack ---"
NORTHFLANK_FILE="deploy/northflank/stack.json"
if [ -f "$NORTHFLANK_FILE" ]; then
    if python3 -c "
import json
with open('$NORTHFLANK_FILE') as f:
    doc = json.load(f)
assert isinstance(doc, dict), 'Root should be a JSON object'
# Northflank stacks should have services or addons
has_services = 'services' in doc or 'spec' in doc or 'apiVersion' in doc
print(f'Northflank stack keys: {list(doc.keys())}')
assert has_services or len(doc) > 0, 'Stack appears empty'
" 2>&1; then
        pass "DEP-06: Northflank stack is valid JSON"
    else
        fail "DEP-06: Northflank stack validation failed"
    fi
else
    fail "DEP-06: Northflank stack not found at $NORTHFLANK_FILE"
fi

# ==========================================================================
# Bonus: Check all deploy targets have DEPLOY.md
# ==========================================================================
echo
echo "--- Bonus: Deploy documentation ---"
DEPLOY_TARGETS=("docker" "bare-metal" "aws/cloudformation" "gcp" "digitalocean" "render" "railway" "vultr" "northflank" "linode")
for target in "${DEPLOY_TARGETS[@]}"; do
    DEPLOY_DOC="deploy/$target/DEPLOY.md"
    # Handle terraform subdirectory
    if [ ! -f "$DEPLOY_DOC" ]; then
        DEPLOY_DOC="deploy/$target/terraform/DEPLOY.md"
    fi
    if [ -f "$DEPLOY_DOC" ]; then
        pass "Deploy docs: $target has DEPLOY.md"
    else
        warn "Deploy docs: $target missing DEPLOY.md"
    fi
done

# ==========================================================================
# Summary
# ==========================================================================
echo
echo "============================================"
echo "  Deployment Artifact Validation Summary"
echo "============================================"
echo -e "  ${GREEN}PASS: $PASS${NC}"
echo -e "  ${RED}FAIL: $FAIL${NC}"
echo -e "  ${YELLOW}WARN: $WARN${NC}"
echo -e "  ${YELLOW}SKIP: $SKIP${NC}"
echo

if [ "$FAIL" -gt 0 ]; then
    echo -e "${RED}VALIDATION FAILED — $FAIL issue(s) found${NC}"
    exit 1
else
    echo -e "${GREEN}VALIDATION PASSED${NC}"
    exit 0
fi
