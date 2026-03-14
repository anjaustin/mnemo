# Spec 04: Context Assembly

> Target: v0.9.0 (ship incrementally)
> Priority: What developers interact with on every API call. Each piece stands alone.

---

## Problem

Mnemo's context retrieval returns a block of text assembled from relevant episodes,
entities, and facts. The developer specifies `max_tokens` and Mnemo fills the budget.
This works but is unsophisticated:

- No distinction between query types (factual lookup, relationship question, temporal
  question) — all use the same retrieval strategy
- The assembled context is a flat text block with no structure — the LLM must parse
  it to understand what's a fact vs. what's narrative vs. what's metadata
- No explanation of why specific memories were included — developers can't debug
  retrieval quality without manual inspection
- Token budgeting is binary (include or exclude) — no summarization of lower-priority
  memories to fit more signal into the budget

## What Exists Today

| Capability | Status | Location |
|---|---|---|
| Hybrid retrieval (semantic + graph + lexical) | Implemented | `mnemo-retrieval` |
| Reranking (RRF, MMR, cross-encoder) | Implemented | `mnemo-retrieval` |
| Token budgeting (`max_tokens` parameter) | Implemented | `routes.rs` |
| Memory contracts (SupportSafe, CurrentStrict, etc.) | Implemented | `routes.rs` |
| Retrieval policies (balanced, precision, recall, stability) | Implemented | `routes.rs` |
| `temporal_weight` parameter | Implemented | `routes.rs` |
| `min_relevance` threshold | Implemented | `routes.rs` |
| Retrieval policy diagnostics in response | Implemented | context response |
| Temporal diagnostics in response | Implemented | context response |
| Goal-conditioned retrieval | Implemented | `routes.rs` |
| Head context (most recent session only) | Implemented | `routes.rs` |

## Deliverables

### D1: Query Classification

**Classify incoming queries into types and select retrieval strategy accordingly.**

Query types:

| Type | Example | Strategy |
|---|---|---|
| `factual` | "What is Jordan's email?" | Entity lookup → exact match → single fact |
| `relationship` | "How does Jordan relate to the Acme deal?" | Graph traversal → BFS from entities → edge summary |
| `temporal` | "What changed about Acme since January?" | Time-windowed → changes_since → chronological |
| `summary` | "Give me context on Jordan's accounts" | Hybrid → balanced retrieval → narrative assembly |
| `absent` | "Does Jordan have any legal issues?" | Hybrid → if low confidence, explicit "no data found" |

**Implementation:**
- Add `QueryClassifier` in `mnemo-retrieval` that uses keyword heuristics +
  optional LLM classification:
  - Keywords: "what is", "who is" → factual; "how does...relate" → relationship;
    "what changed", "since", "before" → temporal; "context on", "summary of" → summary
  - If heuristics are ambiguous and LLM is available, use a single-shot classification
    prompt (cheap, < 100 tokens)
  - Fall back to `summary` if classification fails
- Route each type to a specialized retrieval path in the pipeline
- Include `query_type` in the context response for transparency

**Non-goal:** The classifier doesn't need to be perfect. The fallback (`summary`)
is the current behavior, so misclassification degrades gracefully to status quo.

### D2: Structured Context Response

**Replace the flat text block with a structured response that separates concerns.**

Current response:
```json
{
  "text": "Jordan works at Acme Corp. Acme renewal is at risk...",
  "token_count": 487,
  "entities": [...],
  "facts": [...]
}
```

Proposed response (additive — `text` field stays for backward compatibility):
```json
{
  "text": "Jordan works at Acme Corp. Acme renewal is at risk...",
  "token_count": 487,
  "entities": [...],
  "facts": [...],
  "structured": {
    "key_facts": [
      {"subject": "Acme", "predicate": "renewal_status", "object": "at_risk",
       "valid_at": "2025-02-15T00:00:00Z", "confidence": 0.92}
    ],
    "recent_changes": [
      {"fact": "renewal_status: green → at_risk", "changed_at": "2025-02-15T00:00:00Z",
       "source_episode": "ep-uuid"}
    ],
    "relationships": [
      {"from": "Jordan", "relation": "manages", "to": "Acme", "confidence": 0.95}
    ],
    "open_questions": [
      "No data on whether SOC 2 evidence has been submitted"
    ]
  }
}
```

**Implementation:**
- Add `StructuredContext` struct in `mnemo-retrieval`
- Populate `key_facts` from the top-ranked valid facts by relevance
- Populate `recent_changes` from facts superseded within the last 30 days
- Populate `relationships` from graph edges connected to query-relevant entities
- Populate `open_questions` from absent-detection (entities mentioned in query
  but not found in memory, or facts with low confidence)
