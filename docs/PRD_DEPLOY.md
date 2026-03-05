# Mnemo Deployment PRD

**Status:** Planning  
**Version:** 0.3.1  
**Date:** 2026-03-04

---

## 1) Problem Statement

Mnemo has a working GHCR image, a `docker-compose.yml` for local development, and binary release artifacts. What it lacks is a deployment story: no cloud templates, no one-click targets, no bare-metal guide, no reverse-proxy reference. Operators who want to run Mnemo in production must figure everything out themselves.

The goal is to match — and in meaningful ways exceed — the deployment surface area of AnythingLLM, the open-source project we are building deployment parity with. AnythingLLM covers: Docker, AWS CloudFormation, GCP Deployment Manager, DigitalOcean Terraform, Render, Railway, Elestio, Northflank, and bare metal. We want the same reach, adapted to Mnemo's three-service architecture.

---

## 2) Mnemo Architecture Constraints

Every deployment target must solve one problem AnythingLLM doesn't have: **Mnemo is a three-service stack.**

| Service | Role | Persistence |
|---|---|---|
| `mnemo-server` | Rust binary — REST API, ingest, retrieval | Stateless |
| `redis` | Episode store, graph state, session index | Persistent volume |
| `qdrant` | Vector index — semantic search | Persistent volume |

This means:
- Single-container deployments are impossible without bundling all three — acceptable only for dev/demo.
- Cloud deployments can use managed Redis (ElastiCache, Memorystore, Upstash) and managed Qdrant (Qdrant Cloud) to reduce ops burden and avoid running all three services on one instance.
- Persistent volumes are non-negotiable for Redis and Qdrant. Data loss on restart is not acceptable for a memory system.
- `mnemo-server` itself is stateless and restartable — it is the easiest piece.

### Resource Floor

| Size | RAM | Disk | Suitable for |
|---|---|---|---|
| Minimum (all-in-one) | 2 GB | 20 GB | Dev, demo, single-user |
| Small production | 4 GB | 40 GB | Small team, low write volume |
| Medium production | 8 GB | 100 GB | Multi-tenant, sustained writes |
| Large production | 16+ GB | 250 GB+ | High-throughput, many users |

---

## 3) Deployment Targets

### T1 — Docker (single host, all-in-one)
**Priority:** P0 — ships first  
**Tooling:** `docker compose`  
**Status:** Partially done (`docker-compose.yml` exists for dev)

The production compose file is different from the dev file: it uses GHCR images instead of `build: .`, sets restart policies, adds explicit resource limits, and strips debug logging.

**Deliverables:**
- `deploy/docker/docker-compose.prod.yml` — production-ready compose with GHCR image pulls, named volumes, resource limits, healthchecks, and `.env` variable passthrough
- `deploy/docker/docker-compose.managed.yml` — variant that externalizes Redis and Qdrant (env vars point to managed services), only runs `mnemo-server`
- `deploy/docker/.env.example` — all required and optional env vars with comments
- `deploy/docker/DEPLOY.md` — quick-start guide: pull images, configure `.env`, `docker compose up -d`, verify `/health`

**Key decisions:**
- Images pulled from `ghcr.io/anjaustin/mnemo/mnemo-server:<version>` — already published
- Redis: `redis/redis-stack:latest` (includes RedisInsight) for single-host; externalizable
- Qdrant: `qdrant/qdrant:<pinned version>` for single-host; externalizable
- Volumes: named, not bind-mounts (portable, backup-friendly)
- Port surface: `8080` (Mnemo), `6379` (Redis, internal only in prod), `6333/6334` (Qdrant, internal only in prod)

---

### T2 — Bare Metal / VPS (systemd)
**Priority:** P0 — ships with T1  
**Tooling:** `systemd`, `nginx`  
**Status:** Not started

Mnemo's single Rust binary is ideal for bare-metal. No Node.js runtime, no build step, just download and run. Redis and Qdrant are installed as system services or via Docker.

