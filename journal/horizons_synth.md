# Synthesis: Mnemo Horizon Possibilities

## The Strategic Reframe

Mnemo's phase change is not a feature addition. It is an identity shift:

**From:** AI memory database (store and retrieve for agents)
**To:** AI memory protocol (the coordination layer agents connect through)

This is not a rewrite. The existing codebase — RBAC, agent scoping, namespaces, MCP
server, graph layer, GNN scaffolding — already contains the platform primitives. The
work is to reframe, reconnect, and extend them under a coherent protocol identity.

---

## Architecture: Three Surfaces, One Protocol

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

**Agent Surface (MCP):** The primary interface for AI agents. Agents don't call REST
endpoints — they connect via MCP and interact through protocol verbs: `publish`
(observations), `subscribe` (knowledge domains), `query` (retrieval), `delegate`
(share memory scope with another agent). This is the one-line integration point.

**Management Surface (REST):** The existing 142 endpoints. Used by developers for
setup, operators for monitoring, compliance teams for audit. No reduction in surface
area — it just moves from "primary API" to "management API."

**Intelligence Layer (GNN):** The protocol's brain. Operates on the live memory graph
to detect contradictions between agents, cluster related observations, predict what
an agent will need based on conversation trajectory, and resolve conflicts when
agents disagree. This is where `mnemo-gnn` and `mnemo-graph` find their purpose.

**Storage Plane:** Redis and Qdrant remain the data backends. Potentially extensible
to PostgreSQL, S3, or other backends. This is commodity infrastructure — the value is
above it.

---

## Key Decisions

### 1. MCP becomes the primary agent-facing interface
**Because:** Distribution beats capability. MCP is becoming the wiring standard. Being
the canonical MCP memory provider places Mnemo where agents already connect. This
also structurally defends against provider absorption — model providers will adopt
protocols, not build them.

### 2. Protocol verbs replace REST-first thinking
**Because:** Agents don't think in CRUD. They think in "I learned something," "what
do I know about X?", "let agent B see my memory of Y." Protocol verbs (`publish`,
`subscribe`, `query`, `delegate`, `revoke`, `resolve`) map to agent cognition, not
database operations. The REST API persists as the management plane.

### 3. Memory topology is the reason-to-exist
**Because:** Nobody else solves multi-agent (or multi-scope) memory coordination.
Topology — who sees what, how conflicts resolve, how context flows between scopes —
is a coordination problem, not a storage problem. Mnemo already has the primitives
(agents, RBAC, namespaces). The work is wiring them into a coherent topology model.

### 4. GNN becomes protocol intelligence, not retrieval improvement
**Because:** Competing on retrieval quality is a parameter-tuning race tied to the
embedding treadmill. Competing on structural intelligence (contradiction detection,
predictive retrieval, conflict resolution over the memory graph) is an architectural
advantage that scales with the graph, not with the embedding model.

### 5. Open-core: protocol is free, coordination is paid
**Because:** Developer adoption requires a free, frictionless entry point (MCP
integration, basic memory operations). Enterprise value is in the coordination layer
(RBAC, encryption, audit, topology management, compliance). This aligns incentives:
grow the ecosystem with the free protocol, monetize the enterprise coordination layer.

---

## Horizon Ideas Beyond STEP_CHANGES.md

The LMM process surfaced several possibilities not in the original analysis:

### A. Memory-as-Protocol (Node 3)
Agents publish observations and subscribe to knowledge domains. Mnemo brokers
conflict resolution, deduplication, and temporal ordering. Pub/sub for agent
knowledge. This creates network effects (more agents = more valuable graph) rather
than just switching costs.

### B. Evaluation as Category Definition (Node 5)
Publish the canonical benchmark suite for AI memory systems. Define how the category
is measured. If the benchmarks are genuinely useful (not just favorable to Mnemo),
this is simultaneous category creation and competitive positioning. Consider:
open-sourcing the eval framework as a standalone tool.

### C. Streaming/Real-Time Memory Protocol (Node 9)
WebSocket/SSE channel for live memory events. Agents subscribe to real-time updates
as other agents publish observations. The live channel is the protocol in motion.
Not batch "ingest then retrieve" but continuous "publish and subscribe." Essential
for long-running agent systems (support bots, research assistants, coding agents).

