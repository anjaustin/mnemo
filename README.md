# Mnemo

| CI | Falsification | Benchmarks | Packages | Release |
| --- | --- | --- | --- | --- |
| [![quality-gates](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/quality-gates.yml) | [![memory-falsification](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/memory-falsification.yml) | [![benchmark-eval](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/benchmark-eval.yml) | [![package-ghcr](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/package-ghcr.yml) | [![release](https://github.com/anjaustin/mnemo/actions/workflows/release.yml/badge.svg)](https://github.com/anjaustin/mnemo/actions/workflows/release.yml) |

| Version | License | Stars | Downloads | GHCR |
| --- | --- | --- | --- | --- |
| [![version](https://img.shields.io/github/v/release/anjaustin/mnemo?display_name=tag&sort=semver)](https://github.com/anjaustin/mnemo/releases/latest) | [![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE) | [![stars](https://img.shields.io/github/stars/anjaustin/mnemo?style=flat&label=stars&color=blue)](https://github.com/anjaustin/mnemo/stargazers) | [![downloads](https://img.shields.io/github/downloads/anjaustin/mnemo/total?label=downloads&color=blue)](https://github.com/anjaustin/mnemo/releases) | [![ghcr](https://img.shields.io/badge/ghcr-mnemo--server-1f6feb)](https://github.com/anjaustin/mnemo/pkgs/container/mnemo%2Fmnemo-server) |

![Mnemosyne](img/mnemosyne.gif)

**Memory infrastructure for production AI agents.**

Mnemo is a free, open-source, self-hosted memory and context engine for agent systems. Built in Rust with Redis and Qdrant, it focuses on temporal correctness, fast recall, and operational simplicity.

## Quickstart

```bash
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
```

No API keys required. Starts Mnemo with local embeddings, Redis, and Qdrant. See [QUICKSTART.md](QUICKSTART.md) for the full walkthrough.

## 30-Second Demo

```bash
# Remember
curl -X POST http://localhost:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"demo","text":"The project deadline is March 15th."}'

# Recall
curl -X POST http://localhost:8080/api/v1/memory/demo/context \
  -H "Content-Type: application/json" \
  -d '{"query":"When is the deadline?"}'
```

## Why Mnemo

- **Temporal memory**: Facts can be superseded while preserving history for point-in-time recall
- **Fast context assembly**: Hybrid retrieval (semantic + graph + full-text) with token budgeting
- **Enterprise controls**: RBAC, data classification, guardrails, multi-agent shared memory
- **Self-hosted**: Deploy-it-yourself control, not a managed black box

**[Full capabilities list →](docs/CAPABILITIES.md)**

## Deploy

All 10 targets falsified. Production Helm chart for Kubernetes.

| Docker | AWS | GCP | DigitalOcean | Render |
|:------:|:---:|:---:|:------------:|:------:|
| [![Docker][docker-btn]][docker-deploy] | [![AWS][aws-btn]][aws-deploy] | [![GCP][gcp-btn]][gcp-deploy] | [![DO][do-btn]][do-deploy] | [![Render][render-btn]][render-deploy] |

| Railway | Vultr | Northflank | Linode | Kubernetes |
|:-------:|:-----:|:----------:|:------:|:----------:|
| [![Railway][railway-btn]][railway-deploy] | [![Vultr][vultr-btn]][vultr-deploy] | [![Northflank][northflank-btn]][northflank-deploy] | [![Linode][linode-btn]][linode-deploy] | [![K8s][k8s-btn]][k8s-deploy] |

[docker-btn]: ./img/deploy/docker.svg
[docker-deploy]: deploy/docker/DEPLOY.md
[aws-btn]: ./img/deploy/aws.svg
[aws-deploy]: deploy/aws/cloudformation/DEPLOY.md
[gcp-btn]: https://deploy.cloud.run/button.svg
[gcp-deploy]: deploy/gcp/DEPLOY.md
[do-btn]: https://www.deploytodo.com/do-btn-blue.svg
[do-deploy]: deploy/digitalocean/DEPLOY.md
[render-btn]: https://render.com/images/deploy-to-render-button.svg
[render-deploy]: deploy/render/DEPLOY.md
[railway-btn]: https://railway.app/button.svg
[railway-deploy]: deploy/railway/DEPLOY.md
[vultr-btn]: ./img/deploy/vultr.svg
[vultr-deploy]: deploy/vultr/DEPLOY.md
[northflank-btn]: https://assets.northflank.com/deploy_to_northflank_smm_36700fb050.svg
[northflank-deploy]: deploy/northflank/DEPLOY.md
[linode-btn]: ./img/deploy/linode.svg
[linode-deploy]: deploy/linode/DEPLOY.md
[k8s-btn]: ./img/deploy/kubernetes.svg
[k8s-deploy]: docs/DEPLOY.md

## SDKs

### Python

```bash
pip install git+https://github.com/anjaustin/mnemo.git#subdirectory=sdk/python
```

```python
from mnemo import Mnemo

client = Mnemo("http://localhost:8080")
client.add("user", "Important fact to remember.")
ctx = client.context("user", "What do you know?")
```

Includes [LangChain](docs/USAGE.md#langchain-adapter) and [LlamaIndex](docs/USAGE.md#llamaindex-adapter) adapters.

### TypeScript

```typescript
import { MnemoClient } from 'mnemo-client';

const client = new MnemoClient('http://localhost:8080');
await client.add('user', 'Important fact.');
const ctx = await client.context('user', 'What do you know?');
```

Includes [LangChain.js](docs/USAGE.md#langchainjs-adapter) and [Vercel AI SDK](docs/USAGE.md#vercel-ai-sdk) adapters.

**[Full usage guide →](docs/USAGE.md)**

## How Temporal Memory Works

Most memory systems overwrite facts. Mnemo keeps the timeline.

```
Aug 2024: "Renewal status is green."
  → status: green (valid_at: Aug 2024)

Feb 2025: "Procurement blocked. Now at risk."
  → status: green (invalid_at: Feb 2025) ← superseded
  → status: at_risk (valid_at: Feb 2025) ← current
```

Old facts aren't deleted. This enables point-in-time queries and change tracking.

## Architecture

```
Agent Runtime
    │
    ▼
REST/gRPC API (mnemo-server)
    │
    ├── Redis   (state, graph, full-text)
    └── Qdrant  (vectors)
```

Single Rust binary. 142 REST endpoints + 30 gRPC RPCs. MCP server for Claude Code.

**[Full architecture →](docs/ARCHITECTURE.md)**

## Competitive Position

| | Mnemo | Zep | Mem0 | Letta |
|---|---:|---:|---:|---:|
| Features shipped | **64** | 38 | 26 | 22 |
| Partial | 2 | 14 | 19 | 11 |
| Not available | 11 | 25 | 32 | 44 |

**[Full competitive matrix →](docs/COMPETITIVE_MATRIX.md)**

## Documentation

| Document | Description |
|----------|-------------|
| **[Quickstart](QUICKSTART.md)** | Get running in 5 minutes |
| **[Usage Guide](docs/USAGE.md)** | API examples, SDKs, integrations |
| **[Capabilities](docs/CAPABILITIES.md)** | Full feature list |
| **[Configuration](docs/CONFIGURATION.md)** | Environment variables reference |
| **[Architecture](docs/ARCHITECTURE.md)** | Data model and pipeline internals |
| **[API Reference](docs/API.md)** | All endpoints with examples |
| **[Deployment](docs/DEPLOY.md)** | Kubernetes and cloud deployment |
| **[Competitive Matrix](docs/COMPETITIVE_MATRIX.md)** | Feature comparison with alternatives |
| **[Project Status](docs/PROJECT_STATUS.md)** | Roadmap and release history |

### More Docs

<details>
<summary>Click to expand full documentation list</summary>

| Document | Description |
|----------|-------------|
| [Tutorial](docs/TUTORIAL.md) | Build a support agent (20-min walkthrough) |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Common issues and solutions |
| [Webhooks](docs/WEBHOOKS.md) | Event types and signature verification |
| [Chat Import](docs/IMPORTING_CHAT_HISTORY.md) | Import formats and migration |
| [Agent Identity](docs/AGENT_IDENTITY_SUBSTRATE.md) | Identity core and experience weighting |
| [Thread HEAD](docs/THREAD_HEAD.md) | Git-like session state |
| [Temporal Vectorization](docs/TEMPORAL_VECTORIZATION.md) | Time-aware retrieval scoring |
| [Benchmarks](docs/BENCHMARKS.md) | Latency and throughput measurements |
| [Evaluation](docs/EVALUATION.md) | Temporal quality measurement |
| [Testing](docs/TESTING.md) | Test commands and falsification |
| [Contributing](CONTRIBUTING.md) | Dev setup and PR process |
| [Security](SECURITY.md) | Vulnerability reporting |
| [Changelog](CHANGELOG.md) | Release notes |

</details>

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions.

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

*Named after Mnemosyne, the Greek Titaness of memory and mother of the Muses.*
