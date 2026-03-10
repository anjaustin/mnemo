# Mnemo — Vultr Deployment (Terraform)

Deploy Mnemo on a [Vultr](https://www.vultr.com) VPS instance using Terraform. The startup script provisions Docker, Redis Stack, Qdrant, and the Mnemo server automatically.

---

## What Gets Created

| Resource | Details |
|---|---|
| `vultr_instance` | Ubuntu 24.04 LTS, `vc2-2c-4gb` (2 vCPU / 4 GB RAM) |
| `vultr_firewall_group` | Rules for SSH (22), Mnemo API (8080), HTTPS (443) |
| Docker stack | `redis-stack-server:7.4.0-v1`, `qdrant:v1.12.4`, `ghcr.io/anjaustin/mnemo/mnemo-server:latest` |

**Cost estimate:** ~$20/month for the `vc2-2c-4gb` plan.

---

## Prerequisites

1. A [Vultr account](https://www.vultr.com) with an API key.
2. An SSH key uploaded to Vultr (`Account → SSH Keys`).
3. [Terraform](https://www.terraform.io/downloads) >= 1.3 installed.

---

## Quick Start

```bash
cd deploy/vultr/terraform

# Create terraform.tfvars (never commit this)
cat > terraform.tfvars <<EOF
vultr_api_key       = "your-api-key"
ssh_key_name        = "your-ssh-key-name"
 mnemo_image                     = "ghcr.io/anjaustin/mnemo/mnemo-server:latest"
 mnemo_llm_provider              = "anthropic"
 mnemo_llm_api_key               = "sk-ant-..."
 mnemo_llm_model                 = "claude-haiku-4-20250514"
 mnemo_embedding_provider        = "local"
 mnemo_embedding_model           = "AllMiniLML6V2"
 mnemo_embedding_dimensions      = "384"
 mnemo_qdrant_prefix             = "mnemo_vultr_384_"
 mnemo_session_summary_threshold = "10"
EOF

terraform init
terraform plan -out=mnemo.plan
terraform apply mnemo.plan
```

---

## Verify

```bash
# Get the instance IP from Terraform output
IP=$(terraform output -raw instance_ip)

# Wait for startup script (~3-5 minutes for Docker install + image pulls)
ssh root@$IP 'tail -f /var/log/mnemo-init.log'

# Health check
curl http://$IP:8080/health
# Expected: {"status":"ok","version":"0.4.0"}

# Write test
curl -s -X POST http://$IP:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on Vultr"}'
```

---

## Tear Down

```bash
terraform destroy
```

---

## Notes

- Uses `redis/redis-stack-server` (not `redis/redis-stack`) to ensure RedisSearch and RedisJSON modules load correctly. Persistence args are passed via the `REDIS_ARGS` environment variable.
- The instance's `user_data` (cloud-init) runs the startup script on first boot only.
- Vultr firewall is a separate resource; the instance does not have an inline firewall.
- Region `ewr` (New Jersey) is the default; change via `region` variable.
- Vultr Terraform provider: `vultr/vultr` v2.x ([docs](https://registry.terraform.io/providers/vultr/vultr/latest/docs)).
