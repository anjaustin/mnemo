# Spec 05: Evaluation as Category Definition

> Target: Ongoing (ships incrementally across v0.8.0 through v1.0.0)
> Priority: Credibility infrastructure. If we can't measure it, we can't claim it.

---

## Problem

Mnemo has a solid eval infrastructure for an early-stage project: 5 harnesses, 4 CI
workflows, quality gates on temporal accuracy and stale rate. But it's all internal
plumbing — results live as ephemeral CI logs, there's no regression detection across
runs, and the harnesses are coupled to Mnemo internals in ways that prevent external
use.

The strategic opportunity (per STEP_CHANGES.md) is to turn eval from an internal
quality tool into a category-defining artifact. The AI memory space has no standard
benchmark. Everyone self-reports numbers on private datasets. If Mnemo publishes a
rigorous, reproducible eval framework that works against any memory system, we define
what "good" means — and we happen to score well on our own benchmark.

This only works if the benchmarks are genuinely useful and not self-serving. The moment
the eval framework feels like marketing, it loses all credibility.

## What Exists Today

### Eval Harnesses (5)

| Harness | File | Lines | What It Tests |
|---|---|---|---|
| Temporal eval | `eval/temporal_eval.py` | 412 | Temporal retrieval accuracy, stale fact rate, p50/p95 latency |
| LongMemEval | `eval/longmem_eval.py` | 583 | 5 task types: single-hop, multi-hop, temporal, preference, absent |
| Import stress | `eval/import_stress.py` | 325 | Bulk import throughput, failure rates |
| Recall quality | `tests/eval_recall_quality.py` | 659 | 40 gold facts: factual recall, temporal correctness, p95 latency |
| Live fleet falsify | `tests/live_fleet_falsify.py` | 149 | Smoke test against 8 live deployment targets |

### CI Workflows (4)

| Workflow | File | Trigger | What It Runs |
|---|---|---|---|
| quality-gates | `.github/workflows/quality-gates.yml` | PR + push to main | fmt, clippy, unit tests, integration tests, e2e smoke, temporal quality budget |
| memory-falsification | `.github/workflows/memory-falsification.yml` | PR + push to main | 244 memory API integration tests |
| benchmark-eval | `.github/workflows/benchmark-eval.yml` | PR + push + manual | temporal eval (2 case packs), LongMemEval, optional Zep comparison |
| nightly-soak | `.github/workflows/nightly-soak.yml` | cron 06:00 UTC + manual | 3x memory API soak, temporal + scientific packs |

### Quality Gates (enforced in CI)

| Gate | Threshold | Source |
|---|---|---|
| Temporal accuracy | >= 95% | `quality-gates.yml` line 180 |
| Stale fact rate | <= 5% | `quality-gates.yml` line 182 |
| p95 retrieval latency | <= 300ms | `quality-gates.yml` line 184 |
| LongMemEval single-hop | >= 80% | `longmem_eval.py` |
| LongMemEval multi-hop | >= 70% | `longmem_eval.py` |
| LongMemEval temporal | >= 75% | `longmem_eval.py` |
| LongMemEval preference | >= 80% | `longmem_eval.py` |
| LongMemEval absent precision | >= 90% | `longmem_eval.py` |
| Recall accuracy (full embeds) | >= 85% | `eval_recall_quality.py` |
| Temporal correctness | >= 90% | `eval_recall_quality.py` |
| p95 latency | <= 500ms | `eval_recall_quality.py` |

### Case Packs (3 JSON files + inline cases)

| Pack | File | Cases |
|---|---|---|
| Temporal | `eval/temporal_cases.json` | 3 cases |
| Scientific v2 | `eval/scientific_research_cases_v2.json` | 10 cases |
| Scientific v1 | `eval/scientific_research_cases.json` | (original, superseded by v2) |
| LongMemEval inline | `eval/longmem_eval.py` CASES list | ~20+ inline cases |
| Recall quality inline | `tests/eval_recall_quality.py` GOLD_FACTS | 40 gold facts |

