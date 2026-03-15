# Mnemo Eval Framework

Benchmark any AI memory system using hand-curated case packs and deterministic
validation. Works out of the box against Mnemo, Zep, and any custom backend that
implements four methods.

```
python -m mnemo_eval --backend mnemo --base-url http://localhost:8080 --packs all
```

---

## Quick Start

**Prerequisites:** Python 3.10+, a running memory system

```bash
# Against a local Mnemo instance
python eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080

# Run all packs via the CLI entry point
python -m mnemo_eval --backend mnemo --base-url http://localhost:8080 --packs temporal,longmem

# Install as a package and use the mnemo-eval command
pip install -e eval/
mnemo-eval --backend mnemo --base-url http://localhost:8080 --packs all
```

---

## Harnesses

| Harness | Script | Packs / Cases | Description |
|---|---|---|---|
| Temporal eval | `temporal_eval.py` | temporal (27), scientific (10) | Temporal retrieval accuracy, stale fact detection, p50/p95 latency |
| LongMemEval | `longmem_eval.py` | longmem (~25 inline) | 5 task types: single-hop, multi-hop, temporal, preference, absent |
| Recall quality | `recall_quality.py` | recall (40 gold facts) | Factual accuracy, temporal correctness, p95 latency |
| Latency bench | `latency_bench.py` | latency (3 scale tiers) | E2E HTTP latency at 10/100/1000 episodes |

### Running a Harness Directly

Every harness accepts `--help` for the full flag list. Common flags:

```bash
# Mnemo target
python eval/temporal_eval.py \
    --target mnemo \
    --mnemo-base-url http://localhost:8080

# Verbose: show per-case pass/fail
python eval/temporal_eval.py \
    --target mnemo \
    --mnemo-base-url http://localhost:8080 \
    --verbose

# Write a result JSON file
python eval/temporal_eval.py \
    --target mnemo \
    --mnemo-base-url http://localhost:8080 \
    --output eval/results/my_run.json

# Zep target
python eval/temporal_eval.py \
    --target zep \
    --zep-base-url https://api.getzep.com/api/v2 \
    --zep-api-key $ZEP_API_KEY

# Both targets side-by-side
python eval/temporal_eval.py \
    --target both \
    --mnemo-base-url http://localhost:8080 \
    --zep-base-url https://api.getzep.com/api/v2 \
    --zep-api-key $ZEP_API_KEY

# Use a specific case pack
python eval/temporal_eval.py \
    --target mnemo \
    --mnemo-base-url http://localhost:8080 \
    --cases eval/scientific_research_cases_v2.json
```

---

## Case Packs

Case packs are JSON files in `eval/cases/` and the root `eval/` directory.

| Pack | File | Cases | Notes |
|---|---|---|---|
| temporal | `eval/temporal_cases.json` | 27 | Core temporal retrieval |
| scientific | `eval/scientific_research_cases_v2.json` | 10 | Research context with temporal structure |
| enterprise_crm | `eval/cases/enterprise_crm.json` | 30 | CRM: deal progression, role changes, absent facts |
| conversational | `eval/cases/conversational.json` | 25 | Preference tracking, corrections, long-gap recall |
| multi_agent | `eval/cases/multi_agent.json` | 20 | Agent scoping, cross-agent isolation, provenance |
| context_assembly | `eval/cases/context_assembly.json` | 15 | Query classification, token budgets, structured output |

### Case Pack Schema

```json
{
  "version": 1,
  "pack": "temporal",
  "description": "Brief description of this pack",
  "cases": [
    {
      "id": "temporal_001",
      "description": "Human-readable description of what this case tests",
      "setup": [
        {
          "content": "Episode text to ingest",
          "created_at": "2025-01-15T10:00:00Z"
        }
      ],
      "query": {
        "text": "Query text to send to the memory system",
        "mode": "temporal",
        "time_intent": "recent"
      },
      "expect": {
        "present": ["keyword1", "keyword2"],
        "absent": ["stale_keyword"]
      }
    }
  ]
}
```

### Adding New Cases

1. Add your case to an existing JSON file, or create a new file in `eval/cases/`
2. Follow the schema above
3. Choose a unique `id` (pack prefix + zero-padded number, e.g., `enterprise_crm_031`)
4. Include at least one `absent` case (fact was never stored) — prevents benchmark overfitting
5. Test locally before submitting a PR

