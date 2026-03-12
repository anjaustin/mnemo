# Mnemo 0.6.0 Roadmap — Enterprise Access Control

**Status**: In Progress
**Baseline**: v0.5.5 (Autonomic Memory: confidence decay, self-healing, narratives, goal-conditioned retrieval, counterfactual memory)
**Theme**: Make Mnemo enterprise-ready. Every memory operation should be governed by who is asking, what they're allowed to see, and what the organization's policies enforce — without sacrificing the single-binary simplicity.

---

## Design Principle

v0.5.5 made memory autonomic — self-healing, self-narrating, confidence-aware. But the access model is still flat: one API key, one user owns all their data, every caller sees everything. Enterprise customers need:

1. **Caller identity** — who is making this request? (not just "which user's data")
2. **Scoped access** — what is this caller allowed to do? (read-only analyst vs. write-capable agent vs. admin)
3. **Data classification** — which facts/entities are safe for which audience? (support-safe vs. internal-only)
4. **Policy enforcement** — declarative rules that block unsafe operations before they happen
5. **Multi-agent coordination** — agents that share memory with explicit access boundaries

The existing foundation is stronger than it looks:
- `UserPolicyRecord` already enforces retention windows, webhook domain allowlists, memory contracts, and retrieval policies per user — with governance audit trails.
- `apply_memory_contract()` already filters context by contract type (`SupportSafe`, `CurrentStrict`, `HistoricalStrict`).
- `validate_identity_core()` already blocks contamination of agent identity.
- Witness chain audit provides tamper-evident logging.
- The `metadata` JSON field on Edge and Entity is available for classification labels.

v0.6.0 builds the **authorization and classification** layer on top of this governance foundation.

---

## Execution Order

| # | Feature | Competitive Impact | Build Effort | Dependency |
|---|---------|-------------------|-------------|------------|
| 1 | Scoped API Keys | Critical — enterprise gate | Medium | None |
| 2 | Data Classification Labels | High — compliance differentiator | Medium | None |
| 3 | Policy-Scoped Memory Views | High — multi-audience context | Medium | #1 + #2 |
| 4 | Memory Guardrails Engine | High — declarative safety | Medium | #2 + #3 |
| 5 | Agent Identity Phase B | Medium — governance maturity | Medium | #1 |
| 6 | Multi-Agent Shared Memory | High — platform play | High | #1 + #2 |

Features 1 and 2 can be built in parallel (no dependency). Features 3-6 depend on the foundation laid by 1 and 2.

---

## Feature 1: Scoped API Keys

### What it is

Replace the current single-secret API key with a key management system that binds each key to a **caller identity**, a **role** (read, write, admin), and an optional **scope** (user IDs, agent IDs, or operations the key is authorized for).

### Why

Currently `MNEMO_API_KEY` is a single shared secret. Every caller has full access to every user's data. This is a non-starter for:
- Enterprise teams with analysts who should only read, not write
- Multi-service architectures where each service gets its own key
- Audit trails that need to attribute actions to specific callers
- SOC 2 CC6.1 (Logical Access Controls)

### Current state

- `MNEMO_API_KEY` env var checked in middleware (`api_key_auth` in routes.rs)
- `MNEMO_AUTH_ENABLED` flag (default false in dev, true in prod)
- No caller identity extracted from key
- No role/scope enforcement
- `x-mnemo-request-id` propagated but not tied to a caller

### Architecture

**Key model:**

```rust
pub struct ApiKey {
    pub id: Uuid,
    pub name: String,                    // human label ("analytics-reader", "agent-svc")
    pub key_hash: String,                // SHA256(key) — raw key never stored
    pub role: ApiKeyRole,                // Read, Write, Admin
    pub scope: Option<ApiKeyScope>,      // optional fine-grained restrictions
    pub created_by: Option<String>,      // who created this key
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

pub enum ApiKeyRole {
    Read,       // GET endpoints only, context retrieval, graph queries
    Write,      // Read + POST/PUT/PATCH on memory, sessions, episodes
    Admin,      // Write + policy management, key management, user deletion, agent identity
}

pub struct ApiKeyScope {
    pub allowed_user_ids: Option<Vec<Uuid>>,    // restrict to specific users
    pub allowed_agent_ids: Option<Vec<String>>,  // restrict to specific agents
    pub max_classification: Option<Classification>, // max data classification level
}
```

