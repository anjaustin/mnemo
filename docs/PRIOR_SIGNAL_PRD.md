# Prior-Signal Product Direction PRD

Status: draft
Owner: Retrieval / Platform
Priority: Strategic
Last updated: 2026-03-23

## 1) Executive Summary

Mnemo already has hybrid retrieval, temporal reasoning, and some retrieval and temporal diagnostics. The prior-signal discussion highlights a more specific opportunity: turn Mnemo's retrieval stack into a clearer, more evidence-aware system without pretending software alone solves hallucination.

The core product opportunity is not "fix hallucinations" in the abstract. It is to make Mnemo better at recognizing and exposing when historically shaped relevance may conflict with present evidence.

This PRD captures the strategic opportunities that follow from that framing.

## 2) Problem Statement

Today Mnemo assembles context from semantic, full-text, and graph retrieval. But the system currently optimizes for fused relevance, which means:

1. disagreement between retrieval channels can be hidden before operators or agents see it
2. personalization effects can shape retrieval without corresponding observability
3. Mnemo has limited ways to express evidentiary confidence beyond current diagnostics
4. our evaluation coverage is stronger on recall and temporal correctness than on disagreement recovery or stale-memory resistance

This is not a condemnation of current Mnemo. It is a logical next area if we want stronger retrieval observability and clearer evidence handling.

## 3) Product Goals

1. **Make disagreement visible.** Preserve and expose useful differences between semantic, literal, and graph retrieval behavior.
2. **Make personalization inspectable.** Add observability around TinyLoRA's effect when it is enabled.
3. **Improve operator trust.** Help users understand why context was assembled the way it was.
4. **Create evidence-first options.** Over time, support risk-aware context behaviors for use cases that need more caution.
5. **Upgrade the evaluation story.** Measure stale-memory recovery, disagreement handling, and evidence-vs-prior failure modes.

## 4) Non-Goals

- Claiming Mnemo has solved hallucination resistance
- Treating literal retrieval as automatic ground truth
- Immediately rebuilding the retrieval architecture around strict evidence-only behavior
- Replacing hybrid retrieval with a single preferred channel
- Committing to policy modes before instrumentation and real-world evidence justify them

## 5) Strategic Opportunities

### 5.1) Channel Visibility

Preserve semantic, full-text, and graph outputs long enough to inspect them separately before fusion.

Why it matters:

- reveals hidden disagreement
- improves debugging
- creates a foundation for future operator-facing diagnostics

Competitive edge:

- most systems talk about hybrid retrieval; fewer help users understand when hybrid signals conflict

### 5.2) Evidence-Annotated Context

Improve context assembly so it more clearly distinguishes directly retrieved records, current facts, historical facts, and inferred material.

Why it matters:

- makes context more interpretable to both operators and downstream agents
- may improve reviewer confidence in higher-risk workflows

Competitive edge:

- Mnemo could differentiate not just on retrieval quality, but on evidentiary clarity

### 5.3) TinyLoRA Diagnostics

Measure and expose how much personalization changes retrieval behavior when TinyLoRA is enabled.

Why it matters:

- helps distinguish beneficial personalization from drift
- creates a factual basis for later policy or gating decisions

Competitive edge:

- most personalization systems are opaque; observable personalization better supports enterprise review and debugging needs

### 5.4) Disagreement-Aware Evaluation

Add evaluation cases that test changed state, superseded facts, stale preferences, and channel divergence.

Why it matters:

- makes the prior-signal discussion falsifiable
- gives Mnemo more defensible claims than generic relevance metrics

Competitive edge:

- turns an abstract design idea into measurable product proof

### 5.5) Risk-Profile Context Policies

Potential follow-on area, not a committed roadmap item:

Over time, consider operator-selectable behaviors for more conservative context assembly in high-risk use cases.

Examples:

- stronger evidence annotation
- explicit disagreement surfacing
- optional suppression of lower-authority context when conflict is high

Why it matters:

- lets Mnemo serve both general-purpose and higher-caution workloads

Competitive edge:

- configurable evidence behavior is a stronger story than one-size-fits-all retrieval

## 6) Recommended Order

Recommended execution order:

1. channel visibility
2. TinyLoRA diagnostics
3. disagreement-aware evals
4. evidence-annotated context
5. risk-profile context policies

Rationale:

- start with observability before control
- make the problem measurable before making it configurable
- avoid committing to stronger policies before live usage shows they are worth the tradeoffs

## 7) Success Criteria

This initiative is directionally successful when:

- Mnemo can inspect channel behavior separately before fusion
- live usage and evals can identify disagreement-driven failures
- docs and operator materials can explain evidence visibility honestly and precisely
- future policy work, if any, is grounded in actual observations rather than design intuition alone

## 8) Risks

1. **Over-rotating toward caution** may hurt recall and fluency.
   - Mitigation: instrument first, gate later.

2. **Overclaiming in docs or positioning** may create credibility debt.
   - Mitigation: keep wording precise and capability-bound.

3. **Diagnostic overload** may confuse operators if new signals are not clearly defined.
   - Mitigation: ship small, interpretable diagnostics first.

4. **Premature architectural work** may burn time before real usage validates the need.
   - Mitigation: use the live project as a source of evidence before escalating scope.

## 9) Decision

The first concrete step should be channel visibility before fusion. It improves observability, evaluation quality, and future option value without forcing immediate user-facing behavioral changes.
