# Mnemo — DigitalOcean Terraform Deployment

Deploy the full Mnemo stack on a single DigitalOcean Droplet. All three services (mnemo-server, Redis, Qdrant) run via Docker Compose, bootstrapped via user-data.

---

## What Gets Created

| Resource | Default | Cost estimate |
|---|---|---|
| Droplet | s-2vcpu-4gb (2 vCPU / 4 GB RAM) | ~$24/month |
| Boot disk | 80 GB SSD (included) | Included |
| Firewall | ports 8080, 80, 443, 22 open | Free |
| Public IPv4 | 1 static IP | Included |
| **Total** | | **~$24/month** |

---

## Prerequisites

- DigitalOcean account with a personal access token (read/write)
- SSH key uploaded to your DO account (Droplets → Settings → Security → SSH Keys)
- Terraform >= 1.3 installed

---

## Step 1 — Configure

```bash
cd deploy/digitalocean/terraform
```

Create `terraform.tfvars`:

```hcl
do_token     = "dop_v1_..."       # DigitalOcean API token
ssh_key_name = "my-key"           # Name of SSH key in your DO account
region       = "nyc3"             # Change to nearest region
droplet_size = "s-2vcpu-4gb"      # 2 vCPU / 4 GB / ~$24/month

# Recommended image + local embedding config
mnemo_image                = "ghcr.io/anjaustin/mnemo/mnemo-server:latest"
mnemo_llm_provider         = "anthropic"
mnemo_llm_api_key          = "sk-ant-..."
mnemo_llm_model            = "claude-haiku-4-20250514"
mnemo_embedding_provider   = "local"
mnemo_embedding_model      = "AllMiniLML6V2"
mnemo_embedding_dimensions = "384"
mnemo_qdrant_prefix        = "mnemo_do_384_"
mnemo_session_summary_threshold = "10"

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

Review the plan — should show 2 resources: `digitalocean_droplet.mnemo` and `digitalocean_firewall.mnemo`.

---

## Step 3 — Apply

```bash
terraform apply mnemo.plan
```

Apply completes in ~30 seconds. Terraform outputs:
- `droplet_ip` — public IPv4
- `health_check_url` — direct link to `/health`
- `ssh_command` — SSH command

Allow ~3 minutes for Docker install, image pulls, and service startup.

---

## Step 4 — Verify

```bash
IP=$(terraform output -raw droplet_ip)

# Health check (wait ~3 min after apply)
curl http://$IP:8080/health
# Expected: {"status":"ok","version":"0.3.7"}

# Write a memory
curl -s -X POST http://$IP:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on DigitalOcean"}'

# Read context
curl -s -X POST http://$IP:8080/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"DigitalOcean","limit":5}'

# Persistence test
ssh root@$IP "cd /opt/mnemo && docker compose restart mnemo && sleep 20"
# Repeat context query — data should survive
```

---

## SSH Access

```bash
IP=$(terraform output -raw droplet_ip)
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

tee /etc/nginx/sites-available/mnemo > /dev/null <<'EOF'
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

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Health check unreachable after 5 min | `ssh root@$IP 'cat /var/log/mnemo-init.log'` |
| Port 8080 closed | Firewall applied? `doctl compute firewall list` |
| Services not starting | `ssh root@$IP 'cd /opt/mnemo && docker compose ps && docker compose logs'` |
| Context empty after restart | Redis AOF enabled — data survives. Check `docker compose logs redis` |
| SSH key not found | Key must exist in DO account by the name in `ssh_key_name` var |