### Backend Abstraction

`temporal_eval.py` defines a `Backend` ABC and `MnemoBackend` implementation. The
benchmark-eval workflow already supports manual Zep comparison via `--target zep`
and `--target both`. This is the seed of multi-system eval.

### What's Missing

1. **No persistent metrics store.** Results are CI logs. There's no way to query
   "what was our temporal accuracy 10 commits ago?" or "is p95 latency trending up?"
2. **No automated regression detection.** A human must manually compare CI output
   between runs to spot degradation.
3. **Harnesses are spread across two directories** (`eval/` and `tests/`) with
   duplicated HTTP client code and inconsistent patterns.
4. **Not usable outside Mnemo.** The harnesses import Mnemo-specific code paths
   (e.g., `longmem_eval.py` imports from `temporal_eval.py`). No standalone package,
   no `pip install`, no docs for running against a non-Mnemo system.
5. **Limited case diversity.** 3 temporal cases + 10 scientific + ~20 LongMemEval +
   40 gold facts = ~73 total cases. That's enough for CI gating but not enough to
   claim category-defining benchmarks.
6. **No multi-agent eval.** All cases are single-user, single-agent. The scoping and
   topology work (Spec 02) needs eval coverage.
7. **No context assembly eval.** No cases test whether context budgeting, query
   classification, or structured responses (Spec 04) actually improve retrieval
   quality.
8. **Criterion bench is storage-only.** `benches/latency.rs` benches episode ingestion
   and context retrieval at the storage layer, not through the HTTP API. No E2E
   latency benchmarks.

---

## Deliverables

### D1: Persistent Metrics Store (v0.8.0)

**Store eval results in a structured format so they can be compared across runs.**

Implementation:

- Add `eval/results/` directory (gitignored). Each eval run writes a JSON file:
  ```
  eval/results/{workflow}_{commit_sha}_{timestamp}.json
  ```

- Schema per result file:
  ```json
  {
    "version": 1,
    "commit": "abc123",
    "branch": "main",
    "timestamp": "2026-03-14T12:00:00Z",
    "workflow": "benchmark-eval",
    "system": "mnemo",
    "results": {
      "temporal_accuracy": 0.97,
      "temporal_stale_rate": 0.02,
      "temporal_p50_ms": 12,
      "temporal_p95_ms": 45,
      "longmem_single_hop": 0.90,
      "longmem_multi_hop": 0.75,
      "longmem_temporal": 0.80,
      "longmem_preference": 0.85,
      "longmem_absent": 0.95
    },
    "gates": {
      "temporal_accuracy": {"value": 0.97, "threshold": 0.95, "passed": true},
      "temporal_stale_rate": {"value": 0.02, "threshold": 0.05, "passed": true}
    }
  }
  ```

- CI workflows emit this file as a build artifact (GitHub Actions `upload-artifact`).
  A post-eval step in each workflow writes the JSON and uploads it.

- Add `eval/compare.py` — reads two result files and prints a diff table:
  ```
  Metric             Before   After    Delta    Status
  temporal_accuracy  95.0%    97.0%    +2.0%    improved
  temporal_p95_ms    52ms     45ms     -7ms     improved
  longmem_multi_hop  75.0%    72.0%    -3.0%    REGRESSED
  ```

**Non-goal:** No database. No dashboard. JSON files + a diff script. Simplicity is the
point — this must be maintainable without infrastructure.

**Verification:** Run an eval, check the JSON file was written. Run it again on a
different commit, run `compare.py`, and confirm the diff is accurate.

### D2: Automated Regression Detection in CI (v0.8.0)

**Fail the CI pipeline if any metric regresses beyond a tolerance.**

Implementation:

