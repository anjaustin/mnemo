# Nodes of Interest: Mnemo Horizon Possibilities

## Node 1: The Provider Absorption Threat
Model providers (OpenAI, Anthropic, Google) are adding memory-like features natively.
ChatGPT already has memory. Context caching is growing. If memory becomes a provider
feature rather than an infrastructure layer, the standalone memory category contracts.
Why it matters: This is an existential risk to the category, not just to Mnemo. The
counter-position is that enterprise won't trust providers with memory topology, and
multi-model architectures need provider-agnostic memory. But this is a bet, not a
certainty.

## Node 2: MCP as Distribution Channel
Model Context Protocol is becoming the wiring standard for tool and context integration.
Mnemo already has `mnemo-mcp`. If MCP wins, being the canonical MCP memory provider is
a distribution advantage that compounds — it's ecosystem placement, not feature work.
Why it matters: Distribution often matters more than capability. Being the default
memory provider in the MCP ecosystem could be more valuable than any individual feature.

## Node 3: Memory-as-Protocol vs. Memory-as-Database
Current architecture: Mnemo is a memory database with a REST API. Alternative
framing: Mnemo as a memory *protocol* — agents publish observations, subscribe to
knowledge domains, Mnemo brokers conflict resolution, deduplication, temporal ordering.
Pub/sub for agent knowledge rather than CRUD for agent data.
Why it matters: This reframes what Mnemo is at the category level. A protocol creates
network effects; a database creates switching costs. Different defensibility models.
Tension with Node 1: A protocol is harder for providers to absorb than a database.

## Node 4: GNN — Ahead of Its Time or Unnecessary?
`mnemo-gnn` exists as scaffolding. Graph neural networks over memory structures is
genuinely novel — nobody else does inference over the memory graph. But novelty is
ambiguous: it could mean "ahead of the market" or "solving a problem nobody has."
Why it matters: Significant engineering investment has gone into the graph and GNN
layers. If there's no concrete use case that demonstrates measurable value over
simpler retrieval, this investment is premature.
Tension with Node 3: A protocol model could give GNN a purpose — inference over the
live knowledge graph to detect contradictions, cluster related observations, predict
what an agent will need next.

## Node 5: Evaluation as Category Definition
The AI memory space has no agreed-upon benchmarks. Mnemo's LongMemEval and temporal
eval suites are ahead of competitors. Publishing the canonical benchmark suite could
simultaneously define the evaluation standard and position Mnemo favorably.
Why it matters: Whoever defines how memory systems are measured controls the narrative.
Risk: looks self-serving if Mnemo's benchmarks only measure what Mnemo does well.
Must be genuinely useful to be credible.

## Node 6: Developer Experience Gap
Current integration: start server, hit REST endpoints, manage sessions/users/agents
manually. Compare to Supabase (wraps PostgreSQL in auth + real-time + storage +
dashboard + one-line SDK). The "Supabase for AI memory" opportunity exists.
Why it matters: Technical superiority means nothing if integration is painful.
Developer experience is the difference between "technically superior" and "actually
adopted."
Tension with Node 2: MCP could *be* the developer experience — instead of REST SDK
integration, just point your MCP-compatible agent at Mnemo's MCP endpoint.

## Node 7: Surface Area vs. Product-Market Fit
142 endpoints, 10 crates, full Helm deployment, gRPC, OpenAPI, two SDKs. That's a
lot of infrastructure for a project without confirmed product-market fit. The red team
hardening proved the system is solid. But "solid" and "needed" are different things.
Why it matters: If the market wants something different from what Mnemo has built, the
large surface area becomes maintenance burden rather than competitive advantage.
Tension with Node 6: Developer experience improvements could be the fastest path to
PMF signal — make it easy to try, then learn what people actually use.

## Node 8: Privacy/Compliance as Competitive Moat
GDPR right-to-be-forgotten, data residency, audit trails, encryption at rest, RBAC,
audit signing — Mnemo has these; most competitors don't. If the buyer is enterprise
AI teams, compliance isn't a feature — it's a gate. But if the buyer is indie
developers building agents, compliance is irrelevant overhead.
Why it matters: The compliance posture determines the customer segment. Enterprise
needs it; indie developers don't care.
Tension with Node 7: Compliance surface area is exactly the kind of thing that's
expensive to maintain without revenue.

## Node 9: Streaming/Real-Time Memory
Current system is synchronous batch: ingest episode, extract, embed, store. Alternative:
streaming memory via WebSocket/SSE — continuous learning from live agent conversations.
Real-time entity extraction, live knowledge graph updates, immediate availability.
Why it matters: Long-running agent systems (customer support, research assistants,
coding agents) need memory that updates in real-time, not batch. This is a
differentiator nobody else has.
Tension with Node 7: Streaming adds significant architectural complexity. Without
PMF signal, it could be premature investment.

## Node 10: The Embedding Treadmill
Mnemo uses AllMiniLML6V2 (384-dim) locally. Embedding quality is a moving target:
Matryoshka embeddings, ColBERT-style late interaction, multi-vector representations.
Mnemo is embedding-provider-agnostic at the config level, but retrieval quality
depends on embedding quality.
Why it matters: If a competitor ships with a dramatically better embedding strategy,
Mnemo's retrieval quality gap could appear overnight. The abstraction layer helps but
doesn't eliminate the risk.
Dependency on Node 4: GNN inference over the memory graph could provide retrieval
quality that's independent of embedding quality — a structural advantage rather than
a parameter-tuning race.

## Node 11: Multi-Agent Topology (from STEP_CHANGES.md)
Agent-scoped namespaces, memory delegation, cross-agent conflict resolution. Already
identified as the highest-leverage capability bet.
Why it matters: Restated here because it intersects with nearly every other node.
Multi-agent topology + MCP distribution (Node 2) + protocol framing (Node 3) could
be a coherent thesis. Multi-agent topology alone is a feature; multi-agent topology
as a protocol over MCP is a platform.

## Node 12: The "What Is Memory?" Question
The market hasn't converged on what "AI memory" means. Conversation history?
Knowledge graphs? User preferences? Learned behaviors? World models? Mnemo currently
handles episodes, entities, relations, and facts. But if "memory" turns out to mean
something different — say, learned procedural knowledge or behavioral patterns — the
current data model may be wrong.
Why it matters: Building the best implementation of the wrong abstraction is the most
expensive kind of mistake. The data model assumptions need to be challenged.

---

## Tension Summary

| Tension | Nodes | Nature |
|---------|-------|--------|
| Provider absorption vs. infrastructure independence | 1 vs. 2, 3 | Existential |
| Protocol vs. database identity | 3 vs. current architecture | Identity |
| GNN investment vs. concrete use case | 4 vs. 7 | Resource allocation |
| Surface area vs. PMF discovery | 7 vs. 6, 9 | Strategic focus |
| Enterprise compliance vs. developer adoption | 8 vs. 6 | Customer segment |
| Streaming ambition vs. premature complexity | 9 vs. 7 | Timing |
| Feature bet vs. distribution bet | 11 vs. 2 | Strategy |
| Data model assumptions vs. market definition | 12 vs. all | Foundational |
