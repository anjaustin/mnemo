# Deploying Mnemo on Kubernetes

This guide covers deploying Mnemo to Kubernetes using the official Helm chart.

## Prerequisites

- Kubernetes 1.25+
- Helm 3.10+
- `kubectl` configured for your cluster
- A container registry with the `mnemo-server` image (GHCR publishes automatically via CI)

## Quick Start

```bash
# Add dependency repos (one-time)
helm repo add bitnami https://charts.bitnami.com/bitnami
helm repo add qdrant https://qdrant.github.io/qdrant-helm
helm repo update

# Install with default values (local embedding, no auth keys — dev only)
helm install mnemo deploy/kubernetes/mnemo/

# Install with auth keys (production)
helm install mnemo deploy/kubernetes/mnemo/ \
  --set mnemo.bootstrapKeys="your-secret-api-key" \
  --namespace mnemo --create-namespace
```

## Configuration

All configuration is in `values.yaml`. Key settings:

### Mnemo Server

| Parameter | Default | Description |
|-----------|---------|-------------|
| `replicaCount` | `2` | Base replica count (overridden by HPA) |
| `image.repository` | `ghcr.io/anjaustin/mnemo/mnemo-server` | Container image |
| `image.tag` | `""` (appVersion) | Image tag |
| `mnemo.host` | `0.0.0.0` | Bind address |
| `mnemo.port` | `8080` | REST + gRPC port (same port) |
| `mnemo.authEnabled` | `true` | Enable API key auth |
| `mnemo.bootstrapKeys` | `""` | Comma-separated bootstrap API keys |
| `mnemo.existingSecret` | `""` | Use existing Secret for auth keys |
| `mnemo.llmProvider` | `anthropic` | LLM provider |
| `mnemo.llmModel` | `claude-haiku-4-20250514` | LLM model |
| `mnemo.embeddingProvider` | `local` | Embedding provider |
| `mnemo.embeddingModel` | `AllMiniLML6V2` | Embedding model |
| `mnemo.embeddingDimensions` | `384` | Vector dimensions |
| `mnemo.qdrantPrefix` | `mnemo` | Qdrant collection prefix |
| `mnemo.extraEnv` | `[]` | Additional env vars (e.g., API keys) |

### High Availability

| Parameter | Default | Description |
|-----------|---------|-------------|
| `autoscaling.enabled` | `true` | Enable HPA |
| `autoscaling.minReplicas` | `2` | Minimum replicas |
| `autoscaling.maxReplicas` | `10` | Maximum replicas |
| `autoscaling.targetCPUUtilizationPercentage` | `70` | CPU scale threshold |
| `autoscaling.targetMemoryUtilizationPercentage` | `80` | Memory scale threshold |
| `podDisruptionBudget.enabled` | `true` | Enable PDB |
| `podDisruptionBudget.minAvailable` | `1` | Min available during disruptions |

### Security

The chart enforces security best practices by default:

- `runAsNonRoot: true` (UID 65532)
- `readOnlyRootFilesystem: true`
- `allowPrivilegeEscalation: false`
- `capabilities.drop: [ALL]`
- Dedicated ServiceAccount with no extra permissions

### Dependencies

| Parameter | Default | Description |
|-----------|---------|-------------|
| `redis.enabled` | `true` | Deploy Redis subchart |
| `redis.architecture` | `standalone` | Redis topology |
| `redis.auth.enabled` | `false` | Redis auth (enable in prod) |
| `qdrant.enabled` | `true` | Deploy Qdrant subchart |
| `qdrant.replicaCount` | `1` | Qdrant replicas |

## Production Deployment

### Using External Redis and Qdrant

```bash
helm install mnemo deploy/kubernetes/mnemo/ \
  --set redis.enabled=false \
  --set qdrant.enabled=false \
  --set mnemo.redisUrl="redis://your-redis:6379" \
  --set mnemo.qdrantUrl="http://your-qdrant:6334" \
  --set mnemo.bootstrapKeys="your-secret-key" \
  --namespace mnemo --create-namespace
```

### Using an Existing Secret for Auth Keys

