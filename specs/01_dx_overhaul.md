# Spec 01: Developer Experience Overhaul

> Target: v0.8.0
> Priority: Do first. Nothing else matters if the on-ramp is painful.

---

## Problem

Mnemo has 142 REST endpoints, 7 MCP tools, 42-43 SDK methods, and a 7-page
dashboard. The infrastructure is mature. But the path from "I heard about Mnemo"
to "my agent has memory" requires understanding REST endpoints, managing
users/sessions manually, configuring embedding providers, and reading through
dense API documentation. The on-ramp is 15-30 minutes for an experienced developer.
It should be under 5.

## What Exists Today

### MCP Server (`mnemo-mcp`)
- 7 tools: `mnemo_remember`, `mnemo_recall`, `mnemo_graph_query`,
  `mnemo_agent_identity`, `mnemo_digest`, `mnemo_coherence`, `mnemo_health`
- 2 resource templates: `mnemo://users/{user}/memory`,
  `mnemo://agents/{agent_id}/identity`
- Stdio transport only (no SSE/HTTP)
- Hand-rolled JSON-RPC 2.0 (no SDK dependency)
- 62 tests (including adversarial/falsification)
- Pure HTTP client adapter — calls the Mnemo REST API, no internal crate imports
- Config: `MNEMO_MCP_BASE_URL`, `MNEMO_API_KEY`, `MNEMO_MCP_DEFAULT_USER`

### SDKs
- **Python**: 2-line init, 42 methods, zero runtime deps (stdlib urllib), async
  parity, LangChain + LlamaIndex adapters, rich error hierarchy, retries w/ jitter
- **TypeScript**: 2-line init, 43 methods, zero runtime deps (native fetch),
  LangChain.js + Vercel AI SDK adapters, thinner error hierarchy, no 429 retry
- Both: comprehensive API coverage, request-ID correlation, typed results

### Dashboard
- 7 pages: Home, Webhooks, Time Travel (RCA), Governance, Traces, Explorer, LLM Spans
- D3 v7 visualizations: evidence constellation (SVG), knowledge graph (Canvas),
  RCA timeline (SVG)
- Embedded SPA (rust_embed), no build step, served at `/_/`
- Deep-linking, auto-refresh, incident drilldowns

### DX Gaps (from audit)
- Python async client duplicates ~1400 lines of sync client
- Sub-structures (entities/facts/episodes in ContextResult) are untyped dicts
- TypeScript: no 429 retry, no `context_head()`, `graphEntity()` returns untyped
  record, no test suite in repo, casing inconsistency (snake_case results, camelCase
  options)
- Python: no `delete_user()`, `delete_session()`, `delete_entity()`, `delete_edge()`
- No pagination helpers in either SDK
- No streaming support in either SDK
- Dashboard has no memory content explorer (can browse entities/graph but can't see
  actual episode content, fact text, or search memory)
- MCP doc comment says port 3000 but code defaults to 8080

## Deliverables

### D1: Zero-to-Memory Quickstart (documentation + tooling)

**Goal:** Developer goes from nothing to memory-enabled agent in under 5 minutes.

**Concrete steps the developer will take:**

```bash
# 1. Start Mnemo (30 seconds)
curl -fsSL https://raw.githubusercontent.com/anjaustin/mnemo/main/deploy/docker/quickstart.sh | bash
# This runs: docker compose -f deploy/docker/docker-compose.quickstart.yml up -d
# Starts: mnemo-server (pre-built image), redis, qdrant
# Prints: "Mnemo is running at http://localhost:8080"

# 2. Connect an MCP-compatible agent (30 seconds)
# Add to claude_desktop_config.json or .cursor/mcp.json:
# { "mcpServers": { "mnemo": { "command": "docker", "args": ["exec", "-i", "mnemo-server", "mnemo-mcp-server"] } } }

# 3. Use memory (immediately)
# Agent can now call mnemo_remember and mnemo_recall tools
```

**What this requires:**
- `deploy/docker/docker-compose.quickstart.yml` — minimal compose file that starts
  mnemo-server (from GHCR image), redis, qdrant with zero config. No API keys, no
  embedding config, no env files. Uses local embeddings (AllMiniLML6V2) by default.
- `deploy/docker/quickstart.sh` — curl-pipe-bash script that downloads the compose
  file and runs `docker compose up -d`. Prints connection instructions.
- `mnemo-mcp-server` binary included in the Docker image (already built as part of
  the workspace, needs to be added to the Dockerfile).
- `QUICKSTART.md` at repo root — the entire flow above, with screenshots.
- Update `README.md` "Start here" section to lead with this flow.

**Success criterion:** A developer who has Docker installed can go from zero to
a working MCP memory integration in under 5 minutes, timed.

### D2: MCP Tool Vocabulary Refinement

**Current tools and proposed changes:**

| Current Tool | Action | Proposed |
|---|---|---|
| `mnemo_remember` | Keep | Rename to `remember` (shorter, agent-natural) |
| `mnemo_recall` | Keep | Rename to `recall` |
| `mnemo_graph_query` | Keep | Rename to `graph` |
| `mnemo_agent_identity` | Keep | Rename to `identity` |
| `mnemo_digest` | Keep | Rename to `digest` |
| `mnemo_coherence` | Keep | Rename to `coherence` |
| `mnemo_health` | Keep | Rename to `health` |
| (new) | Add | `delegate` — grant another agent read access to a memory scope |
| (new) | Add | `revoke` — revoke delegated access |
| (new) | Add | `scopes` — list memory scopes visible to this agent |

**Rationale:** The `mnemo_` prefix is redundant when the MCP server is already named
"mnemo". Shorter tool names are easier for agents to reason about. The three new tools
(`delegate`, `revoke`, `scopes`) expose the existing MemoryRegion + ACL primitives
through MCP, enabling multi-agent topology without REST API interaction.

