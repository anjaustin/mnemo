# Eval Methodology

This document explains what the Mnemo eval framework measures, what it does **not** measure,
how to interpret results, and how the benchmark is versioned.

---

## What This Benchmark Measures

The eval framework tests whether a memory system can reliably store, retrieve, and reason
over episodic memory under conditions that approximate real AI agent workloads.

Five harnesses, six case packs:

| Harness | Script | Cases | What It Tests |
|---|---|---|---|
| Temporal eval | `temporal_eval.py` | 27 (temporal) + 10 (scientific v2) | Recency, point-in-time recall, stale fact detection |
| LongMemEval | `longmem_eval.py` | ~25 inline | Single-hop, multi-hop, temporal, preference, absent-fact precision |
| Recall quality | `recall_quality.py` | 40 gold facts | Factual accuracy, temporal correctness, p95 latency |
| Latency bench | `latency_bench.py` | 3 scale tiers | E2E ingestion + retrieval latency at 10/100/1000 episodes |
| Enterprise CRM | `cases/enterprise_crm.json` | 30 | Deal progression, contact role changes, absent facts |
| Conversational | `cases/conversational.json` | 25 | Preference tracking, correction handling, long-gap recall |
| Multi-agent | `cases/multi_agent.json` | 20 | Agent scoping, cross-agent isolation, provenance |
| Context assembly | `cases/context_assembly.json` | 15 | Query classification, token budget compliance, structure |

**Total: ~162 cases across 8 packs.**

### Evaluation Approach

All cases use **deterministic string-match or structural validation**. There is no
LLM-as-judge scoring. A case passes or fails based on whether the retrieved context
contains specified keywords, excludes specified keywords (for absent-fact tests), or
meets a numeric threshold (latency, token count).

This is a deliberate design choice. LLM-as-judge introduces:
- Non-reproducibility across model versions
- Cost that scales with the benchmark corpus
- Evaluation bias that is hard to characterize

Deterministic validation is reproducible, fast, and free. It may miss subtle quality
differences in natural-language output, but it catches the regressions that matter:
facts that should be present aren't, facts that should be absent appear, or latency
spikes.

---

## Quality Gates

Gates are hard thresholds that CI enforces on every push:

| Gate | Threshold | Rationale |
|---|---|---|
| Temporal accuracy | >= 95% | Below this, time-sensitive agents make incorrect decisions |
| Stale fact rate | <= 5% | Surfacing outdated facts is worse than surfacing nothing |
| p95 retrieval latency | <= 300ms | The memory call is on the critical path of an agent turn |
| LongMemEval single-hop | >= 80% | Basic factual recall; a failing system is not useful |
| LongMemEval multi-hop | >= 70% | Multi-hop is harder; 70% is a meaningful signal |
| LongMemEval temporal | >= 75% | Time-awareness is a core differentiator |
| LongMemEval preference | >= 80% | User preferences are high-value, high-recall target |
| LongMemEval absent precision | >= 90% | Hallucinating absent facts is a trust-breaking failure mode |
| Recall accuracy | >= 85% | Factual recall on 40 hand-curated gold facts |
| Temporal correctness | >= 90% | Gold facts include temporal assertions |
| p95 latency (recall) | <= 500ms | Full-stack latency including context assembly overhead |

Thresholds are **intentionally strict**. A system that barely passes these gates is
not delivering good memory. These are minimum bars, not performance targets.

---

## Case Pack Versioning

Each case pack JSON file includes a `"version"` field. When cases are modified in
a way that changes what scores are expected (new cases, changed expected answers,
changed thresholds), the version is incremented.

Result files record the commit SHA and timestamp, not the pack version directly.
If you need to compare results across pack versions, note the pack version in your
test documentation.

Current versions:
- `temporal_cases.json`: v1
- `scientific_research_cases_v2.json`: v2
- `cases/enterprise_crm.json`: v1
- `cases/conversational.json`: v1
- `cases/multi_agent.json`: v1
- `cases/context_assembly.json`: v1

---

## What This Benchmark Does NOT Measure

1. **World knowledge.** The benchmark tests recall of facts that were explicitly
   stored. It does not test whether the system can answer questions about things
   that were never ingested. That is the LLM's job, not the memory system's.

2. **Semantic reasoning quality.** Case validation is keyword-based. A context that
   contains the expected keywords but is otherwise incoherent will pass. Semantic
   quality assessment requires human evaluation or LLM-as-judge, neither of which
   is in scope here.

