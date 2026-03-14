# Spec 02: Memory Scoping & Multi-Agent Topology

> Target: v0.8.0 (scoping foundations) through v0.9.0 (topology intelligence)
> Priority: The differentiator. This is the reason to choose Mnemo over alternatives.

---

## Problem

Every AI memory system — including Mnemo today — treats memory as single-tenant
per user. Agent A and Agent B serving the same user see the same memory. There is no
way to:

- Restrict what memory an agent can see based on its role
- Share a subset of one user's memory with another agent without sharing everything
- Have a supervisor agent's context flow to worker agents but not vice versa
- Detect when two agents form contradictory beliefs about the same entity
- Track which agent contributed which memory (provenance)

The primitives exist in the codebase (MemoryRegions, ACLs, RBAC, agent scoping) but
they are not wired into the retrieval pipeline. Regions are metadata-only constructs
right now.

## What Exists Today

### Complete Primitives (ready to build on)

| Primitive | Status | Location |
|---|---|---|
| `MemoryRegion` model (name, owner_agent_id, user_id, entity/edge filters, classification ceiling) | Implemented | `region.rs` |
| `MemoryRegionAcl` model (agent_id, permission: Read/Write/Manage, expiry) | Implemented | `region.rs` |
| `RegionStore` trait (CRUD + ACL grant/revoke/list) | Implemented | `storage.rs`, `redis_store.rs` |
| Region REST API (6 endpoints: CRUD + ACL grant/revoke/list) | Implemented | `routes.rs` |
| `ApiKeyScope` with `allowed_agent_ids` | Implemented | `api_key.rs` |
| `CallerContext.require_agent_access()` | Implemented | `api_key.rs` |
| Agent identity profiles (versioned, opaque core) | Implemented | `agent.rs` |
| `AgentIdentityAuditEvent` with SHA-256 hash chain | Implemented | `agent.rs` |
| Data classification (Public/Internal/Confidential/Restricted) | Implemented | `classification.rs` |
| `MemoryView` (named policy lenses with classification + type + temporal scoping) | Implemented | `view.rs` |
| Qdrant user-level tenant isolation (`user_id` filter on all searches) | Implemented | `qdrant_store.rs` |
| Guardrail rules (composable conditions + actions) | Implemented | `guardrail.rs` |

### Gaps (not yet built)

1. **Region-scoped retrieval.** Region filters (entity types, edge labels,
   classification ceiling) are not applied during context retrieval. The retrieval
   pipeline ignores regions entirely.
2. **No `agent_id` on Session or Episode.** Cannot track which agent created which
   memory.
3. **No per-agent Qdrant filtering.** Vector search filters by `user_id` only, not
   `agent_id`.
4. **No agent-scoped guardrails.** GuardrailScope is Global or User, not Agent.
5. **No agent registration/listing API.** Agents are implicitly created.
6. **Views are global, not agent-scoped.** No `agent_id` on MemoryView.
7. **No inter-agent event notification.** Webhook system is user-facing.

## Deliverables

### D1: Agent-ID on Sessions and Episodes

**Add `agent_id: Option<String>` to Session and Episode models.**

When an API call includes an `X-Agent-Id` header (or the CallerContext has an
`agent_id` from a scoped API key), the agent_id is stamped on the created
session/episode. This enables:
- Filtering episodes by agent ("what did agent A write?")
- Provenance tracking ("this fact came from agent B's conversation")
- Agent-scoped retrieval ("only retrieve episodes agent A created")

**Implementation:**
- Add field to `Session` struct in `session.rs`, `Episode` struct in `episode.rs`
- Propagate through `CreateSessionRequest`, `CreateEpisodeRequest`
- Store in Redis alongside existing fields
- Add `agent_id` to Qdrant point payloads for all three collections
- Add optional `?agent_id=` filter to list endpoints

**Migration:** Existing sessions/episodes will have `agent_id: None`, meaning
"unknown agent" (backward compatible).

### D2: Region-Scoped Retrieval

**Wire MemoryRegion filters into the context retrieval pipeline.**

When an agent calls `/api/v1/memory/{user}/context` (or the `recall` MCP tool),
the retrieval pipeline must:

1. Look up which regions the calling agent has access to (via `list_agent_accessible_regions`)
2. Compute the effective filter: union of all accessible region entity/edge filters
3. Apply the classification ceiling: minimum of caller's ceiling and region's ceiling
4. Filter Qdrant search results through the region constraints
5. Filter graph traversal results through the region constraints
6. Assemble context only from memory that passes all filters

**If the agent has no region access for this user:** Return empty context with an
explanatory message, not the user's full memory. This is the key behavioral change —
regions become mandatory gates for multi-agent access, not optional overlays.

**If no regions exist for this user:** Fall back to current behavior (full memory
access for the calling agent). This preserves backward compatibility for
single-agent setups.

**Implementation:**
- Add `resolve_agent_regions()` function to retrieval pipeline in `mnemo-retrieval`
- Add `RegionConstraints` struct that flattens region filters + classification ceiling
- Modify `hybrid_search()` to accept optional `RegionConstraints`
- Add Qdrant filter conditions for entity_type and edge_label when constraints present
- Add post-filter on graph traversal results
- Thread `agent_id` from CallerContext through the retrieval call chain

