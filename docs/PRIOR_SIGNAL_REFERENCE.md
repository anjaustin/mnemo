# Prior-Signal Reference for Mnemo

Date: March 23, 2026
Status: Internal reference

This memo translates the ideas in `notes/MNEMO_PRIOR_SIGNAL_GAPS.md` into Mnemo terms. It is intended to help future contributors distinguish between what Mnemo already does, what the note proposes, and what would need validation before becoming roadmap work.

## Scope

This memo covers four questions:

- what Mnemo already implements that is relevant to prior-signal separation
- what the note proposes beyond current behavior
- what benefits the proposals could bring
- what tradeoffs, unknowns, and implementation risks remain

## Current Mnemo Reality

Mnemo already has several building blocks that make the note relevant enough to evaluate against Mnemo's current design.

### Existing strengths

- `identity_core`, `experience`, and EWC++ resemble a prior-holder pattern for agent memory.
- Context assembly already combines semantic retrieval, full-text retrieval, and graph traversal.
- Temporal diagnostics and routing diagnostics already exist in context responses.
- Stored-fact contradiction handling already exists in graph and ingest logic.
- Mnemo includes optional TinyLoRA personalization, which can shape retrieval behavior for `(user, agent)` pairs when enabled.

### Current limitations relative to the note

- retrieval channels are fused before response
- there is no channel-separated output
- there is no retrieval-time disagreement metric
- there is no evidence-deference policy
- there is no exposed LoRA divergence diagnostic
- there is no LoRA gate based on divergence or agreement

## What the Note Gets Right

The note usefully identifies a plausible class of failure: retrieval can become too shaped by historical relevance and thereby underweight present evidence when user reality changes.

This is especially important for:

- stale preference recovery
- changed state or superseded facts
- long-running user relationships
- agent-specific personalization drift

The note also usefully argues that fusion can discard disagreement signals before either the operator or the model can see them.

## What the Note Does Not Yet Establish

The note is persuasive, but several claims still need empirical support before becoming implementation commitments.

### Claims that need validation

- how often semantic-vs-literal disagreement correlates with actual bad answers
- whether LoRA rotation magnitude is a reliable proxy for retrieval risk
- whether `guided` deference improves answers without harming usability too much
- whether `strict` deference improves safety enough to justify recall loss
- whether channel-separated output materially improves operator understanding in real use

### Claims that need narrower wording

- literal retrieval should be described as less prior-shaped, not inherently authoritative
- raw embeddings should be described as less personalized, not neutral ground
- the proposals improve one hallucination class, not hallucination in general

## Working Interpretation

The healthiest way to use the note is as a design lens for evidence-first retrieval, not as a directive to immediately rebuild Mnemo around strict deference.

That lens suggests three priorities:

1. expose disagreement that is currently hidden
2. instrument personalization effects before trying to suppress them
3. add policy only after observability and real-world evaluation improve

## Recommended Posture

### Use it for future design review

When reviewing retrieval, reranking, personalization, or context assembly work, ask:

- does this increase prior-shaped behavior?
- does this suppress or reveal disagreement?
- does this improve relevance at the cost of evidentiary clarity?
- does this create a new operator-facing diagnostic need?

### Use it for evaluation design

Future evals should consider not just relevance and latency, but also:

- disagreement frequency
- disagreement correctness
- supersession recovery
- personalization drift under change
- operator clarity when channels diverge

### Use it for roadmap reflection

The note should influence long-term direction, but not force immediate architectural work absent real-world validation.

## Non-Goals

This memo does not claim:

- that Mnemo has already implemented the five-component architecture
- that TinyLoRA is a design flaw
- that full-text retrieval should always override semantic retrieval
- that software-level evidence control fully solves model hallucination

## Summary

Mnemo already has many of the ingredients needed to explore more evidence-first memory behavior. The note adds a sharper theory of one failure mode and suggests promising future controls. The right response today is disciplined reflection, better instrumentation, and careful evaluation rather than immediate architectural overhaul.
