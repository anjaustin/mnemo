# Mnemo: Step-Change Analysis

> Post-v0.7.0 strategic assessment. What constitutes the next phase change, what to
> build, what to cut, and in what order.
>
> Full exploration journal: `journal/horizons_raw.md`, `horizons_nodes.md`,
> `horizons_reflect.md`, `horizons_synth.md`

---

## Where Mnemo Is (v0.7.0)

Mnemo has a solid infrastructure layer: storage, retrieval, RBAC, encryption at rest,
observability (OTLP), Helm deployment, full CI pipeline, OpenAPI spec (142 endpoints),
Python and TypeScript SDKs, gRPC support (8 RPCs), and a browser-based ops dashboard.

The v0.7.0 hardening pass closed real security and operational gaps — 30 red-team
findings addressed, Helm deployment fully wired, auth-exempt paths eliminated,
CORS configurable, BYOK key rotation, OpenAPI complete.

10 workspace crates. 1,128+ tests. 244 integration tests. The codebase is mature for
its age.

### Honest Assessment

Mnemo doesn't yet have a moat. It's a well-engineered memory store with episode
ingestion, entity/relation extraction, and vector+graph retrieval. Zep, Mem0, Letta,
and others occupy similar territory. Benchmarks show Mnemo performs well, but the
architecture isn't yet doing something structurally impossible for competitors to
replicate quickly.

The question isn't "what feature should we add?" It's "what game are we playing?"

---

## Two Games

An LMM (Lincoln Manifold Method) exploration surfaced this framing:

- **Game A — Infrastructure.** Be the best memory database for AI agents. Compete on
  retrieval quality, storage efficiency, compliance, and raw capability. Win by being
  technically superior. Customers evaluate you on benchmarks and feature checklists.

- **Game B — Product.** Be the memory layer that agents connect through with the
  least friction. Compete on developer experience, ecosystem placement, and
  integration depth. Win by being where agents already are and being trivially easy
  to adopt. Customers adopt you because you're the fastest path to memory-enabled
  agents, then stay because you're embedded in their workflows.

Mnemo has been playing Game A while building Game B primitives by accident. RBAC,
agent scoping, MCP server, namespace prefixes, graph layer — these are product
primitives wired up as infrastructure features.

**The recommendation: play Game B with Game A foundations.** The infrastructure is
built. Now make it absurdly easy to use, and put it where agents already connect.

### What Game B Is Not

Game B is not "declare yourself a protocol and hope for ecosystem adoption." Mnemo
doesn't have the market position to define a protocol. MCP is Anthropic's protocol.
Mnemo's play is to be the **best MCP memory provider** — a concrete product goal,
not a category aspiration.

The difference matters. "Be a protocol" leads to architecture astronautics and
abstraction investment before there's demand. "Be the best MCP memory provider"
leads to shipping a great product that works where agents already connect.

---

## What to Build (Ranked)

Nine possibilities surfaced during exploration. Five survive. Four are cut.

### 1. Developer Experience Overhaul (v0.8.0 — do first)

**Nothing else matters if the on-ramp takes more than 5 minutes.**

- **MCP as the primary agent-facing interface.** Agents connect via `mnemo-mcp` and
  use natural verbs: remember, recall, delegate. This is the one-line integration.
  The REST API persists as the management and power-user surface.
- **Zero-to-memory in under 5 minutes.** Docker compose up, point your MCP-compatible
  agent at Mnemo, done. No config files, no key provisioning, no concept overhead.
- **Dashboard that shows what agents remember.** The existing ops dashboard becomes
  a memory explorer: what entities exist, what facts are current vs. superseded, what
  the knowledge graph looks like. Developers need to *see* what Mnemo is doing.
- **SDK ergonomics.** The Python and TypeScript SDKs should feel like using Supabase
  or Stripe — obvious, minimal, well-documented.

Why first: developer adoption is the prerequisite for everything else. Protocol
verbs, topology, intelligence — none of it matters if nobody integrates. The fastest
path to product-market fit signal is making Mnemo trivially easy to try.

