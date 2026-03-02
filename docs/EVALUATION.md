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

Run Mnemo + Zep comparison (requires `zep_api.key`):

```bash
python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key
```

For Mnemo it executes two profiles over the same temporal dataset:

- `temporal`: uses `mode`, `time_intent`, `as_of`, and temporal weighting
- `baseline`: same queries without temporal controls

For Zep it runs baseline-style memory retrieval on the same scenario pack.

Current dataset: `eval/temporal_cases.json`.

Expected output format:

```text
| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |
|---|---:|---:|---:|---:|
| temporal | ... | ... | ... | ... |
| baseline | ... | ... | ... | ... |
```

## Latest local snapshot

From a local run on 2026-03-02 (`python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080`):

| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |
|---|---:|---:|---:|---:|
| temporal | 100.0% | 0.0% | 84 | 84 |
| baseline | 66.7% | 33.3% | 80 | 80 |

Interpretation: temporal controls improved correctness and reduced stale recall in this dataset with a small latency tradeoff.

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