- Add a regression check step to `benchmark-eval.yml` after the eval runs:
  1. Download the most recent `main` branch result artifact (using
     `actions/download-artifact` or `gh api` to fetch the latest artifact from
     the default branch)
  2. Run `eval/compare.py --baseline {previous} --current {current} --fail-on-regression`
  3. Regression = any gate metric that was previously passing now fails, OR any
     numeric metric drops by more than the tolerance:
     - Accuracy metrics: tolerance = 2 percentage points
     - Latency metrics: tolerance = 20%
     - Pass/fail gates: zero tolerance (pass → fail = always a regression)

- The first run on a branch with no baseline artifact skips regression detection
  (no previous result to compare against) and just enforces the static gates.

- PR comments: the regression check step writes a summary to `$GITHUB_STEP_SUMMARY`
  with the comparison table so reviewers can see the impact.

**Non-goal:** No persistence service. No time-series DB. The artifact store is the
persistence layer, and GitHub Actions retention (90 days) is sufficient.

**Verification:** Introduce a deliberate regression (e.g., skip a retrieval step),
push to a PR branch, and confirm CI fails with the regression table in the summary.

### D3: Eval Framework Consolidation (v0.8.0)

**Unify the eval harnesses into a consistent, standalone-ready structure.**

Current problems:
- `eval_recall_quality.py` is in `tests/` but isn't a Rust test
- `live_fleet_falsify.py` is in `tests/` but is a deployment probe
- `longmem_eval.py` imports from `temporal_eval.py` via `sys.path` hacking
- Each harness has its own HTTP client class
- No shared config, no shared output format

Changes:

1. **Move all eval harnesses to `eval/`:**
   - `tests/eval_recall_quality.py` → `eval/recall_quality.py`
   - `tests/live_fleet_falsify.py` → `eval/live_fleet_falsify.py`
   - Update CI workflow references accordingly

2. **Extract shared `eval/lib.py`:**
   - `HttpClient` class (currently duplicated in `temporal_eval.py` and
     `import_stress.py`)
   - `Backend` ABC and `MnemoBackend` (currently in `temporal_eval.py`)
   - `ResultWriter` — writes the D1 JSON result format
   - Common constants (timeouts, default URLs)

3. **Standardize output:**
   - Every harness writes a JSON result file (D1 format) via `ResultWriter`
   - Every harness prints a human-readable summary table to stdout
   - Every harness exits 0 on pass, 1 on gate failure

4. **Add `eval/README.md`:**
   - What each harness tests
   - How to run locally: `python3 eval/temporal_eval.py --mnemo-base-url http://localhost:8080`
   - How to run against non-Mnemo systems (using `--target` flag)
   - How to add new cases
   - How to write a new Backend implementation

**Non-goal:** No `pip install`, no PyPI package yet. That's D6. This deliverable is
about making the eval code clean and consistent internally.

**Verification:** All 4 CI workflows still pass after the moves. Each harness produces
a valid JSON result file.

### D4: Expanded Case Packs (v0.9.0)

**Increase case diversity to make benchmarks meaningful, not just CI gates.**

New case packs:

1. **Enterprise CRM pack** (`eval/cases/enterprise_crm.json`, ~30 cases):
   - Deal progression tracking (Acme Q1 → Q2 → renewal at risk)
   - Contact role changes (Jordan moved from engineer to VP)
   - Multi-stakeholder relationship queries
   - Temporal: "What was the deal status before the reorg?"
   - Absent: "Does Jordan have a signed contract?" (no data)

2. **Conversational assistant pack** (`eval/cases/conversational.json`, ~25 cases):
   - User preference tracking across sessions
   - Correction handling ("Actually I said Portland, not Seattle")
   - Multi-turn context (query references earlier conversation)
   - Long-gap recall (fact stored 50+ episodes ago)

