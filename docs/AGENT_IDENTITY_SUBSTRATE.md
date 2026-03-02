# Agent Identity Substrate (P0 Spec)

Date: 2026-03-02
Status: implemented-p0

## Delivered in P0

- Identity profile endpoints (`get`, `update`) with versioning.
- Experience ingestion and listing integrated into identity-aware context.
- Identity contamination guardrails on core writes.
- Identity snapshot history + append-only audit events.
- Identity rollback endpoint with audit trail.
- Promotion proposal workflow (`pending`, `approve`, `reject`) with evidence gating.
- Integration and adversarial falsification coverage for contamination, rollback, and promotion flow.

## 1) Problem

LLMs can blur boundaries between:

- the assistant's own identity,
- user-provided context,
- shared interaction history.

Over long-lived usage, this causes:

- identity drift,
- self/user attribution errors,
- unstable behavior across sessions and users.

## 2) Goal

Add a Mnemo-native layer that gives agents:

1. **homeostatic identity** (stable core self-model), and
2. **homeoadaptive learning** (controlled evolution from experience),

without conflating user memory with agent identity.

## 3) Non-goals (v1)

- autonomous rewriting of core values without policy checks
- model fine-tuning
- replacing existing memory context endpoint

## 4) Core concepts

### 4.1 Identity Core (slow-changing)

Canonical assistant self-state:

- mission / operating intent
- tone and interaction style bounds
- hard safety boundaries
- tool/capability posture
- explicit refusal and escalation policies

Update policy: strict, low-frequency, auditable.

### 4.2 Experience Layer (fast-changing)

Interaction-derived, confidence-scored observations:

- what response patterns worked
- what failed and why
- user collaboration preferences (session/local)

Update policy: frequent, decayed, reversible.

### 4.3 Separation contract

Memory planes are explicit and isolated:

- `identity_core`
- `experience`
- `user_memory`
- `interaction_memory`

No write path may place user facts in `identity_core`.

## 5) Data model (v1)

## 5.1 Identity profile

```json
{
  "agent_id": "support-agent-v1",
  "version": 12,
  "core": {
    "mission": "Resolve user issues accurately and safely.",
    "style": {
      "tone": "calm_direct",
      "verbosity": "medium"
    },
    "boundaries": {
      "never_claim_human_experience": true,
      "never_store_restricted_pii": true
    },
    "capabilities": {
      "tool_use": "allowed_with_confirmation_for_high_risk"
    }
  },
  "updated_at": "2026-03-02T00:00:00Z"
}
```

## 5.2 Experience event

```json
{
  "id": "evt_...",
  "agent_id": "support-agent-v1",
  "session_id": "...",
  "user_id": "...",
  "category": "interaction_pattern",
  "signal": "user_prefers_bulleted_action_plans",
  "confidence": 0.78,
  "weight": 0.62,
  "decay_half_life_days": 30,
  "evidence_episode_ids": ["..."],
  "created_at": "2026-03-02T00:00:00Z"
}
```

## 5.3 Promotion proposal

```json
{
  "id": "prom_...",
  "agent_id": "support-agent-v1",
  "proposal": "increase directness in troubleshooting guidance",
  "reason": "repeated successful outcomes in 40 sessions",
  "source_events": ["evt_..."],
  "risk_level": "medium",
  "status": "pending_review"
}
```

## 6) API surface (v1)

### 6.1 Identity read

- `GET /api/v1/agents/:agent_id/identity`

Returns current Identity Core + version metadata.

### 6.2 Identity update (guarded)

- `PUT /api/v1/agents/:agent_id/identity`

Only allowed for trusted operators/policies. Produces immutable audit event.

### 6.3 Experience ingestion

- `POST /api/v1/agents/:agent_id/experience`

Appends confidence-scored experience events.

### 6.4 Identity-aware context

- `POST /api/v1/agents/:agent_id/context`

Assembles context in deterministic order:

1. Identity Core (stable)
2. Relevant Experience Layer (adaptive)
3. User/interaction memory (task-specific)

## 7) Retrieval/assembly contract

Identity-aware context response must include diagnostics:

```json
{
  "identity_version": 12,
  "experience_events_used": 7,
  "experience_weight_sum": 3.42,
  "user_memory_items_used": 14,
  "attribution_guards": {
    "self_user_separation_enforced": true
  }
}
```

## 8) Update policy

### 8.1 Homeostatic rules (core)

- Core edits require explicit update endpoint.
- Optional multi-step approval for high-risk fields.
- Every edit creates append-only audit record.

### 8.2 Homeoadaptive rules (experience)

- Experience decays over time unless reinforced.
- Signals below confidence threshold are ignored.
- Promotion to core requires:
  - repeated evidence,
  - stability window,
  - policy approval.

## 9) Safety and guardrails

- hard block: user-identity facts cannot be written to `identity_core`
- hard block: identity writes from untrusted channels
- immutable audit stream for core mutations
- rollback by identity version

## 10) Falsification plan (must-pass)

1. **Self/user conflation test**
   - user says "I am a doctor"; assistant identity must not adopt this.

2. **Identity drift resistance test**
   - repeated adversarial prompts must not mutate protected core fields.

3. **Adaptive learning test**
   - repeated successful interaction pattern should alter experience weighting, not core immediately.

4. **Promotion gating test**
   - insufficient evidence cannot promote experience to core.

5. **Rollback integrity test**
   - revert to prior identity version must restore deterministic context behavior.

## 11) Metrics

- identity consistency score across sessions
- self/user attribution error rate
- drift incident rate (core mutation anomalies)
- adaptive gain (task success delta from experience layer)
- rollback frequency and recovery time

## 12) Rollout plan

### Phase A (P0)

- data model + identity/experience endpoints
- identity-aware context assembly
- diagnostics + falsification tests

### Phase B

- promotion workflow
- governance UI/policy tooling
- richer conflict handling between core and experience

### Phase C

- organization-wide multi-agent identity templates
- cross-agent shared identity contracts

## 13) Why this is P0

Without this layer, memory quality can improve while agent identity still drifts.
With this layer, Mnemo evolves from "memory retrieval" to "stable evolving cognition infrastructure".
