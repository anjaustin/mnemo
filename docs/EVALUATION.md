# Evaluation Playbook

This is the reproducible path for proving Mnemo's memory quality and reliability claims.

## What we measure

- Temporal accuracy (current vs historical correctness)
- Stale fact rate (wrong old facts resurfacing)
- Latency (p50/p95 for memory context)

## Local temporal benchmark

Run the built-in harness (Mnemo only):

```bash
python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080
```

For per-case diagnostics during falsification:

```bash
python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080 --verbose
```

Run Mnemo + Zep comparison (requires `zep_api.key`):

```bash
python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key
```

## CI automation

Workflow: `.github/workflows/benchmark-eval.yml`

- PR / push to `main`: runs Mnemo benchmark (`--target mnemo`).
- PR / push to `main`: runs Mnemo benchmark on default pack and scientific research pack v2.
- Manual dispatch: supports `mnemo`, `zep`, or `both`.

Nightly stability soak workflow: `.github/workflows/nightly-soak.yml`

- runs memory API falsification three times in sequence (flake detection)
- runs temporal and scientific v2 eval packs with `--verbose`

For Zep runs in GitHub Actions, configure repository secret:

- `ZEP_API_KEY`

Latest CI evidence:

- Mnemo benchmark success: run `22591312119`
- Manual `both` attempt: run `22591413221` (failed on missing `ZEP_API_KEY` secret)
- Manual `both` rerun after setting secret: run `22591534300` (Zep adapter executed but returned `errors=3` across cases)

For Mnemo it executes two profiles over the same temporal dataset:

- `temporal`: uses `mode`, `time_intent`, `as_of`, and temporal weighting
- `baseline`: same queries without temporal controls

For Zep it runs baseline-style memory retrieval on the same scenario pack.

Default dataset: `eval/temporal_cases.json`.

Scientific research assistance dataset (Michael Levin style scenarios):

```bash
python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080
```

This pack emphasizes evolving research claims, hypothesis updates, and historical vs current retrieval correctness.

Scientific research assistance v2 dataset (harder contradiction and synthesis cases):

```bash
python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080
```

Expected output format:

```text
| System | Profile | Accuracy | Stale Fact Rate | Errors | p50 Latency (ms) | p95 Latency (ms) |
|---|---|---:|---:|---:|---:|---:|
| mnemo | temporal | ... | ... | ... | ... | ... |
| mnemo | baseline | ... | ... | ... | ... | ... |
| zep | baseline | ... | ... | ... | ... | ... |
```

## Latest local snapshot

From a local run on 2026-03-02 (`python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080`):

| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |
|---|---:|---:|---:|---:|
| temporal | 100.0% | 0.0% | 84 | 84 |
| baseline | 66.7% | 33.3% | 80 | 80 |

Interpretation: temporal controls improved correctness and reduced stale recall in this dataset with a small latency tradeoff.

## Latest scientific pack snapshot

From local runs on 2026-03-02:

`python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080`

| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |
|---|---:|---:|---:|---:|
| temporal | 100.0% | 0.0% | 94 | 124 |
| baseline | 50.0% | 50.0% | 80 | 121 |

Interpretation: domain-shaped scientific cases show a larger gap between temporal and baseline retrieval, with baseline stale-fact exposure reaching 50%.

From local runs on 2026-03-02:

`python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080`

| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |
|---|---:|---:|---:|---:|
| temporal | 100.0% | 0.0% | 78 | 106 |
| baseline | 50.0% | 40.0% | 75 | 105 |

Interpretation: v2 raises difficulty with denser contradiction and synthesis cases. Temporal retrieval still materially outperforms baseline. During falsification, we identified and fixed one scorer false-positive in v2 expectation tokens (substring overlap between `2.5 uM` and `5 uM`), then reran the pack.

## Importer stress snapshot

Real export dataset used:

- `downloads/6957c8e02c797beeb082b42e1f53a0d4f97ed813369f7b25376485225dded6b4-2025-10-21-02-29-50-e815fa493cfa481c941b2165f06911b9.zip`
- importable messages: `8945`
- conversations: `47`

Local run commands:

```bash
python3 eval/import_stress.py --mode dry-run --iterations 1 --base-url http://localhost:8080
python3 eval/import_stress.py --mode import --iterations 1 --base-url http://localhost:8080
```

Observed local throughput snapshot (single-iteration):

| Mode | Imported Messages | Total Job Time (ms) | Throughput (messages/sec) | Failed Messages |
|---|---:|---:|---:|---:|
| dry-run | 8945 | 1573 | 5686.59 | 0 |
| import | 8945 | 13109 | 682.36 | 0 |

## Latest max falsification sweep

From a full local sweep on 2026-03-03:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo check --workspace`
- `cargo test --workspace --lib --bins`
- `MNEMO_TEST_REDIS_URL=redis://localhost:6379 MNEMO_TEST_QDRANT_URL=http://localhost:6334 cargo test --workspace --tests -- --test-threads=1`
- `cargo test -p mnemo-server --test memory_api -- --test-threads=1` (repeated x3 for soak)
- `python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080`
- `python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases.json --mnemo-base-url http://localhost:8080`
- `python3 eval/temporal_eval.py --target mnemo --cases eval/scientific_research_cases_v2.json --mnemo-base-url http://localhost:8080 --verbose`
- `bash tests/e2e_smoke.sh http://localhost:8080`
- `python3 eval/import_stress.py --mode dry-run --iterations 1 --base-url http://localhost:8080`
- `python3 eval/import_stress.py --mode import --iterations 1 --base-url http://localhost:8080`

Observed outcomes:

- workspace and integration suites passed (`memory_api`: 23/23, `ingest`: 3/3, `storage`: 6/6)
- memory API soak loop passed 3/3 consecutive runs
- deterministic smoke passed (9/9 checks)
- scientific v2 remained stable (`temporal` 100% accuracy, 0% stale; `baseline` 50% accuracy, 40% stale)
- importer stress remained green with no row failures:
  - dry-run: 8945 messages, 1414 ms, 6326.03 msgs/sec
  - import: 8945 messages, 9820 ms, 910.90 msgs/sec

Note: `mnemo-ingest` integration tests require `MNEMO_TEST_REDIS_URL` to match a reachable Redis instance (CI uses `redis://localhost:6379`).

## Competitive runbook (Mnemo vs Zep)

1. Run the same scenario set end-to-end on both systems.
2. Capture result tables and raw logs.
3. Publish numbers with exact environment details (hardware, model provider, dataset, retries).
4. Track trend over time in release notes.

Recommended scorecard columns:

- temporal accuracy
- stale fact rate
- contradiction error rate
- p50/p95 context latency
- setup friction (minutes to first working memory turn)

## Narrative guidance

Position claims around outcomes, not internals:

- "fewer stale answers"
- "better point-in-time recall"
- "same-day production reliability"

This keeps the story user-relevant and falsifiable.