```bash
# Create the secret first
kubectl create secret generic mnemo-auth \
  --from-literal=MNEMO_AUTH_BOOTSTRAP_KEYS="key1,key2" \
  --namespace mnemo

# Reference it in the install
helm install mnemo deploy/kubernetes/mnemo/ \
  --set mnemo.existingSecret=mnemo-auth \
  --namespace mnemo
```

### Injecting LLM API Keys

```bash
# Create a secret with your API key
kubectl create secret generic mnemo-llm-keys \
  --from-literal=anthropic-api-key="sk-ant-..." \
  --namespace mnemo

# Reference via extraEnv
helm install mnemo deploy/kubernetes/mnemo/ \
  --set mnemo.bootstrapKeys="your-key" \
  --set 'mnemo.extraEnv[0].name=ANTHROPIC_API_KEY' \
  --set 'mnemo.extraEnv[0].valueFrom.secretKeyRef.name=mnemo-llm-keys' \
  --set 'mnemo.extraEnv[0].valueFrom.secretKeyRef.key=anthropic-api-key' \
  --namespace mnemo --create-namespace
```

### Enabling Ingress

```bash
helm install mnemo deploy/kubernetes/mnemo/ \
  --set ingress.enabled=true \
  --set ingress.className=nginx \
  --set 'ingress.hosts[0].host=mnemo.example.com' \
  --set 'ingress.hosts[0].paths[0].path=/' \
  --set 'ingress.hosts[0].paths[0].pathType=Prefix' \
  --set 'ingress.tls[0].secretName=mnemo-tls' \
  --set 'ingress.tls[0].hosts[0]=mnemo.example.com' \
  --set 'ingress.annotations.cert-manager\.io/cluster-issuer=letsencrypt-prod' \
  --namespace mnemo --create-namespace
```

## Verifying the Deployment

```bash
# Check pods are running
kubectl get pods -n mnemo -l app.kubernetes.io/name=mnemo

# Check the health endpoint
kubectl port-forward -n mnemo svc/mnemo 8080:8080
curl http://localhost:8080/health

# Access Swagger UI
open http://localhost:8080/swagger-ui

# Test gRPC
grpcurl -plaintext localhost:8080 list
```

## Upgrading

```bash
helm upgrade mnemo deploy/kubernetes/mnemo/ \
  --reuse-values \
  --set image.tag="0.7.0" \
  --namespace mnemo
```

The deployment has a `checksum/config` annotation that triggers rolling restarts when ConfigMap values change.

## Uninstalling

```bash
helm uninstall mnemo --namespace mnemo

# PVCs are not deleted automatically (Redis/Qdrant data preserved)
kubectl delete pvc -l app.kubernetes.io/instance=mnemo -n mnemo
```

## Architecture

```
                    ┌─────────────┐
                    │   Ingress   │  (optional)
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │   Service   │  ClusterIP :8080
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼─────┐┌────▼────┐┌─────▼─────┐
        │  Pod (1)   ││ Pod (2) ││  Pod (N)  │  HPA: 2-10
        │ mnemo-srv  ││mnemo-srv││ mnemo-srv │
        └─────┬──┬──┘└────┬──┬─┘└─────┬──┬──┘
              │  │        │  │        │  │
              │  └────────┼──┼────────┼──┘
              │           │  │        │
        ┌─────▼───────────▼──▼────────▼─────┐
        │            Redis (standalone)      │
        └───────────────────────────────────┘
        ┌───────────────────────────────────┐
        │            Qdrant (1 replica)      │
        └───────────────────────────────────┘
```

## Chart Files

| File | Purpose |
|------|---------|
| `Chart.yaml` | Chart metadata, subchart dependencies |
| `values.yaml` | Default configuration values |
| `templates/_helpers.tpl` | Template helpers (names, labels, URLs) |
| `templates/deployment.yaml` | Mnemo server Deployment |
| `templates/service.yaml` | ClusterIP Service |
| `templates/configmap.yaml` | Non-secret env vars |
| `templates/secret.yaml` | Bootstrap auth keys (conditional) |
| `templates/serviceaccount.yaml` | ServiceAccount |
| `templates/hpa.yaml` | HorizontalPodAutoscaler |
| `templates/pdb.yaml` | PodDisruptionBudget |
| `templates/ingress.yaml` | Ingress (conditional) |
| `templates/NOTES.txt` | Post-install instructions |
