# Mnemo Benchmark Results

Reproducible benchmark results for Mnemo v0.7.0. All results produced by the
[eval framework](eval/README.md) using deterministic string-match validation — no
LLM-as-judge. See [eval/METHODOLOGY.md](eval/METHODOLOGY.md) for full methodology,
regression tolerances, and known limitations.

---

## Configuration

| Parameter | Value |
|---|---|
| Mnemo version | 0.7.0 |
| Embedding provider | local (`AllMiniLML6V2`) |
| Embedding dimensions | 384 |
| LLM provider | anthropic (`claude-haiku-4-20250514`) |
| Commit | `86f21bd` |
| Run date | 2026-03-15 |

---

## LongMemEval

**16 cases across 5 task types.** Tests factual recall, multi-hop reasoning,
temporal awareness, user preference tracking, and absent-fact precision.

| Task Type | Accuracy | Gate | p95 Latency | Status |
|---|---:|---:|---:|---|
| Single-hop factual | 100.0% | ≥ 80% | 40ms | PASS |
| Multi-hop reasoning | 100.0% | ≥ 70% | 29ms | PASS |
| Temporal recall | 100.0% | ≥ 75% | 41ms | PASS |
| Preference tracking | 100.0% | ≥ 80% | 47ms | PASS |
| Absent-fact precision | 100.0% | ≥ 90% | 31ms | PASS |

All gates pass. Latency well under the 300ms gate across all task types.

Result file: [`eval/results/longmem_mnemo.json`](eval/results/longmem_mnemo.json)

---

## Temporal Eval (Core Pack — 27 cases)

Tests recency (return the latest version of a changing fact), point-in-time recall
(return the version that was true at a specified past date), and stale-fact suppression
(do not return facts that have been superseded).

Two retrieval profiles are evaluated:
- **temporal** — `temporal_weight: 0.9`, explicit `time_intent` and `mode` parameters passed
- **baseline** — standard retrieval, no temporal parameters

| Profile | Accuracy | Stale Rate | Gate (accuracy) | Gate (stale) | p95 Latency |
|---|---:|---:|---|---|---:|
| temporal | **85.2%** | **3.7%** | 95% — MISS | 5% — PASS | 66ms |
| baseline | 77.8% | 11.1% | — | — | 59ms |

The temporal profile improves accuracy by 7.4 percentage points and cuts stale rate
from 11.1% to 3.7% vs baseline. The 95% gate is not yet met; 4 of 27 cases fail
on point-in-time historical queries (returning the current version of a fact when
asked for the historical state).

Result files: [`eval/results/temporal_mnemo.json`](eval/results/temporal_mnemo.json) (pre-Spec 08, 27 cases), [`eval/results/temporal_mnemo_spec08.json`](eval/results/temporal_mnemo_spec08.json) (post-Spec 08, 31 cases)

---

## Temporal Eval (Scientific Pack — 10 cases)

Harder temporal reasoning over slowly-evolving research notes: protocol version
changes across months, belief revisions, multi-hop synthesis.

| Profile | Accuracy | Stale Rate | p95 Latency |
|---|---:|---:|---:|
| temporal | 60.0% | 40.0% | 62ms |
| baseline | 40.0% | 40.0% | 58ms |

**Honest assessment:** This is a genuine weakness. Cases where a fact evolves slowly
over many months (e.g., a research protocol updated four times across a year) produce
high stale rates — the older version is ranked higher than the newer one even with
explicit temporal weighting. This is a known limitation of cosine-similarity-based
retrieval combined with AllMiniLML6V2; semantically similar facts cluster together
and recency scoring alone is insufficient for fine-grained disambiguation.

The temporal profile (60%) significantly outperforms baseline (40%), confirming
that `as_of`/`time_intent` parameters are doing real work — but 60% is not
production-ready for this scenario.

Result file: [`eval/results/scientific_mnemo.json`](eval/results/scientific_mnemo.json)

---

## Recall Quality (40 gold facts)

Tests factual accuracy and temporal correctness over 40 hand-curated gold facts
distributed across 4 synthetic users.

| Gate | Value | Threshold | Status |
|---|---:|---:|---|
| Factual recall accuracy | 87.5% | 85% | PASS |
| Temporal correctness | 66.7% | 90% | FAIL |
| p95 retrieval latency | 56ms | 500ms | PASS |

**Factual recall (87.5%):** 35/40 facts retrieved. Failures were on vague query
phrasings (e.g., "Does Alice have any food allergies?" when the stored fact was
"Alice is allergic to shellfish" — keyword match on "shellfish" missed).

**Temporal correctness (66.7%):** 2/3 temporal gold facts retrieved correctly. One
failure: historical query for Carol's transportation method returned the current
episode text but keyword extraction produced an adjacent phrase rather than the
target keyword. This is a keyword-match harness artifact as much as a product gap.

Result file: [`eval/results/recall_mnemo.json`](eval/results/recall_mnemo.json)

---

## Summary Table

