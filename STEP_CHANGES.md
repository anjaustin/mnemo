# Mnemo: Step-Change Analysis

> Post-v0.7.0 strategic assessment. Where Mnemo is, where it needs to go, and what
> constitutes a genuine phase change versus incremental improvement.

## Current Position (v0.7.0)

Mnemo has a **solid infrastructure layer**: storage, retrieval, RBAC, encryption at
rest, observability (OTLP), Helm deployment, full CI pipeline, OpenAPI spec (142
endpoints), Python and TypeScript SDKs, gRPC support (8 RPCs), and a browser-based
ops dashboard.

The v0.7.0 hardening pass closed real security and operational gaps — 30 red-team
findings addressed, Helm deployment fully wired, auth-exempt paths eliminated,
CORS configurable, BYOK key rotation, OpenAPI complete.

**10 workspace crates**, **1,128+ tests**, **244 integration tests** — the codebase
is mature for its age.

### Honest Assessment

Mnemo doesn't yet have a moat. It's a well-engineered memory store with episode
ingestion, entity/relation extraction, and vector+graph retrieval. Zep, Mem0, Letta,
and others occupy similar territory. Benchmarks show Mnemo performs well, but the
architecture isn't yet doing something structurally impossible for competitors to
replicate quickly.

---

## Step-Change Candidates

### 1. Multi-Agent Memory Topology (Highest Leverage)

**Category-defining, not feature-adding.**

Every memory system today — including Mnemo — treats memory as "store things, retrieve
things." The real unsolved problem in production AI is: **how do multiple agents share,
scope, and compose memory without stepping on each other?**

Mnemo already has users, sessions, agents, and RBAC. It's closer to this than anyone
in the space.

#### What This Means Concretely

- **Agent-scoped memory namespaces with controlled visibility.** Agent A can read
  agent B's memories but not C's. Visibility rules are declarative, not ad-hoc.
- **Memory delegation and inheritance.** A supervisor agent's context flows down to
  worker agents, but not up. Hierarchical scoping that mirrors real agent topologies
  (fan-out, pipelines, peer groups).
- **Cross-agent conflict resolution.** When two agents form contradictory memories
  about the same entity, there's a principled resolution strategy — not just
  "last write wins."
- **Shared memory surfaces.** Agents can publish memories to shared namespaces
  (e.g., "team knowledge") with explicit provenance tracking (which agent contributed
  what, and when).

#### Why This Is the Phase Change

- The market is moving fast toward multi-agent systems (LangGraph, CrewAI, AutoGen,
  OpenAI Swarm). **Nobody has solved the memory layer for them.** They all punt on
  shared state.
- It creates real lock-in: once teams model their agent topology against Mnemo's
  memory scoping, switching costs are structural, not just integration friction.
- It gives the GNN and graph layers (`mnemo-gnn`, `mnemo-graph`) a concrete, valuable
  purpose rather than being architectural scaffolding.
- Existing primitives (agents, users, sessions, RBAC, Qdrant namespace prefixes)
  form a foundation — this isn't a ground-up rewrite.

#### Risks

- Hard to design the right abstractions before the multi-agent ecosystem settles.
  The topology patterns (hierarchical, peer, mesh) are still emerging.
- Waiting for the ecosystem to settle means someone else defines the memory layer.
  First-mover advantage matters here.

---

### 2. Temporal Reasoning as a First-Class Primitive

**From "we store timestamps" to "we reason over time."**

Mnemo already has timestamps on episodes and temporal eval benchmarks. But the gap
between storing temporal metadata and performing genuine temporal reasoning is enormous.

#### What This Means Concretely

- **Automatic belief-change detection.** "User preferred X, but as of March they
  prefer Y." Not just versioning — understanding that a newer statement supersedes
  an older one for the same topic.
- **Temporal conflict resolution.** Not just "most recent wins" but understanding
  that some facts are time-scoped (preferences, project status) and others aren't
  (birthdate, company name). Different resolution strategies for different fact types.
- **Causal chain extraction.** "User complained about latency -> we switched
  providers -> user reported improvement." Connecting episodes into causal narratives
  rather than treating them as independent documents.
- **Time-windowed retrieval.** "What did this user care about last quarter?" as a
  first-class query, not a filter bolted onto vector search.
- **Decay and reinforcement.** Memories that are repeatedly referenced gain salience;
  memories that are never accessed gracefully fade in retrieval ranking (not deleted,
  just deprioritized).