**Deliverables:**
- `deploy/bare-metal/DEPLOY.md` — step-by-step guide:
  1. Install Redis + Qdrant (Docker or native packages)
  2. Download `mnemo-server` binary from GitHub Releases
  3. Create `/etc/mnemo/mnemo.toml` config
  4. Install `deploy/bare-metal/mnemo.service` systemd unit
  5. `systemctl enable --now mnemo`
  6. Configure nginx reverse proxy
  7. Verify `/health`
- `deploy/bare-metal/mnemo.service` — systemd unit file with `Restart=always`, `EnvironmentFile=`, and `LimitNOFILE`
- `deploy/bare-metal/nginx.conf` — reverse proxy with timeout config appropriate for long-running context requests (no WebSocket needed — Mnemo is pure HTTP/REST)
- `deploy/bare-metal/update.sh` — fetch latest binary from GitHub Releases, swap in place, `systemctl restart mnemo`

**Key decisions:**
- Binary downloaded via `gh release download` or direct URL from `SHA256SUMS.txt`-verified asset
- Redis and Qdrant run as Docker containers even on bare metal (simplest, most reliable)
- nginx handles TLS termination; Mnemo binds to `127.0.0.1:8080`
- No process manager beyond systemd (no PM2, no supervisord)

---

### T3 — AWS (CloudFormation)
**Priority:** P1  
**Tooling:** AWS CloudFormation  
**Status:** Not started

Single EC2 instance running the full three-service stack via Docker Compose. Equivalent to AnythingLLM's CloudFormation approach.

**Deliverables:**
- `deploy/aws/cloudformation/mnemo_cfn.yaml` — CloudFormation template:
  - EC2 instance (t3.medium default, parameterizable)
  - Security group: port 8080 open (0.0.0.0/0), SSH restricted to parameterized CIDR
  - EBS volume (20 GiB gp3, separate from root, mounted at `/data`)
  - UserData script: install Docker, pull GHCR images, write compose file and `.env`, start stack
  - Output: instance public IP + `http://<ip>:8080/health` URL
- `deploy/aws/cloudformation/DEPLOY.md` — console and CLI deploy instructions, parameter table, cost estimate, SSH access for log inspection

**Infrastructure output:**
- 1 EC2 instance (t3.medium ≈ $30/month)
- 1 Security Group
- 1 EBS gp3 volume (20 GiB ≈ $1.60/month)

**Optional future extension:** ECS Fargate variant with managed ElastiCache + Qdrant Cloud (managed services, no persistent volume management).

---

### T4 — GCP (Deployment Manager or Terraform)
**Priority:** P1  
**Tooling:** Terraform (preferred over GCP Deployment Manager — better ecosystem)  
**Status:** Not started

AnythingLLM uses GCP Deployment Manager YAML. We'll use Terraform instead — it's provider-agnostic and sets us up for reuse across T4/T5/T6.

**Deliverables:**
- `deploy/gcp/terraform/main.tf` — GCP Compute Engine VM (e2-medium default), firewall rule for port 8080, startup script installs Docker and launches compose stack
- `deploy/gcp/terraform/variables.tf` — project, region, zone, machine type, disk size
- `deploy/gcp/terraform/outputs.tf` — external IP, health URL
- `deploy/gcp/DEPLOY.md` — `gcloud auth`, `terraform init/plan/apply`, verify, `terraform destroy`

**Infrastructure output:**
- 1 Compute Engine VM (e2-medium ≈ $24/month)
- 1 Firewall rule
- 1 Persistent disk (20 GiB ≈ $0.80/month)

---

### T5 — DigitalOcean (Terraform)
**Priority:** P1  
**Tooling:** Terraform  
**Status:** Not started

Matches AnythingLLM's DigitalOcean target. Uses the same Terraform module structure as T4 for consistency — different provider, same patterns.

**Deliverables:**
- `deploy/digitalocean/terraform/main.tf` — DO Droplet (s-2vcpu-4gb default), user-data installs Docker + compose stack
- `deploy/digitalocean/terraform/variables.tf` — token, region, droplet size, SSH key
- `deploy/digitalocean/terraform/outputs.tf` — public IPv4, health URL
- `deploy/digitalocean/DEPLOY.md` — token setup, `terraform init/plan/apply`, verify, destroy

