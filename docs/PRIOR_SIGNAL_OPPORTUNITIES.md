# Prior-Signal Strategic Opportunities

Date: March 23, 2026
Status: Strategic engineering reference

This document captures engineering opportunities suggested by the prior-signal discussion. It is not a commitment list. It is a structured set of options to revisit as Mnemo gains more real-world usage, evaluation data, and operator feedback.

## Framing

The prior-signal discussion does not justify immediate architectural overhaul. It does justify sharper attention to one class of failure: retrieval behavior that may over-favor prior-shaped relevance when present evidence has changed.

The most valuable opportunities are therefore staged, with instrumentation and evaluation ahead of stronger controls.

## Opportunity Tier 1: Instrumentation

These are the lowest-risk, highest-learning opportunities.

### Channel Visibility

Opportunity:

- preserve semantic, full-text, and graph outputs long enough to inspect them separately before fusion

Why it matters:

- reveals disagreement that is currently hidden
- improves debugging and operator trust
- creates a basis for future evaluation

What success would look like:

- internal diagnostics show channel overlap and divergence on real queries
- operators can inspect where a final context block came from

### Personalization Diagnostics

Opportunity:

- measure how much TinyLoRA changes query behavior when it is enabled

Why it matters:

- helps distinguish helpful personalization from drift
- makes future gating decisions evidence-based rather than theoretical

What success would look like:

- retrieval logs or diagnostics expose a usable measure of adaptation impact
- live-project usage shows whether large adaptation shifts correlate with confusing outputs

## Opportunity Tier 2: Evaluation

These opportunities make the prior-signal discussion testable.

### Disagreement-Aware Evals

Opportunity:

- add eval cases where semantic and literal channels may diverge due to changed user state, stale memories, or superseded facts

Why it matters:

- tests the exact failure mode raised in the note
- avoids optimizing only for static relevance benchmarks

What success would look like:

- Mnemo can measure whether disagreement predicts answer errors
- future changes can be judged on disagreement recovery, not just retrieval score

### Live Project Feedback Loop

Opportunity:

- use upcoming live project work as an early qualitative and quantitative testbed for these ideas

Why it matters:

- real usage will surface whether these concerns are occasional edge cases or recurring product issues
- live testing can reveal operator needs that theory misses

What success would look like:

- documented cases of stale-memory recovery, disagreement, and personalization drift
- a set of real examples that can seed future eval corpora

## Opportunity Tier 3: Operator and Product Controls

These should come after instrumentation begins to show clear value.

### Evidence-Annotated Context

Opportunity:

- make context blocks clearer about what is directly retrieved, historical, inferred, or superseded

Why it matters:

- improves human interpretability
- reduces ambiguity in high-stakes use

What success would look like:

- operators can tell why a context block should be trusted or treated cautiously

### Policy Modes for High-Risk Use

Opportunity:

- consider configurable context behavior for different risk profiles over time

Examples:

- conservative evidence-first assembly
- explicit disagreement annotation
- optional suppression of lower-authority context when conflicts are strong

Why it matters:

- different workloads need different tradeoffs between recall and caution

What success would look like:

- operator-facing controls map cleanly to real deployment needs
- policy choices are justified by evaluation rather than guesswork

## Opportunity Tier 4: Retrieval Architecture Changes

These are the highest-cost opportunities and should follow evidence, not precede it.

### LoRA Gating or Dampening

Opportunity:

- reduce personalization influence when adaptation appears to diverge too far from the unadapted query path

Why it matters:

- could limit retrieval over-attachment to historical relevance patterns

Risks:

- reduced recall
- degraded personalization quality
- false confidence if the gating signal is noisy

### More Explicit Channel Separation

Opportunity:

- return channel-separated retrieval data in addition to fused context

Why it matters:

- creates a cleaner foundation for disagreement analysis and operator inspection

Risks:

- API complexity
- SDK surface growth
- pressure to over-interpret raw channels without enough temporal or quality filtering

## Opportunity Tier 5: Research and Positioning

These are long-horizon opportunities that may shape how Mnemo presents itself.

### Evidence-First Memory Positioning

Opportunity:

- develop a clearer public story around evidence visibility, temporal correctness, and disagreement-aware retrieval

Why it matters:

- gives Mnemo a distinctive narrative beyond generic memory claims

Risk:

- overclaiming before capabilities are mature

### Model-Architecture Collaboration

Opportunity:

- use Mnemo as the retrieval and evidence-management layer for future work on more structurally separated model architectures

Why it matters:

- keeps Mnemo aligned with the deeper research question without pretending software alone solves it

## Suggested Order of Operations

A reasonable order would be:

1. preserve the discussion as reference material
2. gather live-project observations
3. add instrumentation and disagreement-aware evals
4. decide whether operator-visible controls are justified
5. only then consider stronger retrieval architecture changes

## Summary

The prior-signal discussion opens up real strategic opportunities for Mnemo. The best near-term opportunities are better diagnostics, better evaluation, and better reflection. Stronger control mechanisms should be earned by evidence from real usage rather than adopted because they sound elegant in theory.