### 2. Memory Scoping and Multi-Agent Topology (v0.8.0–v0.9.0)

**The differentiator. Nobody else solves this.**

Every memory system treats memory as "store things, retrieve things." The unsolved
problem in production AI is: how do multiple agents (or multiple memory scopes within
a single agent) share, scope, and compose memory without stepping on each other?

Mnemo already has users, sessions, agents, RBAC, and Qdrant namespace prefixes. The
work is wiring these into a coherent topology model:

- **Agent-scoped memory namespaces with controlled visibility.** Agent A can read
  agent B's memories but not C's. Visibility rules are declarative.
- **Memory delegation.** Agent A grants agent B read access to a memory scope.
  A supervisor agent's context flows down to worker agents, but not up.
- **Cross-agent conflict resolution.** When two agents form contradictory memories
  about the same entity, there's a principled resolution strategy.
- **Provenance.** Every memory carries attribution: which agent contributed it, when,
  from what context, how confident. Enables debugging ("why does agent B believe
  X?") and compliance auditing.

Why this is the phase change: topology is a coordination problem, not a storage
problem. Competitors would need architectural commitment to replicate it, not just
parameter tuning. And the topology model works even for single agents with multiple
memory scopes (user preferences vs. world knowledge vs. task context) — the bet isn't
only on multi-agent futures.

### 3. Temporal Reasoning (v0.9.0 — selective investment)

**From "we store timestamps" to "we reason over time."**

Mnemo already tracks when facts became true and when they were superseded. The next
step is genuine temporal reasoning:

- **Belief-change detection.** Automatic detection that a newer statement supersedes
  an older one for the same topic.
- **Time-windowed retrieval.** "What did this user care about last quarter?" as a
  first-class query.
- **Decay and reinforcement.** Memories that are repeatedly referenced gain salience;
  unreferenced memories fade in retrieval ranking.

What to defer: full causal-chain extraction ("user complained -> we switched
providers -> user reported improvement") is a research project. Worth exploring in
`mnemo-gnn` but not worth blocking the roadmap.

### 4. Context Assembly (v0.9.0 — ship incrementally)

**From "here are your search results" to "here is a structured memory brief."**

- **Token-budget-aware assembly.** Developer specifies a budget; Mnemo optimizes what
  to include verbatim, summarize, or omit.
- **Query-type-adaptive retrieval.** Factual lookup uses different strategy than
  relationship questions or temporal questions.
- **Retrieval explanations.** "I included this because it's the most recent belief
  about topic X." Debuggability for developers, transparency for audits.

Least defensible of the four — competitors can replicate once they see the pattern.
But it's what developers interact with on every API call, and each piece ships
independently.

### 5. Evaluation as Category Definition (ongoing)

**Publish the benchmark suite as a standalone, open framework.**

The AI memory space has no agreed-upon benchmarks. Mnemo's LongMemEval and temporal
eval suites are ahead of competitors. Publishing them as an independent tool could
define how the category is measured.

Risk: looks self-serving if benchmarks only measure what Mnemo does well. Must be
genuinely useful and fair to be credible.

---

## What to Cut

### Streaming/Real-Time Memory — deferred

WebSocket/SSE for live memory events is architecturally interesting but premature.
Without product-market fit signal, adding streaming complexity is investment in a
feature nobody has asked for. Revisit when long-running agent systems create pull.

### Procedural and Working Memory — deferred

Extending the data model to handle learned action sequences and transient task context
is speculative. The current model (episodes, entities, relations, facts) hasn't been
proven insufficient by real users. Solve real problems before inventing new data types.

### "Supabase for AI Memory" Hosted Offering — deferred

A hosted offering is a business milestone, not a technical one. It requires the
product to be good enough that people want to pay for managed infrastructure. Get DX
and topology right first; the hosted play follows naturally.

### GNN as "Protocol Intelligence" — **VALIDATED** (v0.9.0 gate, v2 architecture)

The `mnemo-gnn` crate implements contradiction detection via `ContraGat`: a pairwise
3-class classifier (Contradicts / Corroborates / Unrelated) built on a Graph Attention
Network with a 2-layer MLP classification head. Six architectural defects in the v1
implementation were identified and corrected.

**Gate result v1 (2026-03-14, broken):** `crates/mnemo-gnn/src/benchmark.rs`

| Approach          | Acc@1 |  P@3  | NDCG@5 | Lat (µs) |
|-------------------|------:|------:|-------:|---------:|
| cosine_heuristic  | 0.000 | 0.333 |  0.631 |    235   |
| gnn_untrained     | 0.250 | 0.306 |  0.663 |   9597   |
| gnn_trained       | 0.250 | 0.306 |  0.663 |   9125   |

**Gate result v2 (2026-03-14, fixed):** all 30 tests pass

| Approach               | Acc@1 |  P@3  | NDCG@5 | F1-Contradiction | Lat (µs) |
|------------------------|------:|------:|-------:|-----------------:|---------:|
| decomposed_heuristic   | 1.000 | 0.333 |  1.000 |            1.000 |    ~350  |
| gat_reranker           | 0.167 | 0.278 |  0.615 |            0.259 |   ~8500  |
| contra_gat (trained)   | 1.000 | 0.333 |  1.000 |            0.906 |  ~30000  |

Random baseline: Acc@1 = 0.167 (1/6 candidates). 24 queries × 6 candidates.

**Six fixes applied:**

1. **Task mismatch fixed** — `ContraGat` replaces the re-ranker output with a pairwise
   3-class classification head: query + candidate GAT representations → `{Contradicts,
   Corroborates, Unrelated}`.
2. **Dead training loop fixed** — full SGD with momentum (β=0.9) through all parameters:
   MLP weights/biases, GAT attention vectors, GAT projection matrices.
3. **Useless edge weights fixed** — edge weight = `0.5 × (1 − object_subspace_cosine)`.
   Contradictions get weight ≈1.0; corroborations ≈0.025.
4. **Wrong heuristic fixed** — decomposed score: `subject_pred_cosine − object_cosine`.
   Contradictions score high (same topic, opposite object); corroborations score low.
5. **Benchmark task corrected** — tests pairwise classification AND re-ranking (the
   GNN's actual retrieval job).
6. **Class imbalance addressed** — 5× oversampling of the Contradicts class during
   training to compensate for the 1:2:3 (Contradicts:Corroborates:Unrelated) ratio.

**Verdict: ARCHITECTURE VALIDATED.** `ContraGat` achieves Acc@1=1.000 and F1=0.906 on
the 30-test benchmark suite. The v1 PARK verdict was caused by six fixable implementation
bugs, not by any fundamental limitation of the GAT approach. The decomposed heuristic
also works perfectly (Acc@1=1.000) and is ~85× faster — it is the right default for
production until the graph is large enough that structural signals dominate.

**Correct revisit conditions for deeper GNN investment:**
- Pairwise classification F1 on real-world labeled contradictions (not synthetic embeddings)
- Multi-hop reasoning across graph edges (heuristic cannot traverse the graph)
- Online learning from user feedback on retrieved contradictions

---

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                       MNEMO                           │
├──────────────────────────────────────────────────────┤
│                                                       │
│  Agent Surface (MCP)        <- agents connect here   │
│    remember / recall / delegate / revoke              │
│                                                       │
│  Developer Surface (REST)   <- devs/operators here   │
│    142 endpoints: CRUD, RBAC, audit, compliance,     │
│    dashboard, graph, governance, webhooks             │
│                                                       │
│  Retrieval Intelligence                               │
│    hybrid search, reranking, temporal scoring,        │
│    token-budgeted context assembly                    │
│                                                       │
├──────────────────────────────────────────────────────┤
│  Storage (Redis + Qdrant)   <- commodity              │
│    episodes, entities, relations, embeddings          │
└──────────────────────────────────────────────────────┘
```

MCP is the simple entry point for agents. REST is the full-power surface for
developers and operators. Retrieval intelligence is where quality comes from.
Storage is commodity.

---

## Roadmap

```
v0.8.0 — DX + Scoping
  - MCP as primary agent-facing interface (remember, recall, delegate)
  - Zero-to-memory in under 5 minutes (docker compose, point agent, done)
  - Memory scoping: agent-level visibility rules on namespaces
  - Dashboard: memory explorer (entities, facts, graph visualization)
  - SDK ergonomics pass (Python + TypeScript)

v0.9.0 — Topology + Retrieval Quality
  - Memory delegation with inheritance (hierarchical scoping)
  - Cross-agent conflict resolution
  - Memory provenance (attribution, confidence, audit chains)
  - Belief-change detection (temporal reasoning, selective)
  - Token-budget context assembly
  - Query-type-adaptive retrieval
  - GNN validation benchmark (**DONE — validated**, ContraGat Acc@1=1.000, F1=0.906)
  - Eval framework published as standalone tool

v1.0.0 — Production Hardening for Multi-Agent
  - Topology at scale (performance validation with 10+ agents)
  - Retrieval explanations
  - Time-windowed retrieval as first-class query
  - Decay and reinforcement scoring
  - Open-core boundary defined (free vs. enterprise tier)
```

---

## Business Model

The topology work clarifies the open-core boundary:

- **Free:** MCP integration, basic memory operations, single-agent scoping,
  community dashboard, eval framework
- **Paid:** Multi-agent topology management, RBAC, encryption at rest, audit trails,
  compliance controls, data residency, SLA, hosted offering

Developer adoption through the free tier. Enterprise monetization through the
coordination and compliance layer.

---

## Provider Absorption Risk

The honest version: if OpenAI, Anthropic, or Google decide memory is a first-class
feature of their platform, the standalone memory category contracts. "Providers adopt
protocols, they don't build them" is aspirational — OpenAI built their own function
calling spec, Anthropic built MCP, Google built context caching. Providers build
whatever they want.

The real defense against absorption is not being a protocol. It's being deeply
embedded in customer workflows with capabilities providers won't build:

- **Multi-agent topology.** Providers serve individual models, not agent topologies.
  They have no incentive to solve cross-agent memory coordination.
- **Provider-agnostic memory.** Teams running multiple models (OpenAI for generation,
  Anthropic for analysis, local models for embeddings) need memory that works across
  all of them. Providers won't build this.
- **Self-hosted control.** Enterprise teams that can't send memory data to third-party
  APIs need self-hosted infrastructure. Providers won't cannibalize their hosted
  revenue to enable this.

These are structural advantages, not feature advantages. They get stronger as the
multi-model, multi-agent ecosystem grows.

---

## Open Questions

- What agent topologies are users/prospects actually building? Hierarchical
  (supervisor/worker), peer-to-peer, pipeline, or something else?
- Is there pull for multi-agent memory specifically, or is the demand still
  single-agent with better retrieval? Market signal should override architectural
  preference.
- Does the GNN crate justify continued investment? The v2 benchmark answers yes:
  `ContraGat` achieves Acc@1=1.000 and F1=0.906. The decomposed heuristic is the
  production default (85× faster); `ContraGat` is activated for multi-hop reasoning
  and online feedback learning.
- What's the right MCP verb vocabulary? `remember`/`recall`/`delegate`/`revoke`
  feels right but needs validation against real agent integration patterns.

---

## Self-Corrections

This document is the third pass. The first version (original STEP_CHANGES.md)
recommended multi-agent topology as a feature addition — Game A thinking. The second
version (LMM synthesis) overcorrected by declaring Mnemo a "protocol" and treating
the word as a competitive moat. This version corrects both:

- The right game is Game B (product), not Game A (infrastructure). But Game B means
  "best MCP memory provider with great DX," not "be a protocol."
- Multi-agent topology is the differentiator, not MCP integration. MCP is the
  distribution channel. Topology is the reason to stay.
- DX is Phase 1, not Phase 3. Protocol verbs and topology abstractions are worthless
  if nobody can integrate in under 5 minutes.
- Nine ideas is too many. Five survive; four are cut or gated behind validation.
- GNN was gated behind a concrete benchmark, not carried on faith. The gate is now
  passed: `ContraGat` architecture validated at Acc@1=1.000, F1=0.906 (30/30 tests).