#### Why This Matters

- Temporal reasoning directly improves retrieval quality — measurable in benchmarks
  and immediately visible to developers.
- The GNN crate and graph layer become genuinely differentiating rather than
  architectural overhead. Causal chains and belief evolution are graph problems.
- Competitors mostly treat time as a filter dimension. Making it a reasoning
  dimension is a real capability gap.

#### Risks

- Hardest of the three to get right. Temporal reasoning touches extraction, storage,
  retrieval, and context assembly — it's a cross-cutting concern.
- Hardest to evaluate. Temporal benchmarks are underdeveloped across the industry.
  You'd need to build the eval framework alongside the capability.
- Risk of over-engineering: most users may be satisfied with "most recent wins" for
  a long time.

---

### 3. Memory-Aware Context Assembly

**From "here are your search results" to "here is a structured memory brief."**

The retrieval layer currently does vector search + graph traversal + reranking.
That's table stakes. The step-change would be an **opinionated context compiler**.

#### What This Means Concretely

- **Token-budget-aware assembly.** Given a model's context window and the user's
  query, Mnemo decides what to include verbatim, what to summarize, and what to omit.
  The developer specifies a budget; Mnemo optimizes within it.
- **Query-type-adaptive retrieval.** Factual lookup ("what's the user's email?")
  uses different retrieval strategy than relationship questions ("how does user A
  relate to project B?") or temporal questions ("what changed since last week?").
  Mnemo classifies the query and selects the strategy.
- **Structured memory briefs.** Instead of returning a flat list of retrieved chunks,
  return a structured document: key facts, recent changes, relevant relationships,
  open questions. Consumable by an LLM as a coherent briefing, not a search dump.
- **Retrieval explanations.** "I included this memory because it's the most recent
  belief about topic X" or "I included this because it connects entity A to entity B
  via relation R." Debuggability for developers, transparency for audits.

#### Why This Matters

- This is what developers interact with daily. The quality of context assembly
  directly determines whether Mnemo makes their agents smarter or just adds latency.
- Immediately shippable in increments. Token budgeting alone is valuable. Structured
  briefs alone are valuable. Each piece stands on its own.
- Natural upsell surface: basic assembly is free/open, advanced assembly
  (multi-strategy, explanations) is a premium feature.

#### Risks

- Least defensible of the three. Context assembly is an integration layer that
  competitors can replicate once they see the pattern.
- Risk of becoming an "LLM wrapper" — if the assembly layer is just prompting an LLM
  to summarize, there's no technical moat. The value has to come from the retrieval
  intelligence, not the summarization.

---

## Recommendation

**Lead with Option 1 (Multi-Agent Memory Topology).** It's category-defining, plays
to Mnemo's existing strengths (agents, RBAC, namespaces), and targets a problem nobody
has solved. The multi-agent ecosystem is growing fast and the memory layer is the
missing piece.

**Pursue Option 3 (Context Assembly) in parallel at smaller scale.** It's immediately
useful, incrementally shippable, and improves the developer experience on every API
call. It doesn't require the same design commitment as the topology work.

**Invest in Option 2 (Temporal Reasoning) selectively.** Belief-change detection and
time-windowed retrieval are high-value and can ship independently. Full causal-chain
extraction is a research project — worth exploring in `mnemo-gnn` but not worth
blocking the roadmap on.

### Sequencing

```
Phase 1 (near-term):  Multi-agent memory scoping + basic context assembly
Phase 2 (mid-term):   Memory delegation/inheritance + token-budget assembly
Phase 3 (longer-term): Temporal reasoning + causal chains + structured briefs
```

The key insight: **Phase 1 changes what Mnemo is** (from a memory store to a
multi-agent memory control plane). Phases 2 and 3 make it better at being that thing.

---

## Open Questions

- What agent topologies are users/prospects actually building? Hierarchical
  (supervisor/worker), peer-to-peer, pipeline, or something else? The scoping model
  should match real usage, not theoretical elegance.
- Is there pull from enterprise prospects for multi-agent memory specifically, or is
  the demand still single-agent with better retrieval? Market signal should override
  architectural preference.
- How does the MCP (Model Context Protocol) integration play into multi-agent
  topology? MCP is becoming a standard for tool/context wiring — Mnemo's MCP server
  (`mnemo-mcp`) could be the natural surface for agent-to-agent memory sharing.
