# Deployment

Production deployment guides for Mnemo.

---

## In This Section

| Guide | Description |
|-------|-------------|
| **[Docker](docker.md)** | Local and single-server deployment |
| **[Kubernetes & Cloud](../../DEPLOY.md)** | Helm chart and cloud providers |

For platform-specific guides, see the [deploy/](../../../deploy/) directory.

---

## Deployment Options

### Development / Testing

Use Docker Compose:

```bash
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
```

### Production (Single Server)

Use Docker Compose with production config:

```bash
docker compose -f docker-compose.prod.yml up -d
```

### Production (Kubernetes)

Use Helm chart:

```bash
helm repo add mnemo https://anjaustin.github.io/mnemo/charts
helm install mnemo mnemo/mnemo
```

### Managed Platforms

One-click deploys available for:
- Render
- Railway
- Northflank

---

## Architecture

```
┌─────────────────────────────────────────┐
│              Load Balancer               │
│         (nginx / traefik / ALB)         │
└─────────────────┬───────────────────────┘
                  │
        ┌─────────┴─────────┐
        │                   │
        ▼                   ▼
┌───────────────┐   ┌───────────────┐
│    Mnemo      │   │    Mnemo      │
│   Replica 1   │   │   Replica 2   │
└───────┬───────┘   └───────┬───────┘
        │                   │
        └─────────┬─────────┘
                  │
        ┌─────────┴─────────┐
        │                   │
        ▼                   ▼
┌───────────────┐   ┌───────────────┐
│     Redis     │   │    Qdrant     │
│   (HA/Cluster)│   │   (Cluster)   │
└───────────────┘   └───────────────┘
```

---

## Resource Requirements

| Component | Minimum | Recommended | Notes |
|-----------|---------|-------------|-------|
| Mnemo (remote embed) | 256 MB | 512 MB | Stateless |
| Mnemo (local embed) | 1.5 GB | 2 GB | FastEmbed models |
| Redis | 256 MB | 1 GB | Scale with data |
| Qdrant | 512 MB | 2 GB | Scale with vectors |

---

## Checklist

Before going to production:

- [ ] Enable authentication (`MNEMO_AUTH_ENABLED=true`)
- [ ] Configure persistent storage for Redis and Qdrant
- [ ] Set up SSL/TLS termination
- [ ] Configure backups
- [ ] Set up monitoring (Prometheus + Grafana)
- [ ] Configure log aggregation
- [ ] Test failover and recovery

---

## Quick Links

- **[Docker Compose](docker.md)** - Fastest path to running
- **[Kubernetes Helm](../../DEPLOY.md)** - Production-grade deployment
- **[Configuration](../reference/configuration.md)** - All settings