**Request context:**

Every authenticated request extracts a `CallerContext` from the API key, threaded through handlers:

```rust
pub struct CallerContext {
    pub key_id: Uuid,
    pub key_name: String,
    pub role: ApiKeyRole,
    pub scope: Option<ApiKeyScope>,
}
```

**Enforcement points:**
- Middleware: extract `CallerContext` from `x-api-key` header, reject if revoked/expired
- Route-level: each handler checks `caller.role` against required minimum role
- Data-level: `allowed_user_ids` scope restricts which user paths a key can access

**API endpoints:**
- `POST /api/v1/keys` — create key (Admin only). Returns raw key once; only hash stored.
- `GET /api/v1/keys` — list keys (Admin only). Excludes raw key.
- `DELETE /api/v1/keys/:id` — revoke key (Admin only)
- `POST /api/v1/keys/:id/rotate` — rotate key (Admin only). Invalidates old, returns new.

**Backward compatibility:**
- `MNEMO_API_KEY` env var continues to work as a "bootstrap admin key"
- When no scoped keys exist, the bootstrap key has Admin role
- When `MNEMO_AUTH_ENABLED=false`, all requests get implicit Admin context

**Storage:** Redis, keyed by `{prefix}api_key:{id}` (JSON) + `{prefix}api_keys` (sorted set) + `{prefix}api_key_hash:{hash}` (lookup index).

**Files touched:** `mnemo-core/src/models/` (new `api_key.rs`), `mnemo-core/src/traits/storage.rs` (new `ApiKeyStore` trait), `mnemo-storage/src/redis_store.rs`, `mnemo-server/src/routes.rs` (middleware + key CRUD), `mnemo-server/src/state.rs`

### Success criteria

- Bootstrap key retains full Admin access
- Read-scoped key cannot POST episodes or modify policies
- User-scoped key cannot access other users' data
- Revoked key returns 401 immediately
- Key rotation produces new key, old key rejected within 1 request
- Governance audit records which key performed each operation
- 12+ falsification tests covering role escalation, scope bypass, expired keys, revoked keys

---

## Feature 2: Data Classification Labels

### What it is

Add a `classification` field to Edge and Entity models that categorizes data by sensitivity level. Classification flows from entities/edges into context assembly, enabling policy-scoped views.

### Why

"Policy-Scoped Memory Views" (Feature 3) needs something to filter on. Today, all facts are equally visible. With classification labels, organizations can mark facts as `public`, `internal`, `confidential`, or `restricted` — and retrieval can enforce "this support agent only sees `public` + `internal` data."

### Current state

- Edge and Entity have no visibility/classification fields
- `metadata` JSON field exists on both but has no schema enforcement
- `EdgeFilter` has no classification parameter
- Memory contracts (`SupportSafe`, etc.) provide coarse filtering but not per-fact granularity

### Architecture

**Classification enum:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Classification {
    Public,        // safe for any audience (customers, external agents)
    Internal,      // safe for internal agents and operators
    Confidential,  // restricted to authorized agents/users
    Restricted,    // highest sensitivity — PII, financial, health data
}
```

The enum derives `PartialOrd`/`Ord` so that `Public < Internal < Confidential < Restricted` — a caller with `max_classification = Internal` can see `Public` and `Internal` but not `Confidential` or `Restricted`.

**Model changes:**

```rust
// Edge — add field
pub classification: Classification,  // default: Classification::Internal

