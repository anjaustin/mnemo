# Benchmarks

## Status

Benchmark automation is active and publishing results through:

- `eval/temporal_eval.py`
- `.github/workflows/benchmark-eval.yml`

Current measured snapshots:

Baseline temporal pack (local run, 2026-03-02):

| System | Profile | Accuracy | Stale Fact Rate | Errors | p50 Latency (ms) | p95 Latency (ms) |
|---|---|---:|---:|---:|---:|---:|
| mnemo | temporal | 100.0% | 0.0% | 0 | 106 | 106 |
| mnemo | baseline | 66.7% | 33.3% | 0 | 81 | 81 |

Scientific research pack v2 (local run, 2026-03-02):

| System | Profile | Accuracy | Stale Fact Rate | Errors | p50 Latency (ms) | p95 Latency (ms) |
|---|---|---:|---:|---:|---:|---:|
| mnemo | temporal | 100.0% | 0.0% | 0 | 78 | 106 |
| mnemo | baseline | 50.0% | 40.0% | 0 | 75 | 105 |

Interpretation:

- Mnemo temporal mode consistently outperforms Mnemo baseline on this dataset.
- Scientific-domain cases widen the baseline-vs-temporal quality gap and are now part of routine falsification.

---

## Methodology

All benchmarks are run with:
- **Tool:** [Criterion.rs](https://github.com/bheisler/criterion.rs) for microbenchmarks, `hey` or `wrk` for HTTP throughput
- **Infrastructure:** Docker Compose (Redis Stack + Qdrant) on the same machine
- **Warm-up:** 5 iterations discarded before measurement
- **Samples:** 100 iterations minimum
- **Metrics:** P50, P95, P99 latency; throughput (ops/sec)

### How to Reproduce

```bash
# Start infrastructure
docker compose up -d redis qdrant

# Run eval harness against local Mnemo
cargo run --bin mnemo-server &
sleep 3
python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080

# Run scientific research packs
python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080
python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080 --verbose

# Optional: side-by-side with Zep (requires API key)
python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key

# Legacy microbenchmarks
cargo bench

# Run HTTP benchmarks (requires server running)
cargo run --release --bin mnemo-server &
sleep 2

# Throughput test: context retrieval
hey -n 1000 -c 50 -m POST \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"What does the user prefer?"}]}' \
  http://localhost:8080/api/v1/users/USER_ID/context

# Throughput test: episode ingestion
hey -n 5000 -c 100 -m POST \
  -H "Content-Type: application/json" \
  -d '{"type":"message","role":"user","content":"Test message"}' \
  http://localhost:8080/api/v1/sessions/SESSION_ID/episodes
```

---

## Targets

These are the performance targets from the PRD. Use `docs/EVALUATION.md` and `docs/COMPETITIVE.md` for the latest run evidence and methodology notes.

### Latency

| Operation | Target | P50 | P95 | P99 |
|-----------|--------|-----|-----|-----|
| Episode ingestion (API → Redis) | <5ms | TBD | TBD | TBD |
| Context retrieval (API → response) | <50ms | TBD | TBD | TBD |
| Semantic search (Qdrant round-trip) | <30ms | TBD | TBD | TBD |
| Entity extraction (LLM round-trip) | Measure only | TBD | TBD | TBD |
| Full-text search (RediSearch) | <10ms | TBD | TBD | TBD |
| RRF fusion (in-process) | <1ms | TBD | TBD | TBD |

### Throughput

| Operation | Target | Measured |
|-----------|--------|----------|
| Concurrent episode ingestion | 1000 eps/sec | TBD |
| Concurrent context retrieval | 500 req/sec | TBD |
| Background extraction (with LLM) | Depends on LLM | TBD |

### Memory

| Metric | Target | Measured |
|--------|--------|----------|
| Base server footprint | <50MB RSS | TBD |
| Per 1K users (100 episodes each) | <200MB Redis | TBD |
| Per 1K users (100 episodes each) | <500MB Qdrant | TBD |

---

## Hardware Spec

*To be filled when benchmarks are run:*

```
CPU:     
Memory:  
OS:      
Rust:    
Redis:   
Qdrant:  
Docker:  
```

---

## Notes

- Episode ingestion latency measures the synchronous API call (store in Redis + add to pending queue). It does **not** include the asynchronous LLM extraction step.
- Context retrieval latency includes embedding generation, Qdrant search, RediSearch full-text search, graph traversal, RRF fusion, and context string assembly.
- LLM extraction latency is dominated by the external API call and varies by provider (OpenAI ≈ 500ms–2s, Ollama local ≈ 200ms–5s depending on model).
- All latency measurements use pre-warmed connections (connection pool already established).
