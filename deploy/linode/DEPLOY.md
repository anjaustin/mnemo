# Mnemo — Linode / Akamai Cloud Terraform Deployment

Deploy the full Mnemo stack on a single Linode instance. All three services (mnemo-server, Redis, Qdrant) run via Docker Compose, managed by a cloud-init startup script.

---

## What Gets Created

| Resource | Default | Cost estimate |
|---|---|---|
| Linode instance | g6-standard-2 (2 vCPU / 4 GB RAM) | ~$18/month |
| Boot disk | 80 GB (included) | Included |
| Firewall | ports 8080, 80, 443, 22 open | Free |
| Public IPv4 | 1 static IP | Included |
| **Total** | | **~$18/month** |

> Linode is the most cost-effective of the T3–T5 targets at the same resource floor.

---

## Prerequisites

- Linode account with a personal access token (read/write)
- Terraform >= 1.3 installed
- SSH key pair — public key ready to paste

---

## Step 1 — Configure

```bash
cd deploy/linode/terraform
```

Create `terraform.tfvars`:

```hcl
linode_token        = "your-linode-api-token"
region              = "us-ord"        # Chicago — change to nearest region
instance_type       = "g6-standard-2" # 2 vCPU / 4 GB / ~$18/month

ssh_authorized_keys = ["ssh-ed25519 AAAA... your@key"]

# Recommended image + local embedding config
# mnemo_image                = "ghcr.io/anjaustin/mnemo/mnemo-server:latest"
# mnemo_llm_provider         = "anthropic"
# mnemo_llm_api_key          = "sk-ant-..."
# mnemo_llm_model            = "claude-haiku-4-20250514"
# mnemo_embedding_provider   = "local"
# mnemo_embedding_model      = "AllMiniLML6V2"
# mnemo_embedding_dimensions = "384"
# mnemo_qdrant_prefix        = "mnemo_linode_384_"
# mnemo_session_summary_threshold = "10"

# Enable auth before public exposure
# mnemo_auth_enabled  = "true"
# mnemo_auth_api_keys = "key1,key2"
```

---

## Step 2 — Init and Plan

```bash
terraform init
terraform plan -out=mnemo.plan
```

Review the plan — should show 2 resources: `linode_instance.mnemo` and `linode_firewall.mnemo`.

---

## Step 3 — Apply

```bash
terraform apply mnemo.plan
```

Apply completes in ~30 seconds. Terraform outputs:
- `instance_ip` — public IPv4
- `health_check_url` — direct link to `/health`
- `ssh_command` — SSH command

The startup script runs in the background after apply. Allow ~3 minutes for Docker install, image pulls, and service startup.

---

## Step 4 — Verify

```bash
# Get IP from output
IP=$(terraform output -raw instance_ip)

# Health check (wait ~3 min after apply)
curl http://$IP:8080/health
# Expected: {"status":"ok","version":"0.4.0"}

If you already have shell access to the host, you can also update the stack in place without a working Linode API token by replacing the `mnemo` image in the compose file and keeping `/data/redis` and `/data/qdrant` mounted.

# Write a memory
curl -s -X POST http://$IP:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on Linode"}'

# Read context
curl -s -X POST http://$IP:8080/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"Linode","limit":5}'

# Persistence test
ssh root@$IP "cd /opt/mnemo && docker compose restart mnemo && sleep 20"
# Repeat context query — data should survive
```

---

## SSH Access

```bash
IP=$(terraform output -raw instance_ip)
ssh root@$IP

# Check init log
tail -f /var/log/mnemo-init.log

# Check stack status
cd /opt/mnemo && docker compose ps
docker compose logs mnemo
```

---

## Reverse Proxy (nginx + TLS)

```bash
ssh root@$IP

apt-get install -y nginx certbot python3-certbot-nginx

sudo tee /etc/nginx/sites-available/mnemo > /dev/null <<'EOF'
server {
    listen 80;
    server_name your.domain.example;
    proxy_connect_timeout 60s;
    proxy_send_timeout   120s;
    proxy_read_timeout   120s;
    location / {
        proxy_pass       http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
EOF

ln -s /etc/nginx/sites-available/mnemo /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx
certbot --nginx -d your.domain.example
```

---

## Updating Mnemo

```bash
ssh root@$IP "cd /opt/mnemo && docker compose pull mnemo && docker compose up -d mnemo"
```

---

## Teardown

```bash
terraform destroy
```

> The boot disk is destroyed with the instance. Back up `/data/redis` and `/data/qdrant` first if you need the data.

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Health check unreachable after 5 min | `ssh root@$IP 'cat /var/log/mnemo-init.log'` |
| Port 8080 closed | Firewall created? `linode-cli firewalls list` |
| Services not starting | `ssh root@$IP 'cd /opt/mnemo && docker compose ps && docker compose logs'` |
| Context empty after restart | Redis AOF enabled — data survives. Check `docker compose logs redis` |
| `terraform apply` fails on firewall | Linode API token needs read/write firewall scope |
