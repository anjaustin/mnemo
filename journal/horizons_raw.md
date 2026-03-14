# Raw Thoughts: Mnemo Horizon Possibilities

## Stream of Consciousness

The three candidates in STEP_CHANGES.md — multi-agent topology, temporal reasoning,
context assembly — are real, but they came from inside-out thinking. What's on the
architecture diagram, what the codebase can do next. I want to flip the lens. What
forces are acting on the AI memory space from the outside? What's changing in the
ecosystem that could make entirely new categories possible — or make the current
category irrelevant?

First gut-level observation: the memory layer for AI agents is still pre-paradigm.
Nobody has won. Zep pivoted from open-source to hosted. Mem0 is thin. Letta is
research-flavored. LangMem is a LangChain extension. The market is fragmented because
nobody has proven what "memory" even means for production AI systems. It could mean
conversation history (boring, commoditized), it could mean knowledge graphs (academic,
hard to maintain), or it could mean something nobody has named yet.

What scares me: that "memory" gets absorbed into the model providers. OpenAI already
has memory in ChatGPT. Anthropic could add persistent state. Google has context
caching. If the foundation model providers decide memory is a feature, not an
infrastructure layer, Mnemo's category could shrink. The counter-argument is that
enterprise customers won't trust model providers with their data topology, and
multi-model architectures need a provider-agnostic memory layer. But that's a bet on
enterprise paranoia and multi-model futures — both of which I believe in but can't
guarantee.

What excites me: MCP (Model Context Protocol) is becoming the wiring standard for
tool and context integration. Mnemo already has an MCP server crate. If MCP wins as
the standard, Mnemo could be the canonical memory provider in that ecosystem. That's
a distribution channel, not a feature. Being the default MCP memory server is
potentially more valuable than any individual capability.

Another thing I keep coming back to: the GNN crate exists but it's mostly scaffolding.
Graph neural networks on memory structures is genuinely novel — nobody else is doing
inference over the memory graph itself. The question is whether that's novel because
it's ahead of its time, or novel because it's unnecessary. Both are possible.

What about the embedding space itself? Mnemo currently uses AllMiniLML6V2 (384-dim)
locally with ONNX Runtime. That's fine for v0.7.0 but the embedding landscape is
moving fast. Matryoshka embeddings, late interaction (ColBERT-style), multi-vector
representations. If Mnemo's retrieval quality depends on embedding quality, and
embedding quality is a moving target, then Mnemo needs to be embedding-agnostic at
the architecture level (which it mostly is — `mnemo-llm` abstracts providers) but
also needs to leverage advances rather than just tolerate them.

Operational concern: Mnemo has 142 endpoints and 10 crates. That's a lot of surface
area for a pre-product-market-fit project. Are we building too much infrastructure
before we know what people want? The red team hardening was valuable — it proved
the system is solid. But "solid" and "needed" are different things.

Privacy and compliance angle: GDPR right-to-be-forgotten, data residency, audit
trails. Mnemo already has encryption at rest, RBAC, audit signing. If the play is
enterprise AI memory, these aren't features — they're table stakes. But most
competitors don't have them. That's an advantage if it matters to buyers.

Weird thought: what if memory isn't a database? What if the right abstraction is
memory as a protocol? Not "store and retrieve" but "here's what I know, here's what
I learned, here's what changed." A pub/sub model for agent knowledge, where Mnemo is
the broker, not the store. Agents publish observations, subscribe to knowledge
domains, and Mnemo handles conflict resolution, deduplication, and temporal ordering.
That's a fundamentally different architecture from what exists today.

Another angle: evaluation. The LongMemEval and temporal benchmarks are good, but the
industry doesn't have agreed-upon benchmarks for memory systems. Could Mnemo define
the evaluation standard? If Mnemo publishes the canonical benchmark suite and scores
well on it, that's simultaneous category definition and competitive advantage. The
risk is that it looks self-serving. The opportunity is that nobody else is doing it.

What about the developer experience? Right now integrating Mnemo means: start the
server, hit REST endpoints, manage sessions/users/agents yourself. Compare that to
something like Supabase which wraps PostgreSQL in an incredibly ergonomic developer
experience with auth, real-time, storage, edge functions all bundled. What would
"Supabase for AI memory" look like? SDK-first, hosted option, dashboard that shows
you what your agents remember, one-line integration.

Pricing and business model: is Mnemo open-core (open source + paid features)? Fully
open source with hosted offering? The architecture decisions should support the
business model, not the other way around. If the play is open-core, which features
are free and which are paid? RBAC, encryption, multi-agent topology are natural
paid-tier features. But gating security features feels wrong.

I keep thinking about the "memory as control plane" positioning. A control plane
implies there are data planes underneath. The data planes could be different storage
backends (Redis, Qdrant, PostgreSQL, S3). The control plane manages the topology,
access control, conflict resolution, and retrieval intelligence. That abstraction
could be very powerful — it separates "where data lives" from "how agents interact
with memory." But it's also a big architectural commitment.

One more thing: the current system is synchronous. Ingest an episode, extract
entities, embed, store. What about streaming? Real-time memory updates from agent
conversations as they happen. WebSocket or SSE connections where the memory layer
is continuously learning, not batch-processing. That changes the architecture
significantly but could be a killer feature for long-running agent systems.

Questions arising:
- Is the market for AI memory infrastructure or AI memory product?
- Should Mnemo bet on MCP as a distribution channel?
- Is the GNN work ahead of its time or unnecessary?
- What would "memory as a protocol" look like concretely?
- Should evaluation/benchmarks be a strategic pillar or just marketing?
- How much does developer experience matter vs. raw capability?
- Where does Mnemo sit on the open-core spectrum?
- Does streaming/real-time memory change the category?
- What happens when model providers add native memory?

First instincts:
- MCP is probably the highest-leverage distribution bet right now.
- Multi-agent topology is the right capability bet.
- Developer experience is underinvested but could be the difference between
  "technically superior" and "actually adopted."
- The GNN work should be validated by a concrete use case before more investment.
- Streaming memory is fascinating but premature without product-market fit signal.