| Harness | Pack | Cases | Score | Gate | Status |
|---|---|---:|---:|---|---|
| LongMemEval | single-hop | 4 | 100.0% | ≥ 80% | PASS |
| LongMemEval | multi-hop | 3 | 100.0% | ≥ 70% | PASS |
| LongMemEval | temporal | 3 | 100.0% | ≥ 75% | PASS |
| LongMemEval | preference | 3 | 100.0% | ≥ 80% | PASS |
| LongMemEval | absent | 3 | 100.0% | ≥ 90% | PASS |
| Temporal eval | core (31, post-Spec 08) | 31 | 83.9% | ≥ 95% | MISS |
| Temporal eval | scientific (10) | 10 | 60.0% | ≥ 95% | MISS |
| Recall quality | factual | 40 | 87.5% | ≥ 85% | PASS |
| Recall quality | temporal | 3 | 66.7% | ≥ 90% | FAIL |
| Latency | recall p95 | — | 56ms | ≤ 500ms | PASS |
| Latency | longmem p95 | — | 47ms | ≤ 300ms | PASS |

---

## Latency Profile

All latency measurements are end-to-end HTTP round-trips including embedding,
vector search, context assembly, and JSON serialization. Single-client; no concurrency.

| Harness | p50 | p95 | Gate |
|---|---:|---:|---|
| Temporal eval (core) | 44ms | 66ms | ≤ 300ms PASS |
| Temporal eval (scientific) | 51ms | 62ms | ≤ 300ms PASS |
| LongMemEval | ~38ms | 47ms | ≤ 300ms PASS |
| Recall quality | 40ms | 56ms | ≤ 500ms PASS |

---

## Competitor Comparison

Zep was not benchmarked in this run — a live Zep instance is required and no API
key is currently configured. The eval framework supports Zep via `--target zep`; see
[eval/README.md](eval/README.md) for instructions.

**Historical note from previous evaluation sessions:** When Zep CE was run against
the temporal case pack with its default session-memory adapter (no explicit temporal
query parameters), temporal accuracy was 0% — it has no `as_of` or `time_intent`
equivalent. Zep's Graphiti graph backend provides temporal awareness at the graph
layer but does not expose point-in-time query parameters at the retrieval API level.

This comparison is directionally informative but should be reproduced with a live
Zep instance before being cited. See [eval/METHODOLOGY.md](eval/METHODOLOGY.md)
§ "Running Competitor Systems Honestly".

---

## What the Failing Gates Mean

Mnemo does not pass all its own gates in this run. The two significant gaps:

**1. Temporal accuracy on core pack (83.9% on 31 cases after Spec 08)**

Spec 08 (commit following `86f21bd`) added 4 regression cases targeting `as_of`
hard-filter consistency and all 4 new cases pass. The 5 remaining failures are
current-intent queries where a semantically similar older fact outscores the newer
one — a ranking issue that `as_of` filtering does not address (no cutoff applies
when `time_intent=current`).

**Root cause:** soft temporal scoring (Spec 03) applies a multiplier in `[0.2, 2.0]`
but cosine similarity differences between similar facts can exceed this range. When
the old fact is 40ms closer to the query embedding than the new one, the temporal
multiplier doesn't overcome it.

**Next:** stronger temporal differentiation — explicit invalidation metadata,
supersession detection, or embedding fine-tuning on temporal ordering signals.

**2. Scientific pack stale rate (40%)**

Slowly-evolving facts (protocol revisions over many months) produce high stale rates.
AllMiniLML6V2 embeds semantically similar research notes into a tight cluster;
temporal scoring alone cannot separate them reliably. D1+D2 (Spec 08) do not fix
this because the cases use `time_intent=current` without `as_of`, so no hard filter
is applied.

**Planned fix:** Per-field decay (facts with explicit supersession markers decay faster)
and/or homeoadaptive LoRA personalization (Spec 06/07) can help agents that provide
feedback signals — but this requires instrumentation at the agent level.

---

## Reproducing These Results

```bash
# Prerequisites: Mnemo server running with 384-dim local embeddings
MNEMO_SERVER_PORT=8081 \
MNEMO_QDRANT_PREFIX=mnemo_eval384_ \
MNEMO_EMBEDDING_PROVIDER=local \
MNEMO_EMBEDDING_MODEL=AllMiniLML6V2 \
MNEMO_EMBEDDING_DIMENSIONS=384 \
target/debug/mnemo-server &

# Run all harnesses
python3 eval/temporal_eval.py --target mnemo --mnemo-base-url http://localhost:8081 \
    --output eval/results/temporal_mnemo.json

python3 eval/temporal_eval.py --target mnemo \
    --cases eval/scientific_research_cases_v2.json \
    --mnemo-base-url http://localhost:8081 \
    --output eval/results/scientific_mnemo.json

python3 eval/longmem_eval.py --mnemo-base-url http://localhost:8081 \
    --output eval/results/longmem_mnemo.json

python3 eval/recall_quality.py --server http://localhost:8081 \
    --output eval/results/recall_mnemo.json
```

Full methodology: [eval/METHODOLOGY.md](eval/METHODOLOGY.md)