For temporal cases, `created_at` must be an ISO 8601 UTC timestamp. Use past dates
relative to your test run — the harness does not mock the clock.

---

## Running Against a Non-Mnemo System

### Option 1: Use an existing backend

```bash
# Zep (supported out of the box)
python eval/temporal_eval.py \
    --target zep \
    --zep-base-url https://api.getzep.com/api/v2 \
    --zep-api-key $ZEP_API_KEY
```

### Option 2: Implement a custom backend

1. Copy `eval/backends/custom_backend.py` and rename it
2. Implement the four abstract methods: `setup_user`, `ingest`, `query`, `cleanup`
3. Import your backend and pass it to any harness

```python
# my_backend.py
from backends.custom_backend import CustomBackend

class MySystemBackend(CustomBackend):
    name = "mysystem"

    def setup_user(self, external_id):
        # ... create user and session in your system
        return user_id, session_id

    def ingest(self, user_id, session_id, content, created_at=None):
        # ... store content in your system
        return True

    def query(self, user_id, session_id, query, profile="default"):
        # ... retrieve context from your system
        return 200, context_text, latency_ms

    def cleanup(self, user_id, session_id):
        # ... delete test data
        pass
```

Then run a harness with your backend directly (pass it as the `backend` argument
to the harness's `run()` function — see each harness script's `if __name__ == "__main__"`
section for the wiring pattern).

See `eval/backends/custom_backend.py` for the full template with inline documentation.

---

## Persistent Results and Regression Detection

Every harness writes a JSON result file to `eval/results/` when `--output` is set.
The file name format is: `{workflow}_{commit[:8]}_{timestamp}.json`

```bash
# Write result files
python eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8080 \
    --output eval/results/temporal_$(git rev-parse --short HEAD)_$(date -u +%Y%m%dT%H%M%S).json

# Compare two result files
python eval/compare.py eval/results/baseline.json eval/results/current.json

# Compare and fail if any metric regressed
python eval/compare.py eval/results/baseline.json eval/results/current.json \
    --fail-on-regression

# Auto-compare the two most recent result files
python eval/compare.py --latest
```

### Result File Schema

```json
{
  "version": 1,
  "commit": "abc123ef",
  "branch": "main",
  "timestamp": "2026-03-14T12:00:00Z",
  "workflow": "benchmark-eval",
  "system": "mnemo",
  "metrics": {
    "temporal_accuracy": 0.97,
    "temporal_stale_rate": 0.02,
    "temporal_p50_ms": 12,
    "temporal_p95_ms": 45
  },
  "gates": {
    "temporal_accuracy": {"value": 0.97, "threshold": 0.95, "passed": true}
  }
}
```

### Regression Tolerances

| Metric type | Tolerance |
|---|---|
| Accuracy / rate metrics | ±2 percentage points |
| Latency metrics | ±20% relative |
| Gate pass→fail | Always a regression (zero tolerance) |

---

## CI Integration

The `benchmark-eval.yml` workflow runs on every PR and push to `main`:

1. Builds the Mnemo server
2. Runs temporal eval, scientific pack, and LongMemEval
3. Writes D1-format result files and uploads them as artifacts
4. Downloads the baseline result from the previous `main` run
5. Runs `compare.py --fail-on-regression` to detect regressions
6. Writes the comparison table to the GitHub Step Summary

The latency bench runs on `main` pushes and manual dispatch only (not on every PR —
it takes longer and requires a populated dataset).

---

## Package Installation

The eval framework can be installed as a standalone Python package:

```bash
# Install from the repo
pip install -e eval/

# Run the CLI
mnemo-eval --help
mnemo-eval --list-packs
mnemo-eval --backend mnemo --base-url http://localhost:8080 --packs temporal,longmem

# List available packs
mnemo-eval --list-packs
```

Zero runtime dependencies. The package uses stdlib only.

---

## Methodology

See [METHODOLOGY.md](METHODOLOGY.md) for:
- What the benchmark measures and what it does NOT measure
- How to interpret accuracy and latency results
- How to run competitor systems honestly
- Known limitations
- How to contribute cases