- Add `structured` field to context response (optional — only included when
  `?structured=true` query parameter is set, to avoid response size bloat for
  callers who don't need it)

**SDK update:** Add `structured: bool` parameter to `context()` in both SDKs.
Add typed response models for `StructuredContext`.

### D3: Retrieval Explanations

**For each fact/entity included in the context, explain why it was included.**

```json
{
  "explanations": [
    {
      "fact_id": "edge-uuid",
      "reason": "most_recent_belief",
      "detail": "Most recent valid fact for subject 'Acme' with predicate 'renewal_status'"
    },
    {
      "fact_id": "edge-uuid-2",
      "reason": "graph_connection",
      "detail": "Connected to query entity 'Jordan' via 'manages' relationship (1 hop)"
    },
    {
      "fact_id": "edge-uuid-3",
      "reason": "temporal_relevance",
      "detail": "Fact created within requested time window (last 30 days)"
    }
  ]
}
```

**Reason types:**
- `semantic_match` — vector similarity above threshold
- `graph_connection` — connected to a query entity via graph traversal
- `most_recent_belief` — latest valid fact for a subject-predicate pair
- `temporal_relevance` — within the requested time window
- `contract_required` — included because the memory contract mandates it
- `reinforced` — high access count (from Spec 03 D3)

**Implementation:**
- During retrieval, annotate each result with its retrieval reason
- Include explanations in the context response when `?explain=true`
- Store explanations in an `ExplanationCollector` that accumulates reasons as
  results flow through the pipeline stages

**SDK update:** Add `explain: bool` parameter to `context()`. Add typed
`RetrievalExplanation` model.

### D4: Summarization Tier in Token Budgeting

**Current behavior:** Token budgeting includes facts verbatim until the budget is
exhausted, then stops. Lower-ranked facts are dropped entirely.

**Proposed behavior:** Three tiers within the token budget:

```
Token Budget (e.g., 2000 tokens)
├── Tier 1: Verbatim (top 60% of budget) — highest-ranked facts included as-is
├── Tier 2: Compressed (next 25%) — medium-ranked facts summarized to ~30% of
│   their original token count by the LLM
└── Tier 3: Mentioned (final 15%) — lowest-ranked facts listed as one-line
    references: "Also relevant: [entity] [predicate] [object] (Jan 2025)"
```

**Implementation:**
- After ranking, partition results into three tiers by score
- Tier 1: include verbatim (current behavior)
- Tier 2: if LLM is available, send a summarization prompt. If not, truncate to
  first sentence. Budget: ~30% of original tokens.
- Tier 3: format as single-line references (fixed format, no LLM needed)
- Make tier ratios configurable: `MNEMO_CONTEXT_TIER1_RATIO` (default 0.6),
  `MNEMO_CONTEXT_TIER2_RATIO` (default 0.25), `MNEMO_CONTEXT_TIER3_RATIO` (0.15)

**Fallback:** If LLM is unavailable, Tier 2 falls back to first-sentence truncation.
The system still works without an LLM for summarization.

---

## Non-Goals

- **Agentic retrieval (multi-turn search).** The context endpoint is a single-turn
  query. Multi-turn retrieval (search, evaluate, refine) is an application-level
  concern, not a memory infrastructure concern.
- **Custom summarization prompts.** The tier-2 summarization prompt is internal.
  Developers who want custom summarization should process the raw retrieval results.
- **Streaming context assembly.** SSE/WebSocket streaming of context as it's
  assembled. Deferred per STEP_CHANGES.md.

## Risks

1. **Query classification accuracy.** If the classifier frequently misroutes queries,
   developers will get worse results than status quo. Mitigate: aggressive fallback
   to `summary` (current behavior) and include `query_type` in response so developers
   can see and override.
2. **Structured response size.** The `structured` field adds significant JSON to
   the response. Mitigate: opt-in via `?structured=true`, not default.
3. **Tier-2 summarization latency.** Adding an LLM call to context assembly adds
   latency. Mitigate: only invoke for Tier 2 (not the primary results), and make it
   optional (disabled if LLM is unavailable or if `?summarize=false`).
4. **Explanation overhead.** Tracking retrieval reasons through the pipeline adds
   code complexity. Mitigate: `ExplanationCollector` is a lightweight append-only
   struct that's only instantiated when `?explain=true`.

## Success Criteria

- [ ] Query classifier routes queries to specialized retrieval paths
- [ ] Factual queries return single facts instead of full narratives
- [ ] Temporal queries use time-windowed retrieval automatically
- [ ] Structured context response available via `?structured=true`
- [ ] Retrieval explanations available via `?explain=true`
- [ ] Tiered token budgeting compresses mid-ranked results instead of dropping them
- [ ] All existing retrieval quality gates still pass
- [ ] p95 latency increase from classification < 15ms (heuristic path)
- [ ] p95 latency increase from structured assembly < 25ms