### D3: Agent Registration and Discovery

**Add explicit agent lifecycle API.**

Currently agents are implicitly created when their identity is first accessed.
This means there's no way to list all agents, no way to know which agents exist
before granting them region access.

**New endpoints:**
- `GET /api/v1/agents` — list all registered agents (paginated)
- `POST /api/v1/agents` — explicitly register an agent (name, description, owner)
- `GET /api/v1/agents/:agent_id` — agent metadata + identity summary
- `DELETE /api/v1/agents/:agent_id` — deregister (soft delete, preserves audit)

**Auto-registration:** When an agent identity is accessed for the first time (via
existing `GET /api/v1/agents/:agent_id/identity`), auto-register if not already
registered. This preserves backward compatibility.

**Implementation:**
- Add `AgentRegistration` model in `agent.rs`: `agent_id`, `name`, `description`,
  `owner` (who registered it), `created_at`, `status` (active/deregistered)
- Add `AgentRegistrationStore` trait
- Add Redis implementation
- Add route handlers

### D4: Memory Delegation via MCP

**The `delegate`, `revoke`, and `scopes` MCP tools (from Spec 01) wire into these
region primitives.**

When Agent A calls the `delegate` tool:
```json
{
  "tool": "delegate",
  "arguments": {
    "user": "jordan",
    "target_agent": "agent-b",
    "scope_name": "deal-context",
    "entity_types": ["organization", "person"],
    "permission": "read",
    "expires_in_hours": 24
  }
}
```

This creates a MemoryRegion (if it doesn't exist) scoped to the specified entity
types for user "jordan", then grants Agent B read access via an ACL entry with
24-hour expiry.

When Agent B calls `recall`:
```json
{
  "tool": "recall",
  "arguments": {
    "user": "jordan",
    "query": "What are the Acme renewal blockers?"
  }
}
```

The retrieval pipeline checks Agent B's accessible regions, finds the
"deal-context" region, and returns only organization/person entities matching the
query — not the user's full memory.

### D5: Agent-Scoped Guardrails

**Add `Agent { agent_id: String }` variant to `GuardrailScope`.**

This enables rules like:
- "Block agent-intern from accessing Restricted-classified data"
- "Audit all writes from agent-experimental"
- "Redact financial entities when agent-support retrieves context"

**Implementation:**
- Add variant to `GuardrailScope` enum in `guardrail.rs`
- Update guardrail evaluation to check agent_id from CallerContext
- Add test cases

### D6: Provenance on Facts

**Add `source_agent_id: Option<String>` and `source_episode_id: Option<Uuid>` to
the `Edge` (fact) model.**

When the ingest pipeline extracts entities and facts from an episode, it stamps
the source agent and episode on each fact. This enables:
- "Which agent contributed this fact?"
- "What episode is the evidence for this fact?"
- Dashboard provenance explorer (Spec 01 D3 can show this)

**Implementation:**
- Add fields to `Edge` in `edge.rs`
- Propagate from episode metadata during ingest in `mnemo-ingest`
- Include in API responses for edge/fact endpoints
- Include in dashboard fact browser

---

## Non-Goals

- **Cross-user regions.** Regions are scoped to a single user's data. Sharing
  memory across users is a different problem (multi-tenant federation) and
  out of scope.
- **Real-time region sync / push notifications.** Regions define visibility
  boundaries but don't push updates when new data matches. This is a streaming
  concern (deferred per STEP_CHANGES.md).
- **Hierarchical agent trees.** Agent delegation is flat (A grants B access).
  Hierarchical inheritance (A's grants cascade to A's children) is v1.0.0.
- **Agent-to-agent messaging.** Agents communicate through shared memory, not
  through a message bus.

## Risks

1. **Region-scoped retrieval adds latency.** Looking up regions and applying filters
   adds overhead to every retrieval call. Mitigate: cache region lookups per-agent
   with short TTL (30s). Benchmark before and after.
2. **Backward compatibility.** Existing single-agent setups must continue to work
   without configuration changes. The "no regions = full access" fallback is critical.
3. **Complexity cliff.** Regions + ACLs + Views + Guardrails + Classification creates
   a combinatorial access control surface. Must have clear precedence rules and good
   error messages ("Agent B cannot access this fact because region X has classification
   ceiling Internal but the fact is Confidential").

## Success Criteria

- [ ] `agent_id` stamped on sessions and episodes when provided
- [ ] Region filters applied during retrieval — agent with region access gets
      filtered results, agent without gets empty context
- [ ] Agent registration API: create, list, get, delete
- [ ] MCP `delegate`/`revoke`/`scopes` tools create/manage regions and ACLs
- [ ] Agent-scoped guardrails enforce per-agent rules
- [ ] Facts carry provenance (source agent + episode)
- [ ] Existing single-agent integrations work without changes (backward compat)
- [ ] Retrieval latency increase from region filtering < 10ms p95
