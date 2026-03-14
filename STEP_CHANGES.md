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

---

## LMM Horizon Exploration (Lincoln Manifold Method Pass)

> Full journal: `journal/horizons_raw.md`, `journal/horizons_nodes.md`,
> `journal/horizons_reflect.md`, `journal/horizons_synth.md`

The three candidates above came from inside-out thinking — what the codebase can do
next. An LMM pass flipped the lens to ask what external forces are shaping the
possibility space. The method surfaced a deeper structural question that reframes
the entire analysis.

### The Two-Game Insight

The candidates above are all **Game A (Infrastructure)** moves: "what feature should
we add to the database?" The LMM process revealed a second game:

- **Game A — Infrastructure:** Be the best memory database for AI agents. Compete on
  retrieval quality, storage efficiency, compliance, and raw capability. Win by being
  technically superior.
- **Game B — Platform:** Be the memory *layer* that agents connect through. Compete on
  ecosystem placement, developer experience, and network effects. Win by being where
  agents already are.

**Mnemo has been playing Game A while building Game B primitives by accident.** The
RBAC, agent scoping, MCP server, namespace prefixes, graph layer — these are platform
primitives wired up as infrastructure features. The phase change isn't adding a
capability. It's reframing what the existing capabilities mean.

### The Protocol Thesis

**Reframe Mnemo from a memory database to a memory protocol.** Agents don't call REST
endpoints — they connect via MCP and interact through protocol verbs: `publish`
(observations), `subscribe` (knowledge domains), `query` (retrieval), `delegate`
(share memory scope with another agent).

Three surfaces, one protocol:

```
┌──────────────────────────────────────────────────────┐
│                    MNEMO PROTOCOL                     │
├──────────────────────────────────────────────────────┤
│                                                       │
│  Agent Surface (MCP)        ← agents connect here    │
│    publish / subscribe / query / delegate             │
│                                                       │
│  Management Surface (REST)  ← operators/devs here    │
│    CRUD / RBAC / audit / compliance / dashboard       │
│                                                       │
│  Intelligence Layer (GNN)   ← protocol brain          │
│    contradiction detection / clustering /              │
│    predictive retrieval / conflict resolution          │
│                                                       │
├──────────────────────────────────────────────────────┤
│  Storage Plane (Redis + Qdrant)  ← commodity          │
│    episodes / entities / relations / embeddings       │
└──────────────────────────────────────────────────────┘
```

### Why Protocol > Database

- **Provider absorption defense.** If Mnemo is a database, OpenAI/Anthropic/Google
  can build one. If Mnemo is a protocol, providers become *clients* of it. Providers
  adopt protocols; they don't build them.
- **MCP as distribution.** Being the canonical MCP memory provider places Mnemo where
  agents already connect. Distribution beats capability.
- **Network effects.** A protocol creates network effects (more agents = more valuable
  graph). A database creates switching costs. Different defensibility models.
- **GNN finds its purpose.** Protocol intelligence — contradiction detection,
  clustering, predictive retrieval over the live memory graph — is a graph inference
  problem. The `mnemo-gnn` investment stops being speculative scaffolding.
- **Embedding treadmill escape.** If retrieval quality comes from graph intelligence
  over topology (structural) rather than vector similarity (parametric), Mnemo's
  advantage is architectural, not tied to which embedding model is fastest this month.

### Additional Horizon Ideas (from LMM exploration)

Beyond the three candidates above, the LMM pass surfaced six additional possibilities:

#### 4. Memory-as-Protocol (Pub/Sub for Agent Knowledge)

Agents publish observations and subscribe to knowledge domains. Mnemo brokers conflict
resolution, deduplication, and temporal ordering. Not CRUD for agent data — pub/sub
for agent knowledge. This is the identity-level reframe described above.

#### 5. Evaluation as Category Definition

The AI memory space has no agreed-upon benchmarks. Mnemo's LongMemEval and temporal
eval suites are ahead of competitors. Publishing the canonical benchmark suite as a
standalone, open framework could simultaneously define the evaluation standard and
position Mnemo favorably. Risk: looks self-serving if benchmarks only measure what
Mnemo does well. Must be genuinely useful to be credible.

#### 6. Streaming/Real-Time Memory

WebSocket/SSE channel for live memory events. Agents subscribe to real-time updates
as other agents publish observations. Not batch "ingest then retrieve" but continuous
"publish and subscribe." Essential for long-running agent systems (support bots,
research assistants, coding agents). In the protocol model, this is the protocol in
motion — the live channel through which agents coordinate.

#### 7. Procedural and Working Memory

Current data model handles episodic and declarative memory. Unexplored: procedural
memory (learned sequences of actions, tool-use patterns) and working memory (transient
context relevant to the current task). In a protocol model, these become new
observation types that agents publish, not new schema that Mnemo enforces. The data
model becomes extensible by convention.

#### 8. The "Supabase for AI Memory" DX Play

Wrap the protocol in exceptional developer experience: hosted offering, one-click MCP
integration, dashboard showing what agents remember, usage analytics, playground for
testing retrieval. The protocol is the engine; the DX is the product. This is the
adoption accelerant that turns technical capability into actual usage.

#### 9. Memory Provenance and Audit Chains

Every memory carries provenance: which agent contributed it, when, from what context,
how confident. Audit chains trace how knowledge flowed through the agent topology.
This is compliance infrastructure that also enables debugging ("why does agent B
believe X?" — trace the provenance chain). Unique to a protocol model where multiple
agents contribute to a shared knowledge graph.

### Revised Recommendation

The original recommendation (lead with multi-agent topology) still holds, but the
LMM pass elevates it: **topology is not a feature to add — it's the reason the
protocol exists.** The revised sequencing:

```
Phase 1 (v0.8.0):  Protocol foundation
  - MCP protocol verbs (publish, subscribe, query, delegate)
  - Memory scoping (agent-level visibility rules)
  - Developer docs reframed around protocol identity

Phase 2 (v0.9.0):  Topology and intelligence
  - Delegation with inheritance (hierarchical scoping)
  - Cross-agent conflict resolution (contradiction detection via mnemo-gnn)
  - Streaming channel (WebSocket/SSE) for real-time memory events
  - Benchmark suite published as standalone eval framework

Phase 3 (v1.0.0):  Platform and DX
  - Hosted offering (the "Supabase play")
  - Provenance explorer and topology visualization in dashboard
  - Procedural and working memory as observation types
  - Enterprise tier: compliance controls, data residency, SLA
```

### Open-Core Model

The protocol framing clarifies the business model:
- **Free:** Protocol (MCP integration, basic memory operations, pub/sub)
- **Paid:** Coordination (RBAC, encryption, audit trails, topology management,
  compliance, data residency)

Developer adoption through the free protocol; enterprise monetization through the
coordination layer.

### Tensions Resolved by the Protocol Framing

| Tension | Resolution |
|---------|------------|
| Provider absorption vs. independence | Providers adopt protocols, they don't build them |
| GNN investment vs. concrete use case | GNN = protocol intelligence (contradiction detection, clustering) |
| 142 endpoints vs. PMF discovery | REST becomes management plane; MCP becomes the simple entry point |
| Enterprise compliance vs. developer adoption | Free protocol for adoption; paid compliance for enterprise |
| Streaming ambition vs. premature complexity | Streaming is the protocol in motion, not a bolted-on feature |
| Data model assumptions vs. market definition | Protocol model makes the data model extensible by convention |