### D. Procedural and Working Memory (Node 12)
Current data model handles episodic and declarative memory. Unexplored: procedural
memory (learned sequences of actions, tool-use patterns) and working memory
(transient context relevant to the current task). In a protocol model, these become
new observation types that agents publish, not new schema that Mnemo enforces.

### E. The "Supabase for AI Memory" Play (Node 6)
Wrap the protocol in an exceptional developer experience: hosted offering, one-click
MCP integration, dashboard showing what agents remember, usage analytics, playground
for testing retrieval. The protocol is the engine; the DX is the product. This is
the adoption accelerant.

### F. Memory Provenance and Audit Chains (Node 8 extension)
Every memory carries provenance: which agent contributed it, when, from what context,
and how confident. Audit chains trace how a piece of knowledge flowed through the
agent topology. This is compliance infrastructure that also enables debugging
("why does agent B believe X?" — trace the provenance chain). Unique to a protocol
model.

---

## Implementation Roadmap

### Phase 1: Protocol Foundation (v0.8.0)
- Extend `mnemo-mcp` with protocol verbs: `publish`, `subscribe`, `query`
- Implement memory scoping: agent-level visibility rules on namespaces
- Add `delegate` verb: agent A grants agent B read access to a memory scope
- Publish MCP memory provider spec (open, so others can implement)
- Developer docs reframed around protocol identity

### Phase 2: Topology and Intelligence (v0.9.0)
- Memory delegation with inheritance (hierarchical scoping)
- Cross-agent conflict resolution (contradiction detection via `mnemo-gnn`)
- Streaming channel (WebSocket/SSE) for real-time memory events
- `revoke` and `resolve` protocol verbs
- Benchmark suite published as standalone eval framework

### Phase 3: Platform and DX (v1.0.0)
- Hosted offering (the "Supabase play")
- Dashboard: memory topology visualization, provenance explorer, usage analytics
- Procedural and working memory as observation types
- Full provenance and audit chain support
- Enterprise tier: compliance controls, data residency, SLA

---

## Success Criteria

- [ ] At least one external project integrates via MCP memory protocol (not REST)
- [ ] Protocol verbs (`publish`, `subscribe`, `query`, `delegate`) are implemented
      and documented
- [ ] Memory topology scoping works: agent A's memories are invisible to agent C
      but visible to agent B via explicit delegation
- [ ] GNN detects at least one class of cross-agent contradiction in production
- [ ] Benchmark suite is published independently and used by at least one competitor
      for evaluation
- [ ] Developer on-ramp from "zero to memory-enabled agent" is under 5 minutes
- [ ] Open-core boundary is defined: free tier vs. enterprise tier

---

## Explicit Handling of Major Tensions

**Provider absorption:** Protocol framing is the defense. Providers adopt protocols;
they don't build them. If MCP wins and Mnemo is the canonical memory provider in that
ecosystem, absorption becomes integration instead.

**Surface area vs. PMF:** The surface area doesn't shrink — it gets repackaged. MCP
is the agent entry point (simple). REST is the management plane (powerful). Dashboard
is the operator view (visual). Same 142 endpoints, but developers enter through MCP,
not through the REST API docs.

**GNN justification:** Protocol intelligence gives GNN concrete, measurable purpose.
If contradiction detection doesn't outperform simple heuristics in practice, the GNN
investment is a sunk cost. Validate with a specific benchmark before scaling.

**Timing of multi-agent bet:** Even if multi-agent is early, the topology model works
for single agents with multiple memory scopes. The bet isn't on agent count — it's on
memory coordination complexity, which increases with any non-trivial agent system.

---

## What Surprised Me

The reflection phase revealed that the STEP_CHANGES.md analysis was playing the wrong
game. All three candidates (multi-agent topology, temporal reasoning, context assembly)
are Game A moves: "what feature should we add to the database?" The actual phase change
is Game B: "what would it mean if Mnemo were a protocol, not a database?"

The existing codebase already contains the platform primitives. The work isn't building
new things — it's reframing what the existing things mean and investing in the
connective tissue (MCP surface, topology model, protocol intelligence) that turns
infrastructure into platform.

The wood showed its grain. This is the clean cut.
