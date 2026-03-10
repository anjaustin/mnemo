# Mnemo — GCP Terraform Deployment

Deploy the full Mnemo stack on a single Compute Engine instance with a persistent SSD data disk. All three services (mnemo-server, Redis, Qdrant) run via Docker Compose.

---

## What Gets Created

| Resource | Default | Cost estimate |
|---|---|---|
| Compute Engine VM | e2-medium (2 vCPU / 4 GB RAM) | ~$24/month |
| Boot disk | 20 GB pd-ssd | ~$3.40/month |
| Data disk | 20 GB pd-ssd (Redis + Qdrant) | ~$3.40/month |
| Firewall rule | ports 8080, 80, 443, 22 open | Free |
| External IP | Ephemeral | ~$0/month (ephemeral, charged only when unattached) |
| **Total** | | **~$31/month** |

---

## Prerequisites

- GCP project with billing enabled and Compute Engine API enabled
- `gcloud` CLI installed and authenticated:
  ```bash
  gcloud auth application-default login
  gcloud config set project YOUR_PROJECT_ID
  ```
- Terraform >= 1.3 installed
- (Optional) SSH key registered with GCP for console SSH access

---

## Step 1 — Configure

```bash
cd deploy/gcp/terraform
cp terraform.tfvars.example terraform.tfvars   # see below
$EDITOR terraform.tfvars
```

Create `terraform.tfvars`:

```hcl
project = "your-gcp-project-id"
region  = "us-central1"
zone    = "us-central1-a"

# Recommended image + local embedding config
mnemo_image                     = "ghcr.io/anjaustin/mnemo/mnemo-server:latest"
mnemo_llm_provider              = "anthropic"
mnemo_llm_api_key               = "sk-ant-..."
mnemo_llm_model                 = "claude-haiku-4-20250514"
mnemo_embedding_provider        = "local"
mnemo_embedding_model           = "AllMiniLML6V2"
mnemo_embedding_dimensions      = "384"
mnemo_qdrant_prefix             = "mnemo_gcp_384_"
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

Review the plan — should show 3 resources to create (firewall, disk, instance).

---

## Step 3 — Apply

```bash
terraform apply mnemo.plan
```

Apply takes ~3 minutes. Terraform outputs:
- `instance_external_ip` — the public IP
- `health_check_url` — direct link to `/health`
- `ssh_command` — gcloud SSH command

---

## Step 4 — Verify

Wait ~2 minutes after apply for Docker images to pull and services to start.

```bash
# Get IP from output
IP=$(terraform output -raw instance_external_ip)

# Health check
curl http://$IP:8080/health
# Expected: {"status":"ok","version":"0.3.7"}

# Write a memory
curl -s -X POST http://$IP:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on GCP"}'

# Read context
curl -s -X POST http://$IP:8080/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"GCP","limit":5}'

# Persistence test — SSH in and restart
gcloud compute ssh mnemo-server --zone=us-central1-a --project=YOUR_PROJECT -- \
  "cd /opt/mnemo && sudo docker compose restart && sleep 20"
# Repeat context query — should return same result
```

---

## SSH Access

```bash
# Via gcloud (recommended — handles key management automatically)
gcloud compute ssh mnemo-server --zone=us-central1-a --project=YOUR_PROJECT

# Check init log
sudo cat /var/log/mnemo-init.log

# Check stack status
cd /opt/mnemo && sudo docker compose ps
sudo docker compose logs mnemo
```

---

## Reverse Proxy (nginx + TLS)

```bash
# SSH in
gcloud compute ssh mnemo-server --zone=us-central1-a --project=YOUR_PROJECT

# Install nginx + certbot
sudo apt-get install -y nginx certbot python3-certbot-nginx

# Create site config
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

sudo ln -s /etc/nginx/sites-available/mnemo /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
sudo certbot --nginx -d your.domain.example
```

---

## Updating Mnemo

```bash
gcloud compute ssh mnemo-server --zone=us-central1-a --project=YOUR_PROJECT -- \
  "cd /opt/mnemo && sudo docker compose pull mnemo && sudo docker compose up -d mnemo"
```

---

## Teardown

```bash
terraform destroy
```

> The data disk (`mnemo-data`) is destroyed by default. If you want to preserve data, take a snapshot first:
> ```bash
> gcloud compute disks snapshot mnemo-data --zone=us-central1-a --snapshot-names=mnemo-data-backup
> ```

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Health check unreachable | Firewall rule applied? `gcloud compute firewall-rules list --filter=name:mnemo` |
| Services not starting | Init log: `sudo cat /var/log/mnemo-init.log` |
| Data disk not mounting | `lsblk` — look for disk labeled `mnemo-data` |
| Context returns empty after restart | Redis AOF enabled — data should survive. Check `sudo docker compose logs redis` |
| `terraform apply` fails on firewall | Compute Engine API may not be enabled — enable it in the GCP console |