**Infrastructure output:**
- 1 Droplet (s-2vcpu-4gb ≈ $24/month)
- 1 public IPv4

---

### T6 — Render.com
**Priority:** P2  
**Tooling:** `render.yaml` blueprint  
**Status:** Not started

Render supports Docker services and managed Redis. Qdrant would run as a separate Render service or use Qdrant Cloud.

**Deliverables:**
- `deploy/render/render.yaml` — Render blueprint defining:
  - `mnemo-server` Docker service (GHCR image, port 8080, env vars)
  - `mnemo-redis` Redis service (Render managed Redis) or env var pointing to external
  - `MNEMO_QDRANT_URL` env var pointing to Qdrant Cloud (Qdrant has a free tier)
- `deploy/render/DEPLOY.md` — fork repo, connect Render, set env vars, deploy

**Constraint:** Render's free tier is not suitable (Redis is paid; persistent disk is paid). Starter plan ~$14/month for the server + ~$10/month managed Redis = ~$24/month minimum. Document this clearly.

---

### T7 — Railway
**Priority:** P2  
**Tooling:** Railway template (`railway.json` or template repo)  
**Status:** Not started

Railway supports multi-service templates. Redis is available as a Railway plugin. Qdrant must be self-hosted as a Railway service or externalized.

**Deliverables:**
- `deploy/railway/railway.json` — Railway template manifest:
  - `mnemo-server` service (GHCR image)
  - `mnemo-redis` service (Railway Redis plugin or custom Redis image)
  - `mnemo-qdrant` service (Qdrant image with persistent volume)
  - Environment variable wiring between services
- `deploy/railway/DEPLOY.md` — deploy from template, configure env, verify

---

### T8 — Elestio
**Priority:** P3  
**Tooling:** Elestio managed open source hosting  
**Status:** Not started

Elestio can host any Docker-based open source project. Requires submitting a software listing. Lower engineering effort — primarily documentation and the compose file.

**Deliverables:**
- `deploy/elestio/docker-compose.yml` — Elestio-compatible compose (follows their conventions)
- Submit Mnemo to Elestio's software catalog

---

### T9 — Northflank
**Priority:** P3  
**Tooling:** Northflank stack definition  
**Status:** Not started

Northflank supports multi-service stacks from Docker images with persistent volumes.

**Deliverables:**
- `deploy/northflank/stack.json` — Northflank stack definition for mnemo-server + redis + qdrant
- `deploy/northflank/DEPLOY.md`

---

## 4) Shared Infrastructure: What All Targets Need

Every deployment target uses the same `mnemo-server` binary/image and the same configuration surface. The shared pieces that must be written once:

### 4.1 Production `.env.example`

```bash
# Required
MNEMO_REDIS_URL=redis://localhost:6379
MNEMO_QDRANT_URL=http://localhost:6334

# LLM (optional — Mnemo works without one)
MNEMO_LLM_PROVIDER=openai          # openai | anthropic | ollama | liquid
MNEMO_LLM_API_KEY=
MNEMO_LLM_MODEL=gpt-4o-mini
MNEMO_EMBEDDING_API_KEY=
MNEMO_EMBEDDING_MODEL=text-embedding-3-small

# Auth (disabled by default — enable before exposing publicly)
MNEMO_AUTH_ENABLED=false
MNEMO_AUTH_API_KEYS=               # comma-separated keys

# Server
MNEMO_SERVER_HOST=0.0.0.0
MNEMO_SERVER_PORT=8080

# Webhooks
MNEMO_WEBHOOKS_ENABLED=true
MNEMO_WEBHOOKS_MAX_ATTEMPTS=3

# Logging
RUST_LOG=mnemo=info
```

### 4.2 Reverse Proxy Reference (nginx)

Mnemo is pure HTTP/REST — no WebSocket, no streaming connections that need special proxy configuration. The nginx config is simpler than AnythingLLM's:

