# Thread HEAD Design Note

Date: 2026-03-02
Status: implemented-v1

## Why

Mnemo already has the right primitives (`user` -> `session` -> `episode`), but we do not expose an explicit "current thread state" pointer.

Using a Git-like `HEAD` concept can make temporal behavior easier to understand and easier to consume from SDKs.

## Current behavior (already in place)

- A session is the thread boundary.
- Episodes are ordered events/messages in that thread.
- Facts/edges carry temporal validity (`valid_at` / `invalid_at`) so current truth is inferable.
- Retrieval can be scoped to session or full user memory.

So we already have implicit HEAD behavior, but not a first-class API concept.

## v1 delivered

- Session HEAD metadata fields are live (`head_episode_id`, `head_updated_at`, `head_version`).
- Episode writes advance HEAD metadata.
- Memory context supports `mode=head|hybrid|historical`.
- `mode=head` returns `head` diagnostics in response payload.
- Python SDK supports head mode (`context_head(...)` and `mode="head"`).
- Falsification/integration coverage includes head-mode diagnostics and explicit session override behavior.

## Remaining gap

Developers cannot directly target "the current thread state" in a predictable, explicit way.

This causes ambiguity between:

- immediate conversational context (thread now)
- long-term memory context (user history)
- historical reconstruction (`as_of` style queries)

## Deterministic selection semantics

Thread HEAD is:

- latest stable state of a session
- anchored by latest episode pointer + optional rolled-up summary

When `mode=head` and no explicit session is provided, selection prefers:

1. newer `head_updated_at` (or `last_activity_at`/`updated_at` fallback)
2. higher `head_version`
3. stable tie-breaker by session ID

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

## Follow-ups (v1.1+)

1. Optional `head_summary` refresh strategy for long threads.
2. Add explicit no-session diagnostics (`head: null` reason code).
3. Add user-facing "changes since HEAD version" helper endpoint.
