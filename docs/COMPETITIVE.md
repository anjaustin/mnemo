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

## Scenario packs

- Temporal preference changes
- Point-in-time recall (`as_of`)
- Contradiction/supersession handling
- Active-thread "what now" recall

Current local pack: `eval/temporal_cases.json`.

## Scorecard

| System | Temporal Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) | Setup to first recall (min) |
|---|---:|---:|---:|---:|---:|
| Mnemo (temporal mode) | _pending public run_ | _pending public run_ | _pending public run_ | _pending public run_ | _pending public run_ |
| Mnemo (baseline mode) | _pending public run_ | _pending public run_ | _pending public run_ | _pending public run_ | _pending public run_ |
| Zep | _pending public run_ | _pending public run_ | _pending public run_ | _pending public run_ | _pending public run_ |

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