3. **Throughput or concurrency.** Latency benchmarks are single-client. `import_stress.py`
   covers throughput separately. For production capacity planning, run load tests with
   tools designed for that purpose (k6, Locust, etc.).

4. **Storage durability.** Cases assume a running server with a live store. There are
   no tests for crash recovery, disk persistence guarantees, or replication consistency.

5. **Authorization and access control.** Multi-agent cases test scoping (which agent
   can see which facts) but do not test authentication, encryption, or RBAC.

6. **Model-dependent behavior.** Mnemo uses local embeddings (`AllMiniLML6V2`) by default.
   Results with a different embedding model or different LLM may differ. All published
   results must state the embedding model and LLM configuration used.

---

## How to Interpret Results

### Accuracy metrics (0.0–1.0)
Values are fractions. 0.95 means 95% of test cases passed. Higher is better.
The threshold column shows the minimum passing value.

### Latency metrics (milliseconds)
p50 is the median latency. p95 is the 95th percentile — the latency that 95% of
requests fall within. Lower is better. Latency is measured from the start of the
HTTP request to the completion of the response parse, through the full stack
(HTTP, routing, embedding, vector search, context assembly).

### Gate columns
`passed: true` means the metric is above (or below, for latency) threshold.
`passed: false` is a CI failure. If a gate that previously passed is now failing,
`compare.py` reports it as a regression regardless of the numeric tolerance.

### Regression detection (`compare.py`)
Two result files are compared:
- **Accuracy metrics**: regression if current drops more than 2 percentage points below baseline
- **Latency metrics**: regression if current increases more than 20% above baseline
- **Gate pass/fail**: pass→fail is always a regression; fail→pass is always an improvement

These tolerances exist because natural variance in CI timing and test ordering
causes small fluctuations. A 1pp accuracy drop on 27 cases (1 case) is likely noise.
A 3pp drop is worth investigating.

---

## Running Competitor Systems Honestly

The `ZepBackend` implementation in `lib.py` runs the same case packs against Zep.
When publishing comparative results:

1. Use the same case packs, same version, same infrastructure (same CI runner, same
   hardware tier if running locally)
2. Report the system version under test (Mnemo v0.1.x, Zep vX.Y.Z)
3. Report the configuration used (embedding model, LLM, any tuning parameters)
4. Include the full result JSON files, not just the summary table
5. Do not cherry-pick case packs. If Mnemo performs poorly on multi-hop cases,
   that must be reported alongside the cases where it performs well

If a competitor's API does not support a feature required by a case pack
(e.g., temporal queries, agent scoping), mark those cases as `N/A` rather than
counting them as failures. A system cannot fail a test it was never designed to pass.

---

## Contributing Cases

Case additions are welcome. New cases must:

1. Be hand-curated (no synthetic generation)
2. Include a rationale comment explaining why the case is meaningful
3. Use the standard JSON schema (see any existing `cases/*.json` file)
4. Include at least one case where the expected behavior is "absent" (no relevant
   fact stored) — to prevent benchmark overfitting toward recall
5. Not assume Mnemo-specific features unless the case is in the `multi_agent` or
   `context_assembly` packs (which explicitly require Spec 02/04 features)

Cases that reveal Mnemo weaknesses are particularly valuable. If a case type
consistently fails, that is a product signal, not a benchmark problem.

---

## Known Limitations of This Benchmark

- **27 temporal cases is a small corpus.** A single misclassified case moves the
  temporal accuracy score by ~3.7 percentage points. We are aware of this and accept
  it as a current limitation. The scientific pack adds 10 more temporally-structured
  cases.

- **Keyword matching misses paraphrases.** A context that says "Mr. Chen relocated to
  Austin" will not match an expected keyword of "moved to Austin". Cases are written
  to use keywords that a well-functioning system would surface verbatim, but there
  will be edge cases.

- **Multi-agent cases require Spec 02.** The `multi_agent.json` pack contains cases
  marked `scoped_only: true` that require agent scoping features from Spec 02. These
  cases are skipped by harnesses that do not implement agent-scoped queries. Published
  scores must note which cases were run and which were skipped.

- **Context assembly cases are Mnemo-specific.** The `context_assembly.json` pack
  tests Spec 04 features (query classification, structured response, token budgeting)
  that are not standard in other memory systems. These cases are appropriate for
  internal regression testing but should not be used in cross-system comparisons
  without equivalent feature support on the competing system.
