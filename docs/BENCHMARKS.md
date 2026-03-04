# Benchmarks

This document is the public benchmark snapshot for Mnemo. It focuses on falsifiable outcomes: temporal accuracy, stale-fact rate, and latency.

Primary references:

- Harness: `eval/temporal_eval.py`
- CI workflow: `.github/workflows/benchmark-eval.yml`
- Methodology details: `docs/EVALUATION.md`
- Cross-system caveats: `docs/COMPETITIVE.md`

## Executive Snapshot

Latest local snapshots (2026-03-03):

| Dataset | System | Profile | Accuracy | Stale Fact Rate | Errors | p50 (ms) | p95 (ms) |
|---|---|---|---:|---:|---:|---:|---:|
| `temporal_cases.json` | mnemo | temporal | 100.0% | 0.0% | 0 | 103 | 103 |
| `temporal_cases.json` | mnemo | baseline | 66.7% | 33.3% | 0 | 71 | 71 |
| `scientific_research_cases_v2.json` | mnemo | temporal | 100.0% | 0.0% | 0 | 84 | 102 |
| `scientific_research_cases_v2.json` | mnemo | baseline | 50.0% | 40.0% | 0 | 75 | 107 |

Takeaways:

- Temporal mode consistently outperforms baseline on correctness and stale-fact suppression.
- Scientific-domain cases increase difficulty and widen the gap between temporal and baseline behavior.

## Reproduce in 5 minutes

```bash
# 1) Start dependencies
docker compose up -d redis qdrant

# 2) Start Mnemo server
cargo run --bin mnemo-server

# 3) Run baseline temporal pack
python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080

# 4) Run scientific research packs
python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080
python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080 --verbose
```

Optional cross-system run (requires `zep_api.key`):

```bash
python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key
```

## Scope and Caveats

- These tables are from controlled local runs and should be treated as reproducible engineering evidence, not broad production guarantees.
- Cross-system comparisons must include parity caveats because APIs are not 1:1 (see `docs/COMPETITIVE.md`).
- Retrieval latency includes memory context assembly path. It does not include asynchronous ingestion-side LLM extraction latency.

## Performance Targets

Targets are from the product roadmap and will be replaced by measured values as benchmark coverage expands.

### Latency Targets

| Operation | Target | Current Measured |
|---|---:|---:|
| Episode ingestion (API -> Redis) | <5ms | TBD |
| Context retrieval (API -> response) | <50ms | TBD |
| Semantic search (Qdrant round-trip) | <30ms | TBD |
| Full-text search (RediSearch) | <10ms | TBD |
| RRF fusion (in-process) | <1ms | TBD |

### Throughput Targets

| Operation | Target | Current Measured |
|---|---:|---:|
| Concurrent episode ingestion | 1000 eps/sec | TBD |
| Concurrent context retrieval | 500 req/sec | TBD |
| Background extraction (LLM-dependent) | provider-bound | TBD |

### Memory Footprint Targets

| Metric | Target | Current Measured |
|---|---:|---:|
| Base server footprint | <50MB RSS | TBD |
| Per 1K users (100 episodes each) Redis | <200MB | TBD |
| Per 1K users (100 episodes each) Qdrant | <500MB | TBD |

## Environment Template

Fill this block whenever publishing benchmark claims:

```text
CPU:
Memory:
OS:
Rust:
Redis:
Qdrant:
Docker:
Mnemo commit SHA:
Dataset:
Command:
```
