# Mnemo Deployment Status

Current production-style deployment baseline for Phase 2 cloud falsification.

## Live Fleet

| Platform | URL | Version | Image | Smoke |
|---|---|---|---|---|
| Render | `https://mnemo-m70w.onrender.com` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| Northflank | `https://http--mnemo-server--blcxq2rhfzbr.code.run` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| Railway | `https://mnemo-production-be62.up.railway.app` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| DigitalOcean | `http://157.230.213.155:8080` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| Vultr | `http://173.199.127.234:8080` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| AWS | `http://3.238.130.59:8080` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| GCP | `http://34.133.58.28:8080` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |
| Linode | `http://172.232.7.137:8080` | `0.4.0` | `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0` | `9/9` |

## Current Deployment Profile

All successful cloud targets currently use the same runtime profile:

```dotenv
MNEMO_LLM_PROVIDER=anthropic
MNEMO_LLM_MODEL=claude-haiku-4-20250514
MNEMO_EMBEDDING_PROVIDER=local
MNEMO_EMBEDDING_MODEL=AllMiniLML6V2
MNEMO_EMBEDDING_DIMENSIONS=384
MNEMO_SESSION_SUMMARY_THRESHOLD=10
```

Each provider uses its own `MNEMO_QDRANT_PREFIX` to avoid vector-dimension or collection-shape collisions during rollouts and migrations.

## Provider Quirks

- Render: starter-sized web service OOMs with the larger local embedding model; `AllMiniLML6V2` at `384` dims is the stable floor.
- Northflank: external DNS uses `code.run`; private service names resolve directly within the project namespace.
- Railway: project tokens must use `Project-Access-Token`; a public `*.up.railway.app` domain may need to be created explicitly.
- DigitalOcean: region/size availability can vary by account; `nyc3` may reject stock size choices.
- Vultr: cloud-init `user_data` must be raw text, not pre-base64 encoded.
- Linode: live host access may be available over SSH even when the stored API token is invalid.

## Durable Image Plan

The live fleet was originally validated on a temporary `ttl.sh` image. All deploy guides and IaC templates have been updated to reference the durable GHCR images:

- `ghcr.io/anjaustin/mnemo/mnemo-server:0.4.0`
- `ghcr.io/anjaustin/mnemo/mnemo-server:0.4`
- `ghcr.io/anjaustin/mnemo/mnemo-server:latest`

The fleet status table above reflects the image used at original validation time. New deployments should use the GHCR images above.

## Revalidation Commands

Basic smoke:

```bash
bash tests/e2e_smoke.sh <base-url>
```

Fleet falsification sweep:

```bash
python3 tests/live_fleet_falsify.py
```

Async SDK live smoke:

```bash
PYTHONPATH=sdk/python python3 sdk/python/scripts/async_live_smoke.py
```

## Next Operator Baseline

Before deeper operator UX/dashboard work, keep these true:

- durable GHCR semver tags published
- fleet falsification rerunnable from repo scripts
- async SDK live smoke green against at least one public HTTPS target
- live deployment matrix kept current in this file
