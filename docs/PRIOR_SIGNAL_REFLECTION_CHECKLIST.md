# Prior-Signal Reflection Checklist

Date: March 23, 2026
Status: Engineering checklist

Use this checklist when designing or reviewing retrieval, personalization, reranking, context assembly, or changes to how Mnemo decides what evidence to surface.

## Retrieval Design

- Does this feature increase blending across channels?
- If channels disagree, is that disagreement preserved, diagnosable, or silently fused away?
- Are we improving recall by reducing evidentiary clarity?
- Does the feature assume semantic relevance is always preferable to literal retrieval?
- Are superseded or stale facts more likely to be resurfaced?

## Personalization and TinyLoRA

- Does this change increase prior-shaped retrieval behavior?
- Can we observe how much personalization changed ranking, selection, or final context?
- Are we distinguishing helpful personalization from personalization drift?
- Are we introducing a new bias path without adding diagnostics?
- Does the change make it harder to compare raw and adapted behavior?

## Context Assembly

- If evidence sources conflict, is there a visible indication of the conflict?
- Does the context distinguish current facts, historical facts, model inferences, and directly retrieved records?
- Would a human reviewer know what to trust from the assembled block?
- Are we giving the model conflicting context with no instruction about how to treat it?
- Is the system optimizing for fluency when the use case needs caution?

## Product and Policy Surface

- Should this behavior be configurable by risk profile?
- Does the change deserve an operator-visible control or audit field?
- If Mnemo ever adds evidence-deference modes such as `guided` or `strict`, would this area need them?
- Are we making claims in docs or marketing that exceed what the implementation can guarantee?

## Evaluation and Feedback

- Do we have a way to test this under changed user state, stale memories, or superseded facts?
- Can we measure whether the new behavior improves correctness rather than just retrieval confidence?
- Are we collecting operator or user feedback on confusing disagreement cases?
- What real-world failure would convince us this area needs stronger evidence-first controls?

## Guardrails for Communication

- Avoid saying Mnemo has solved hallucination resistance.
- Avoid treating literal retrieval as automatic truth.
- Avoid describing raw embeddings as prior-free.
- Avoid framing TinyLoRA as either purely beneficial or purely harmful.
- Prefer defined language such as `less prior-shaped` or `evidence-bearing`, and define those terms when used.

## Use of This Checklist

This checklist is intended to shape engineering judgment over time. It should be revisited after live project usage, retrieval regressions, new evaluation work, or any future move toward channel-separated output or evidence-deference policies.