// Entity — add field
pub classification: Classification,  // default: Classification::Internal
```

Default is `Internal` (not `Public`) to enforce a secure-by-default posture: new data is internal unless explicitly declassified.

**Auto-classification during ingestion:**

The LLM extraction prompt is extended to include a classification hint. The prompt asks the LLM to flag facts that appear to contain PII, financial data, or health information. Facts flagged as sensitive are created with `Confidential` or `Restricted` classification. This is best-effort — operators can override via API.

**Classification override API:**

- `PATCH /api/v1/entities/:id/classification` — set classification (Admin or Write role)
- `PATCH /api/v1/edges/:id/classification` — set classification (Admin or Write role)
- Bulk: `POST /api/v1/users/:user/classify` — reclassify entities/edges matching a filter

**Filter integration:**

- `EdgeFilter` gains `max_classification: Option<Classification>` field
- Entity listing gains `max_classification` query parameter
- Qdrant payload: `classification` field indexed for pre-filtering

**Files touched:** `mnemo-core/src/models/edge.rs`, `mnemo-core/src/models/entity.rs`, `mnemo-core/src/models/` (new `classification.rs`), `mnemo-storage/src/redis_store.rs`, `mnemo-storage/src/qdrant_store.rs`, `mnemo-server/src/routes.rs`, `mnemo-ingest/src/lib.rs` (LLM prompt extension)

### Success criteria

- All new edges/entities default to `Internal` classification
- LLM extraction auto-classifies PII-adjacent facts as `Confidential`
- `EdgeFilter` with `max_classification=Public` excludes `Internal`+ facts
- Qdrant payload index on `classification` enables pre-filtered ANN search
- PATCH endpoints validate caller has Admin or Write role
- Existing data migration: add `classification: "internal"` to all existing edges/entities on startup
- 10+ falsification tests covering classification ordering, filter enforcement, auto-classification, upgrade path

---

## Feature 3: Policy-Scoped Memory Views

### What it is

Multiple retrieval "lenses" over the same memory, controlled by the caller's role and the data's classification. A support agent gets a `support_safe` view (public + internal facts, no confidential PII). A sales agent gets a `sales` view (public + internal + customer preferences, no restricted financial data). An admin gets everything.

### Why

This is FACE_MELTER_FEATURES.md item 5. It's the enterprise version of "least-privilege context assembly" — the agent only sees what it should see, governed by the organization's policies, not by the individual developer's judgment.

### Current state

- `apply_memory_contract()` provides 4 coarse-grained contracts: `Default`, `SupportSafe`, `CurrentStrict`, `HistoricalStrict`
- `SupportSafe` suppresses certain categories but uses hardcoded logic, not configurable rules
- `UserPolicyRecord.default_memory_contract` selects the default contract per user
- No caller-identity-based view selection
- No configurable view definitions

### Architecture

**View definitions:**

```rust
pub struct MemoryView {
    pub id: Uuid,
    pub name: String,                           // "support_safe", "sales", "internal_full"
    pub description: String,
    pub max_classification: Classification,     // ceiling for this view
    pub allowed_entity_types: Option<Vec<String>>,  // whitelist entity types (None = all)
    pub blocked_edge_labels: Option<Vec<String>>,   // blacklist specific relationship types
    pub max_facts: Option<u32>,                 // cap fact count in context
    pub include_narrative: bool,                // include narrative summary
    pub temporal_scope: Option<TemporalScope>,  // restrict to recent N days
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum TemporalScope {
    LastNDays(u32),
    Since(DateTime<Utc>),
    CurrentOnly,  // only currently-valid facts
}
```

**View resolution order:**

1. Explicit `view` parameter in request body → use that view definition
2. Caller's API key `scope.max_classification` → auto-select most permissive view the caller qualifies for
3. User's `default_memory_contract` → fall back to existing contract logic
4. Default: `Internal` classification ceiling with no entity/label restrictions

**Integration with `get_memory_context()`:**

The `MemoryContextRequest` gains an optional `view: Option<String>` parameter. When provided:
1. Look up `MemoryView` by name
2. Verify caller's key role + scope permits this view (caller's `max_classification` >= view's `max_classification`)
3. Apply view constraints to the retrieval pipeline:
   - `max_classification` → filter entities and edges before context assembly
   - `allowed_entity_types` → restrict entity search to allowed types
   - `blocked_edge_labels` → exclude specific relationship types from facts
   - `temporal_scope` → restrict to time window
   - `max_facts` → truncate fact list
4. Emit governance audit event recording which view was used

**API endpoints:**

- `POST /api/v1/views` — create view definition (Admin only)
- `GET /api/v1/views` — list view definitions
- `GET /api/v1/views/:name` — get view definition
- `PUT /api/v1/views/:name` — update view definition (Admin only)
- `DELETE /api/v1/views/:name` — delete view definition (Admin only)

**Storage:** Redis, keyed by `{prefix}memory_view:{name}` (JSON) + `{prefix}memory_views` (sorted set).

**Files touched:** `mnemo-core/src/models/` (new `view.rs`), `mnemo-core/src/traits/storage.rs` (new `ViewStore` trait), `mnemo-storage/src/redis_store.rs`, `mnemo-server/src/routes.rs`, `mnemo-server/src/state.rs`, `mnemo-retrieval/src/lib.rs`

### Success criteria

- Support-scoped caller cannot see `Confidential` facts via any view
- View with `blocked_edge_labels: ["salary"]` excludes salary facts from context
- View with `allowed_entity_types: ["person", "product"]` excludes organization entities
- Governance audit records view name and classification ceiling per context request
- `TemporalScope::LastNDays(30)` excludes facts older than 30 days
- View CRUD restricted to Admin role
- 12+ falsification tests covering classification ceiling bypass, label blocking, entity type filtering, temporal scope, cross-view isolation

---

## Feature 4: Memory Guardrails Engine

### What it is

Declarative constraint rules evaluated at both write time (ingestion) and read time (retrieval) that block unsafe operations before they execute. Rules are defined per-user or globally and enforced automatically — no code changes needed to add a new guardrail.

### Why

This is FACE_MELTER_FEATURES.md item 9. Today, policy enforcement is scattered across handlers: retention checks in one place, domain allowlisting in another, contract filtering in a third. The guardrails engine unifies these into a single evaluation pipeline with a declarative rule format.

### Current state

- Retention enforcement: `validate_episode_retention()` and `is_episode_within_retention()`
- Webhook domain allowlisting: `is_target_url_allowed()`
- TLS enforcement: `state.require_tls`
- Memory contracts: `apply_memory_contract()` (read-path filtering)
- Identity contamination guard: `validate_identity_core()`
- No unified rule engine — each check is a bespoke function

### Architecture

**Rule model:**

```rust
pub struct GuardrailRule {
    pub id: Uuid,
    pub name: String,                        // "block_pii_storage", "restrict_health_data"
    pub description: String,
    pub trigger: GuardrailTrigger,           // when does this rule fire?
    pub condition: GuardrailCondition,       // what does it check?
    pub action: GuardrailAction,            // what happens on match?
    pub priority: u32,                       // lower = evaluated first
    pub enabled: bool,
    pub scope: GuardrailScope,              // global or per-user
    pub created_at: DateTime<Utc>,
}

pub enum GuardrailTrigger {
    OnIngest,           // evaluated when episodes are ingested
    OnFactCreation,     // evaluated when edges are created from extraction
    OnRetrieval,        // evaluated during context assembly
    OnEntityCreation,   // evaluated when entities are created
    OnAny,              // all of the above
}

pub enum GuardrailCondition {
    ClassificationAbove(Classification),           // fact classification > threshold
    EntityTypeIn(Vec<String>),                     // entity type matches list
    EdgeLabelIn(Vec<String>),                      // edge label matches list
    ContentMatchesRegex(String),                   // content matches regex pattern
    CallerRoleBelow(ApiKeyRole),                   // caller has insufficient role
    FactAgeAboveDays(u32),                         // fact older than N days
    ConfidenceBelow(f32),                          // fact confidence below threshold
    And(Vec<GuardrailCondition>),                  // all conditions must match
    Or(Vec<GuardrailCondition>),                   // any condition must match
    Not(Box<GuardrailCondition>),                  // negation
}

pub enum GuardrailAction {
    Block { reason: String },                      // reject the operation with error message
    Redact,                                         // remove the matching content from output
    Reclassify(Classification),                    // upgrade classification of the fact
    AuditOnly { severity: String },                // allow but log to governance audit
    Warn { message: String },                      // allow but include warning in response
}

pub enum GuardrailScope {
    Global,                                        // applies to all users
    User(Uuid),                                    // applies to specific user
}
```

**Evaluation pipeline:**

```
Incoming operation (ingest / retrieval / entity creation)
    │
    ▼
Load applicable rules (global + user-specific, sorted by priority)
    │
    ▼
For each rule where trigger matches operation type:
    │
    ├─ Evaluate condition tree against the data
    │
    ├─ If condition matches:
    │   ├─ Block → return error with reason
    │   ├─ Redact → strip content from response
    │   ├─ Reclassify → upgrade classification in-place
    │   ├─ AuditOnly → log to governance audit, continue
    │   └─ Warn → add warning header, continue
    │
    └─ If no match → continue to next rule
    │
    ▼
Operation proceeds (or was blocked)
```

**API endpoints:**

- `POST /api/v1/guardrails` — create rule (Admin only)
- `GET /api/v1/guardrails` — list rules (optional `?scope=global&scope=user:{id}`)
- `GET /api/v1/guardrails/:id` — get rule
- `PUT /api/v1/guardrails/:id` — update rule (Admin only)
- `DELETE /api/v1/guardrails/:id` — delete rule (Admin only)
- `POST /api/v1/guardrails/evaluate` — dry-run: evaluate rules against sample data without executing

**Integration points:**

- `mnemo-ingest`: call guardrails engine after LLM extraction, before edge/entity creation
- `mnemo-server/routes.rs`: call guardrails engine in `get_memory_context()` before context assembly
- `mnemo-server/routes.rs`: call guardrails engine in episode creation handler

**Storage:** Redis, keyed by `{prefix}guardrail:{id}` (JSON) + `{prefix}guardrails` (sorted set) + `{prefix}guardrails_user:{user_id}` (user-specific sorted set).

**Files touched:** `mnemo-core/src/models/` (new `guardrail.rs`), `mnemo-core/src/traits/storage.rs` (new `GuardrailStore` trait), `mnemo-storage/src/redis_store.rs`, `mnemo-server/src/routes.rs`, `mnemo-ingest/src/lib.rs`, `mnemo-retrieval/src/lib.rs`

### Success criteria

- `Block` rule with `ContentMatchesRegex("\\bSSN\\b")` prevents storage of episodes containing SSN references
- `Reclassify` rule auto-upgrades facts containing "salary" to `Restricted`
- `Redact` rule strips `Confidential` facts from `Public`-scoped retrieval
- `AuditOnly` rule logs access to sensitive data without blocking
- Condition combinators (`And`, `Or`, `Not`) compose correctly
- Priority ordering is deterministic: lower-priority rules evaluated first
- Global rules apply to all users; user-specific rules override global
- Dry-run endpoint returns which rules would fire without executing actions
- 15+ falsification tests covering rule priority, condition combinators, action enforcement, scope isolation, regex injection safety

---

## Feature 5: Agent Identity Phase B — Governance + Conflict Handling

### What it is

Extend the agent identity promotion workflow with multi-approver governance, risk-based escalation, conflict detection between experience signals and core identity, and webhook notifications for pending proposals.

### Why

Phase A (shipped in v0.4.0) provides the basic approve/reject workflow. But enterprise teams need:
- Multiple approvers for high-risk identity changes
- Automatic escalation based on risk level
- Detection of conflicting experience signals before they reach promotion
- Notification when proposals need attention

### Current state

- `PromotionProposal` with single approve/reject
- `risk_level` field stored but not enforced
- No notification on pending proposals
- No conflict detection between experience events
- No governance audit trail on promotions (unlike policy changes)
- COW branching allows safe experimentation

### Architecture

**Approval policy:**

```rust
pub struct ApprovalPolicy {
    pub agent_id: String,
    pub low_risk: ApprovalRequirement,      // e.g., 1 approver
    pub medium_risk: ApprovalRequirement,   // e.g., 2 approvers
    pub high_risk: ApprovalRequirement,     // e.g., 3 approvers + cooling period
}

pub struct ApprovalRequirement {
    pub min_approvers: u32,
    pub cooling_period_hours: Option<u32>,  // mandatory wait before auto-apply
    pub auto_reject_after_hours: Option<u32>, // expire if not approved in time
}
```

**Experience conflict detection:**

Before creating a promotion proposal, the system scans existing experience events for signals that contradict the proposed identity change. For example: if 20 experience events say "users prefer concise responses" but the proposal makes the agent more verbose, flag it as conflicting.

```rust
pub struct ConflictAnalysis {
    pub proposal_id: Uuid,
    pub supporting_signals: Vec<Uuid>,    // experience events that align
    pub conflicting_signals: Vec<Uuid>,   // experience events that oppose
    pub conflict_score: f32,              // 0.0 (no conflict) to 1.0 (strong conflict)
    pub recommendation: ConflictRecommendation,
}

pub enum ConflictRecommendation {
    Proceed,             // low conflict, safe to approve
    ReviewConflicts,     // moderate conflict, needs human review
    Reject,              // high conflict, experience evidence opposes this change
}
```

**Webhook notifications:**

- `promotion_proposed` — emitted when a new proposal is created
- `promotion_approved` — emitted when approval threshold is met
- `promotion_rejected` — emitted on rejection
- `promotion_expired` — emitted when auto-reject timer fires
- `promotion_conflict_detected` — emitted when conflict analysis finds opposing signals

**Governance audit integration:**

All promotion actions (propose, approve, reject, expire) now emit `GovernanceAuditRecord` entries with the proposal ID, agent ID, and action details.

**New endpoints:**

- `GET /api/v1/agents/:agent_id/promotions/:id/conflicts` — conflict analysis
- `PUT /api/v1/agents/:agent_id/approval-policy` — set approval policy (Admin only)
- `GET /api/v1/agents/:agent_id/approval-policy` — get approval policy

**Modified endpoints:**

- `POST /api/v1/agents/:agent_id/promotions/:id/approve` — now records approver identity (from `CallerContext`) and checks against `ApprovalRequirement.min_approvers`
- `POST /api/v1/agents/:agent_id/promotions` — now triggers conflict analysis before creating proposal

**Files touched:** `mnemo-core/src/models/agent.rs`, `mnemo-core/src/traits/storage.rs`, `mnemo-storage/src/redis_store.rs`, `mnemo-server/src/routes.rs`, `mnemo-server/src/state.rs`

### Success criteria

- High-risk proposal requires 3 approvals before applying
- Single approval on a high-risk proposal leaves it in `pending` state
- Conflict analysis detects opposing experience signals with > 0.5 score
- Expired proposals auto-reject after configured timeout
- All promotion actions appear in governance audit trail
- Webhook events fire for each promotion lifecycle stage
- 12+ falsification tests covering multi-approver quorum, risk escalation, conflict detection, timeout expiry, audit trail completeness

---

## Feature 6: Multi-Agent Shared Memory with ACLs

### What it is

Allow multiple agents to read from and write to overlapping memory regions with explicit access control. Agent A can share specific entities/facts with Agent B without sharing everything. Access is governed by ACL rules, not by copying data.

### Why

Real enterprise deployments have multiple agents: a support agent, a sales agent, a scheduling agent. They need to share some memory (customer name, account status) but not all (support tickets shouldn't leak to sales, purchase history shouldn't leak to support unless explicitly shared).

### Current state

- All data is siloed by `user_id` at the storage layer
- No `agent_id` on Edge or Entity (entities belong to users, not agents)
- No ACL model anywhere in the codebase
- Agent identities are independent — no cross-agent visibility
- Agent forking (`fork_agent`) copies experience, not user memory

### Architecture

**Memory region model:**

```rust
pub struct MemoryRegion {
    pub id: Uuid,
    pub name: String,                    // "shared_customer_context", "support_internal"
    pub owner_agent_id: String,          // agent that created this region
    pub user_id: Uuid,                   // user whose data this region covers
    pub entity_filter: Option<EntityFilter>,  // which entities are in this region
    pub edge_filter: Option<EdgeFilter>,      // which edges are in this region
    pub classification_ceiling: Classification, // max classification in this region
    pub created_at: DateTime<Utc>,
}

pub struct MemoryRegionAcl {
    pub region_id: Uuid,
    pub agent_id: String,               // agent being granted access
    pub permission: RegionPermission,    // what they can do
    pub granted_by: String,              // who granted this access
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub enum RegionPermission {
    Read,       // can retrieve facts from this region
    Write,      // can add facts to this region
    Manage,     // can modify the region definition and ACLs
}
```

**How it works:**

1. Agent A creates a memory region scoping which entities/edges to share
2. Agent A grants Agent B `Read` access to that region
3. When Agent B calls `get_memory_context()`, the retrieval pipeline includes facts from regions where Agent B has `Read` access — in addition to Agent B's own memory
4. When Agent B ingests an episode with `region_id`, new facts are tagged with that region
5. Region filters compose with view constraints and guardrail rules

**Integration with retrieval:**

`get_memory_context()` gains an optional `agent_id` parameter. When provided:
1. Look up all regions where this agent has `Read` or `Write` access
2. For each region, apply the region's `entity_filter`, `edge_filter`, and `classification_ceiling`
3. Union the results with the agent's own direct memory
4. Apply the caller's view constraints on top (Feature 3)
5. Apply guardrail rules on top (Feature 4)

**API endpoints:**

- `POST /api/v1/regions` — create memory region (Write or Admin role)
- `GET /api/v1/regions` — list regions (filtered by agent_id access)
- `GET /api/v1/regions/:id` — get region details
- `PUT /api/v1/regions/:id` — update region (Manage permission)
- `DELETE /api/v1/regions/:id` — delete region (Manage permission)
- `POST /api/v1/regions/:id/acl` — grant access to an agent
- `GET /api/v1/regions/:id/acl` — list access grants
- `DELETE /api/v1/regions/:id/acl/:agent_id` — revoke access

**Storage:** Redis, keyed by `{prefix}region:{id}` (JSON) + `{prefix}regions` (sorted set) + `{prefix}region_acl:{region_id}` (sorted set of ACL entries) + `{prefix}agent_regions:{agent_id}` (reverse index: regions an agent can access).

**Files touched:** `mnemo-core/src/models/` (new `region.rs`), `mnemo-core/src/traits/storage.rs` (new `RegionStore` trait), `mnemo-storage/src/redis_store.rs`, `mnemo-server/src/routes.rs`, `mnemo-retrieval/src/lib.rs`

### Success criteria

- Agent A creates region, grants Agent B read access; Agent B retrieves shared facts
- Agent B cannot write to a Read-only region
- Revoking access immediately prevents retrieval
- Region's `classification_ceiling` enforced even if the ACL grants access
- Expired ACL entries are ignored
- Region filtering composes correctly with views and guardrails
- Cross-user region creation is blocked (can only share within a user's data)
- 15+ falsification tests covering ACL enforcement, permission escalation, region isolation, expired grants, classification ceiling override attempts, cross-user boundary

---

## Cross-Cutting Concerns

### Migration Path

Existing deployments upgrading from v0.5.5:
- All existing edges/entities get `classification: Internal` on first access (lazy migration)
- Existing `MNEMO_API_KEY` becomes the bootstrap Admin key
- No memory views or guardrail rules exist by default — behavior is unchanged until configured
- Existing memory contracts (`SupportSafe`, etc.) continue to work alongside the new view system

### SDK Updates

Both Python and TypeScript SDKs need:
- Key management methods (`create_key`, `list_keys`, `revoke_key`, `rotate_key`)
- Classification methods (`classify_entity`, `classify_edge`)
- View CRUD methods
- Guardrail CRUD methods
- Region and ACL management methods
- `view` parameter on `context()` / `get_context()`
- `agent_id` parameter on `context()` for multi-agent retrieval

### Documentation

- `docs/API.md` — all new endpoints
- `docs/ARCHITECTURE.md` — updated storage traits, new models
- `docs/SECURITY_CONTROLS.md` — updated SOC 2 mappings for RBAC and classification
- `CHANGELOG.md` — v0.6.0 entry

### Test Budget

Each feature follows the cycle: **code -> test -> falsify -> document -> commit -> push**.

| Feature | Unit tests | Integration tests | Falsification tests |
|---------|-----------|-------------------|-------------------|
| Scoped API Keys | 10 | 8 | 12 |
| Data Classification | 8 | 6 | 10 |
| Policy-Scoped Views | 8 | 8 | 12 |
| Guardrails Engine | 12 | 10 | 15 |
| Identity Phase B | 8 | 8 | 12 |
| Multi-Agent ACLs | 10 | 10 | 15 |
| **Total** | **56** | **50** | **76** |

---

## Future (v0.6.5 — Qdrant-Native Scale)

Deferred:
- Named vectors / multi-vector points
- Hybrid sparse + dense search
- Grouped search (session-balanced context)
- Quantization + HNSW tuning
- Aliases + snapshots (zero-downtime migrations)
- Sharding and replication controls

## Future (v0.7.0 — Agent Identity Phase C)

Deferred:
- Organization-wide multi-agent identity templates
- Cross-agent shared identity contracts
- Identity marketplace (publish/subscribe agent personality modules)
