# Thread HEAD Design Note

Date: 2026-03-02
Status: proposal

## Why

Mnemo already has the right primitives (`user` -> `session` -> `episode`), but we do not expose an explicit "current thread state" pointer.

Using a Git-like `HEAD` concept can make temporal behavior easier to understand and easier to consume from SDKs.

## Current behavior (already in place)

- A session is the thread boundary.
- Episodes are ordered events/messages in that thread.
- Facts/edges carry temporal validity (`valid_at` / `invalid_at`) so current truth is inferable.
- Retrieval can be scoped to session or full user memory.

So we already have implicit HEAD behavior, but not a first-class API concept.

## Gap

Developers cannot directly target "the current thread state" in a predictable, explicit way.

This causes ambiguity between:

- immediate conversational context (thread now)
- long-term memory context (user history)
- historical reconstruction (`as_of` style queries)

## Proposal: first-class Thread HEAD

Define Thread HEAD as:

- latest stable state of a session
- anchored by latest episode pointer + optional rolled-up summary

### Session metadata additions

- `head_episode_id`
- `head_updated_at`
- optional `head_summary`
- optional `head_version` (monotonic increment)

### Write path behavior

On episode creation:

1. set `head_episode_id = new_episode.id`
2. set `head_updated_at = new_episode.created_at`
3. increment `head_version`
4. optionally refresh `head_summary` asynchronously

## Retrieval modes

Add explicit retrieval mode for memory context:

- `mode = head` (prefer current session state)
- `mode = hybrid` (default: head + long-term fusion)
- `mode = historical` (time-window / as-of behavior)

### Suggested request shape (non-breaking extension)

```json
{
  "query": "What am I working on right now?",
  "session": "default",
  "mode": "head"
}
```

### Suggested response diagnostics

```json
{
  "context": "...",
  "mode": "head",
  "head": {
    "episode_id": "...",
    "updated_at": "...",
    "version": 42
  }
}
```

## Relationship to temporal vectorization

Thread HEAD should drive temporal weighting defaults:

- `mode=head`: highest weight on session-local recency
- `mode=hybrid`: balanced session + user temporal relevance
- `mode=historical`: favor validity overlap over recency

This keeps temporal logic tied to existing Mnemo concepts instead of introducing abstract controls first.

## Rollout plan

1. Add session head metadata fields and update on episode writes.
2. Expose `mode` in memory context API (default `hybrid`).
3. Add response `head` diagnostics.
4. Update Python SDK with `mode` enum and helpers.
5. Add falsification tests for `mode=head` correctness.