3. **Multi-agent topology pack** (`eval/cases/multi_agent.json`, ~20 cases):
   - Agent A stores fact, Agent B queries (should/shouldn't see based on scoping)
   - Supervisor agent sees aggregated context from workers
   - Cross-agent contradiction detection
   - Provenance: "Which agent reported this fact?"
   - Requires Spec 02 agent scoping to be implemented first

4. **Context assembly pack** (`eval/cases/context_assembly.json`, ~15 cases):
   - Factual vs. relationship vs. temporal query classification accuracy
   - Token budget compliance (requested 500 tokens, got <= 500)
   - Structured response validation (sections present, no mixing)
   - Summarization quality (compressed context still contains key facts)
   - Requires Spec 04 context assembly features to be implemented first

Target totals after expansion: ~73 existing + ~90 new = ~163 cases.

**Verification:** Each new pack runs through the existing harness infrastructure.
All new cases have `expect` blocks that can be automatically validated.

### D5: E2E Latency Benchmarks (v0.9.0)

**Benchmark retrieval latency through the HTTP API, not just the storage layer.**

The existing `benches/latency.rs` Criterion bench goes through `RedisStateStore`
directly. This misses HTTP overhead, middleware, auth, reranking, and context assembly.

Implementation:

- Add `eval/latency_bench.py`:
  - Starts from a pre-populated user with N episodes (N = 10, 100, 1000)
  - Measures: episode ingestion latency (POST), context retrieval latency (POST),
    entity lookup latency (GET), graph query latency (POST)
  - Reports p50, p95, p99 for each operation at each scale
  - Writes D1-format JSON result file

- Add scale tiers:
  ```
  Tier    Episodes   Expected p95 Context
  small   10         < 100ms
  medium  100        < 300ms
  large   1000       < 1000ms
  ```

- Wire into `benchmark-eval.yml` as an optional step (runs on `main` pushes
  and manual dispatch, not on every PR — too slow).

**Non-goal:** This is not a load test. It's a single-client latency profile at
different data scales. Import stress (`import_stress.py`) already covers throughput.

**Verification:** Run locally, confirm latency numbers are within expected ranges
for local dev (which will be slower than CI).

### D6: Standalone Eval Package (v0.9.0 → v1.0.0)

**Publish the eval framework so anyone can benchmark their memory system.**

This is the category-definition move. It only works if:
1. The framework is genuinely useful for evaluating any memory system, not just Mnemo
2. Adding a new backend is trivial (implement 4-5 methods)
3. The case packs test real capabilities, not Mnemo-specific features
4. Results are reproducible

Implementation:

1. **Backend interface** (already exists as `Backend` ABC, needs formalization):
   ```python
   class MemoryBackend(ABC):
       @abstractmethod
       def setup_user(self) -> tuple[str, str]:
           """Create a user and session. Return (user_id, session_id)."""
       @abstractmethod
       def ingest(self, user_id: str, session_id: str, content: str,
                  created_at: str | None = None) -> None:
           """Store an episode."""
       @abstractmethod
       def query(self, user_id: str, query: str,
                 mode: str = "default", **kwargs) -> str:
           """Retrieve context for a query. Return the context text."""
       @abstractmethod
       def cleanup(self, user_id: str, session_id: str) -> None:
           """Delete test data."""
   ```

2. **Ship as `eval/` directory with a `pyproject.toml`** for optional `pip install`:
   ```toml
   [project]
   name = "mnemo-eval"
   version = "0.1.0"
   description = "AI Memory System Evaluation Framework"
   requires-python = ">=3.10"
   # Zero runtime deps — stdlib only
   ```

3. **Provide reference backends:**
   - `MnemoBackend` (already exists)
   - `ZepBackend` (seed exists in temporal_eval.py `--target zep`)
   - Skeleton `CustomBackend` with instructions

4. **CLI entry point:**
   ```bash
   python -m mnemo_eval --backend mnemo --base-url http://localhost:8080 --packs temporal,longmem,enterprise_crm
   # Or after pip install:
   mnemo-eval --backend mnemo --base-url http://localhost:8080 --packs all
   ```

5. **Output:** JSON result file + human-readable table + optional Markdown report
   suitable for blog posts / README badges.

**Non-goal:** No hosted leaderboard. No web UI. The output is files that people can
publish wherever they want.

**Verification:** Install from the local `eval/` directory, run against a live Mnemo
instance, confirm results match running the scripts directly.

### D7: Credibility Strategy (ongoing)

**Ensure the eval framework is trustworthy, not self-serving.**

Principles:

1. **Include cases where Mnemo is likely to perform poorly.** Multi-hop reasoning
   with long chains, very large context windows, queries requiring world knowledge
   that isn't in the stored episodes. If Mnemo scores 100% on everything, the
   benchmark isn't hard enough.

2. **Document methodology transparently.** Every case pack includes a rationale
   for why those cases were chosen. Every threshold includes a justification. The
   README explains what the benchmark does and does NOT measure.

3. **Encourage external contributions.** Case packs accept PRs. The framework
   is Apache-2.0 licensed (same as Mnemo).

4. **Publish Mnemo's scores alongside limitations.** When writing about benchmark
   results, always include: what version was tested, what configuration was used,
   what the known weaknesses are, and what the benchmark does NOT test.

5. **Run competitor systems honestly.** The Zep backend already exists. When
   publishing comparative results, use the same cases, same infrastructure, same
   methodology. No cherry-picking.

6. **Version the benchmark.** Case packs are versioned (v1, v2). Results specify
   which version they were run against. This prevents gaming through retroactive
   case modification.

Concrete actions:
- Every case pack JSON file includes a `"version"` field
- `eval/METHODOLOGY.md` explains the evaluation approach (written with D6)
- Comparative results include full reproduction instructions

---

## Sequencing

```
v0.8.0
  D1: Persistent metrics store (JSON files + compare script)
  D2: Regression detection in CI
  D3: Eval consolidation (move files, extract shared lib, standardize output)

v0.9.0
  D4: Expanded case packs (enterprise CRM, conversational, multi-agent, context assembly)
  D5: E2E latency benchmarks
  D6: Standalone eval package (pyproject.toml, CLI entry point, reference backends)

v1.0.0
  D6: (continued) Polish and publish
  D7: Credibility strategy (methodology docs, competitor benchmarks, versioning)
```

D1–D3 are prerequisites: consolidate and instrument before expanding. D4–D5 expand
coverage to match the new capabilities from Specs 02–04. D6–D7 turn internal tooling
into an external asset.

---

## Dependencies

| Deliverable | Depends On |
|---|---|
| D1 (metrics store) | None |
| D2 (regression detection) | D1 |
| D3 (consolidation) | None (but natural to ship with D1) |
| D4 multi-agent pack | Spec 02 (agent scoping implemented) |
| D4 context assembly pack | Spec 04 (query classification + structured response) |
| D5 (E2E latency) | None (uses existing HTTP API) |
| D6 (standalone package) | D3, D4 |
| D7 (credibility) | D6 |

---

## What We're NOT Doing

1. **No hosted leaderboard or web dashboard.** The output is JSON files and CLI
   tables. A web leaderboard is a maintenance burden and invites gaming.

2. **No LLM-as-judge evaluation.** All cases use deterministic string-match or
   structural validation. LLM-as-judge is non-reproducible and adds cost.
   If we add it later, it's a separate optional mode, not a gate.

3. **No synthetic data generation.** Case packs are hand-curated. Synthetic
   generation risks Goodhart's law — optimizing for generated patterns rather
   than real memory workloads.

4. **No performance optimization work.** This spec is about measurement, not
   improvement. Performance improvements belong in Specs 03 and 04.

5. **No GNN-specific eval.** The GNN crate is gated behind a validation benchmark
   (per STEP_CHANGES.md). If GNN eval is needed, it gets its own case pack added
   to D4, but only after the GNN work itself is justified.