```nginx
server {
    listen 80;
    server_name your.domain.example;

    # Increase timeouts for long-running context requests
    proxy_connect_timeout 60s;
    proxy_send_timeout    120s;
    proxy_read_timeout    120s;

    location / {
        proxy_pass         http://127.0.0.1:8080;
        proxy_set_header   Host $host;
        proxy_set_header   X-Real-IP $remote_addr;
        proxy_set_header   X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
    }
}
```

TLS termination via Certbot/Let's Encrypt:
```bash
certbot --nginx -d your.domain.example
```

### 4.3 Managed Services Option

For cloud deployments, operators can replace self-hosted Redis and Qdrant with managed services to eliminate volume management:

| Service | Managed option | Notes |
|---|---|---|
| Redis | AWS ElastiCache, GCP Memorystore, Upstash, Redis Cloud | Must support Redis Stack modules (RedisSearch, RedisJSON) |
| Qdrant | Qdrant Cloud (free tier: 1GB) | Point `MNEMO_QDRANT_URL` at the cloud cluster URL + API key |

When using managed services, only `mnemo-server` needs to be deployed — no volumes, no sidecars.

---

## 5) Rollout Priority and Sequencing

| Phase | Targets | Gate |
|---|---|---|
| **Phase 1** | T1 Docker, T2 Bare Metal | Production compose file + systemd unit + nginx ref — no cloud account needed to test |
| **Phase 2** | T3 AWS, T4 GCP, T5 DigitalOcean | Cloud IaC templates — one-command deploy to each provider |
| **Phase 3** | T6 Render, T7 Railway | PaaS blueprints — one-click or near-one-click deploy |
| **Phase 4** | T8 Elestio, T9 Northflank | Catalog submissions — low engineering, high reach |

Each phase gate: deployed successfully from scratch, verified `/health` returns `{"status":"ok"}`, data persists across a container/instance restart.

---

## 6) Directory Layout

```
deploy/
├── docker/
│   ├── docker-compose.prod.yml
│   ├── docker-compose.managed.yml
│   ├── .env.example
│   └── DEPLOY.md
├── bare-metal/
│   ├── DEPLOY.md
│   ├── mnemo.service
│   ├── nginx.conf
│   └── update.sh
├── aws/
│   └── cloudformation/
│       ├── mnemo_cfn.yaml
│       └── DEPLOY.md
├── gcp/
│   └── terraform/
│       ├── main.tf
│       ├── variables.tf
│       ├── outputs.tf
│       └── DEPLOY.md
├── digitalocean/
│   └── terraform/
│       ├── main.tf
│       ├── variables.tf
│       ├── outputs.tf
│       └── DEPLOY.md
├── render/
│   ├── render.yaml
│   └── DEPLOY.md
├── railway/
│   ├── railway.json
│   └── DEPLOY.md
├── elestio/
│   ├── docker-compose.yml
│   └── DEPLOY.md
└── northflank/
    ├── stack.json
    └── DEPLOY.md
```

---

## 7) Falsification Gates

Each deployment target is not complete until:

1. **Cold start**: fresh clone, follow `DEPLOY.md` exactly, reach `GET /health` → `{"status":"ok","version":"..."}`
2. **Write + read**: `POST /api/v1/memory` → `POST /api/v1/memory/:user/context` returns non-empty context
3. **Persistence**: restart the service, repeat the context query — same result
4. **Connectivity**: Redis and Qdrant are reachable from `mnemo-server`; confirm via startup logs
5. **Reverse proxy** (where applicable): access via domain over HTTPS; confirm `x-mnemo-request-id` header present in response

---

## 8) What We Are Not Doing (Scope Exclusions)

- **Kubernetes / Helm chart** — meaningful engineering effort, low immediate return. Add later once cloud IaC is solid.
- **Multi-region / HA** — single-instance deploys only in this PRD. HA is a future concern.
- **Managed Mnemo hosting** — we are self-hosted only. No Mintplex-Labs-equivalent hosted offering in scope.
- **Desktop app** — Mnemo is a server, not an Electron app. No desktop packaging.
- **Windows native** — Docker on Windows works. Native Windows binary is out of scope.
