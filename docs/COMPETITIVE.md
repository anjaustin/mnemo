# Competitive Benchmark Plan

This document defines how Mnemo publishes apples-to-apples benchmark results against other memory systems.

## Objective

Show measurable user outcomes, not architecture claims:

- higher temporal accuracy
- lower stale-fact rate
- predictable latency
- faster developer setup

## Baseline methodology

1. Use the same scenario set and prompts for all systems.
2. Use equivalent model providers and limits where possible.
3. Record full environment metadata (machine, region, model, retries).
4. Publish both aggregate metrics and raw run artifacts.

Command used for side-by-side runs:

```bash
python3 eval/temporal_eval.py --target both --mnemo-base-url http://localhost:8080 --zep-api-key-file zep_api.key
```

## Methodology notes

Important parity caveats for Mnemo vs Zep:

1. API semantics are not 1:1.
   - Mnemo supports explicit temporal controls (`mode`, `time_intent`, `as_of`).
   - Zep memory retrieval is session-driven and does not expose equivalent knobs in the same endpoint.

2. Query protocol differs.
   - Mnemo recall uses explicit query payload.
   - Zep baseline adapter appends the query as a message and then fetches session memory context.

3. Results should be interpreted by outcome category, not strict parameter parity.
   - We compare temporal correctness, stale-fact behavior, and latency under equivalent scenario prompts.

4. Every published table must include run IDs and environment details.

## Scenario packs

- Temporal preference changes
- Point-in-time recall (`as_of`)
- Contradiction/supersession handling
- Active-thread "what now" recall

Current local pack: `eval/temporal_cases.json`.

## Scorecard

| System | Temporal Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) | Setup to first recall (min) | Evidence |
|---|---:|---:|---:|---:|---:|
| Mnemo (temporal mode) | 100.0% | 0.0% | 51 | 51 | _pending_ | `benchmark-eval` push run `22591312119` |
| Mnemo (baseline mode) | 66.7% | 33.3% | 48 | 48 | _pending_ | `benchmark-eval` push run `22591312119` |
| Zep (baseline adapter) | _pending successful run_ | _pending successful run_ | _pending successful run_ | _pending successful run_ | _pending_ | manual run `22591413221` failed: missing `ZEP_API_KEY` secret |

## Run log

- 2026-03-02: Captured Mnemo benchmark results from GitHub Actions push run `22591312119`.
- 2026-03-02: Triggered manual `both` run `22591413221`; Mnemo section executed, Zep section blocked due missing repository secret `ZEP_API_KEY`.

## Internal snapshot (for development)

From `eval/temporal_eval.py` local run on 2026-03-02 (Mnemo target):

| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |
|---|---:|---:|---:|---:|
| temporal | 100.0% | 0.0% | 84 | 84 |
| baseline | 66.7% | 33.3% | 80 | 80 |

These are directional and not cross-system claims until the same harness is run against external systems.

## Publishing standard

Any competitive claim in README/site should include:

1. date stamp
2. dataset version
3. command/workflow used
4. link to raw results