**Implementation:**
- Rename tools in `crates/mnemo-mcp/src/tools.rs`
- Add `delegate` tool: calls `POST /api/v1/regions` + `POST /api/v1/regions/:id/acl`
- Add `revoke` tool: calls `DELETE /api/v1/regions/:id/acl/:agent_id`
- Add `scopes` tool: calls `GET /api/v1/regions?agent_id=<self>`
- Update tool descriptions to be agent-optimized (concise, action-oriented)
- Add tests (standard + adversarial) for each new tool

**Breaking change:** Tool rename is a breaking change for existing MCP integrations.
Mitigate by accepting both old and new names during a deprecation window (one minor
version). Log a warning when old names are used.

### D3: Dashboard Memory Explorer

**Current gap:** The dashboard can visualize entities and graph structure (Explorer
page) but cannot show actual memory content — episode text, fact assertions, temporal
timeline of what was true when, search results for a query.

**New "Memory" page at `/_/memory`:**

1. **User selector** — dropdown or search box listing known users
2. **Episode timeline** — chronological list of episodes for selected user, showing
   role, content preview (first 200 chars), timestamp, session name
3. **Fact browser** — table of current facts: subject, predicate, object, valid_at,
   classification, confidence. Toggle to show superseded facts (greyed out with
   invalid_at timestamp).
4. **Memory search** — text input that calls `/api/v1/memory/{user}/context` and
   displays the assembled context, token count, matched entities, retrieval mode,
   and latency. Shows what an agent would receive.
5. **Temporal diff** — select two timestamps, show what facts were added/superseded
   between them (uses existing `/changes_since` endpoint).

**Implementation:**
- Add new page section to `dashboard/index.html`
- Add corresponding JS module in `dashboard/app.js`
- Style with existing CSS variables
- No new backend endpoints needed — all data available through existing API

**Success criterion:** An operator can search a user's memory from the dashboard
and see exactly what context an agent would receive, including temporal state.

### D4: SDK Ergonomics Pass

**Python SDK fixes (in priority order):**

1. **Add missing deletion methods:** `delete_user()`, `delete_session()`,
   `delete_entity()`, `delete_edge()` — these exist as REST endpoints and in the
   TS SDK but are missing from Python.
2. **Fix async code duplication:** Extract shared parsing logic into `_parsers.py`
   module. Both `client.py` and `async_client.py` import from it. Estimated
   reduction: ~600 lines removed from `async_client.py`.
3. **Type sub-structures:** Replace `list[dict[str, Any]]` for entities/facts/episodes
   in `ContextResult` with typed dataclasses (`ContextEntity`, `ContextFact`,
   `ContextEpisode`). Non-breaking: fields still accessible by key via
   `__getitem__` if needed.
4. **Add pagination helpers:** `for entity in client.iter_entities("user"):`
   auto-paginating generator. Same for `iter_sessions`, `iter_messages`.

**TypeScript SDK fixes:**

1. **Add 429 retry with backoff** matching Python behavior. Respect `Retry-After`
   header.
2. **Add `MnemoConnectionError` and `MnemoTimeoutError`** subtypes to match Python
   hierarchy.
3. **Add `contextHead()` convenience method.**
4. **Type `graphEntity()` return** — use `GraphEntityDetail` interface instead of
   `Record<string, unknown>`.
5. **Add `getPolicyViolations()`** — referenced in README but not implemented.

**Both SDKs:**
- Add event type enum/union for webhook creation (`FactAdded`, `FactSuperseded`,
  `HeadAdvanced`, `ConflictDetected`, etc.) instead of raw strings.

### D5: MCP Doc Comment Fix

Fix the `main.rs` doc comment in `mnemo-mcp` that says default port is 3000 when
the code defaults to 8080. Trivial but misleading.

---

## Non-Goals

- **SSE/HTTP transport for MCP.** Stdio is the standard transport for local MCP
  integrations. SSE would be needed for remote/hosted MCP, which is a v1.0.0 concern.
- **Builder/fluent API for SDKs.** Nice-to-have but not blocking adoption. The
  current options-object pattern works.
- **Connection pooling in Python sync client.** stdlib urllib creates new connections
  per request but this is fine for typical SDK usage patterns.
- **SDK test suite for TypeScript.** Important but not blocking DX. Track separately.

## Risks

1. **Docker image size.** Adding `mnemo-mcp-server` to the Docker image adds a
   second binary. The multi-stage Dockerfile should keep this small but verify.
2. **Quickstart without API keys is insecure.** The quickstart intentionally runs
   without auth for simplicity. Must prominently warn: "This is for local
   development only. Enable API keys before exposing to a network."
3. **Tool rename breaks existing integrations.** Mitigate with deprecation window
   (accept both names, log warning on old name).
4. **Dashboard memory explorer could expose sensitive data.** The dashboard already
   bypasses API key auth. Adding memory content browsing makes this more dangerous.
   Consider: add an optional `MNEMO_DASHBOARD_AUTH` env var that requires a bearer
   token for `/_/` routes.

## Success Criteria

- [ ] Zero-to-memory quickstart completed by a new developer in under 5 minutes
- [ ] MCP tools renamed and 3 new tools (delegate, revoke, scopes) working
- [ ] Dashboard Memory page shows episodes, facts, search, and temporal diff
- [ ] Python SDK: deletion methods, shared parsers, typed sub-structures, pagination
- [ ] TypeScript SDK: 429 retry, error subtypes, contextHead, typed graphEntity
- [ ] Both SDKs: event type enums for webhooks
- [ ] MCP doc comment fixed
