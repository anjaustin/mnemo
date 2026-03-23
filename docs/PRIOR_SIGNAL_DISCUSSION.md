# Prior-Signal Discussion and Strategic Reflection

Date: March 23, 2026

This document captures our discussion of `notes/MNEMO_PRIOR_SIGNAL_GAPS.md` and how its proposals relate to Mnemo today. It is not a product commitment, implementation plan, or claim that Mnemo has already solved hallucination resistance. It is a reference for future upgrades, evaluation, community communication, and engineering reflection.

## Why This Exists

The prior-signal document presents a serious critique of one class of failure in memory systems: retrieval shaped by prior expectations rather than current evidence. That critique is useful. It gives Mnemo a sharper vocabulary for understanding where a high-quality hybrid retrieval system can still fail under change, stale memory, or personalization drift.

At the same time, the document reaches beyond Mnemo's current implementation. Some of its proposals are strong candidates for future work. Others need experiments, narrower claims, or better alignment with the current codebase before they become roadmap items.

This document records that balance.

## The Core Distinction

Mnemo today is a strong hybrid memory and retrieval system. The prior-signal document asks whether Mnemo should also become an evidence-governance system.

That difference matters.

- A hybrid retrieval system tries to return the most relevant context.
- An evidence-governance system tries to detect when relevance is being distorted by prior expectations and then decide what should be trusted.

The prior-signal document is most valuable when read as a push toward better evidence governance, not as a claim that Mnemo is currently architected around that goal.

## What Mnemo Already Has

Several foundations described in the prior-signal document are already meaningfully present in Mnemo.

- Mnemo has a strong prior-holder concept through `identity_core`, `experience`, and EWC++ weighting for agent memory and stability.
- Mnemo already has a literal retrieval path through full-text retrieval, alongside semantic retrieval and graph traversal.
- Mnemo already has contradiction and conflict tooling for stored facts, plus temporal diagnostics in context responses.
- Mnemo already exposes useful retrieval diagnostics such as routing decisions, temporal resolution, and structured context output.

These matter because the document is not proposing a wholly foreign architecture. It is extending real strengths that Mnemo already has.

## Where the Document Identifies Real Value

The discussion surfaced several legitimate benefits in the document's proposals.

### 1. Make Hidden Failure Visible

Today Mnemo fuses semantic, full-text, and graph retrieval into a single response path. That is usually useful, but it can also hide disagreement between channels.

The document's strongest contribution is the claim that disagreement should sometimes be preserved rather than smoothed away. If semantic retrieval says one thing and literal retrieval says another, that conflict is itself a valuable signal.

This would improve:

- operator trust
- debugging clarity
- evaluation quality
- safety in stale-memory cases

### 2. Reframe Full-Text as an Evidence Anchor

Mnemo already has a literal retrieval channel. The document gives it a clearer conceptual role: a less prior-shaped evidence-bearing path.

That is useful even if the literal channel is not treated as perfect truth. It gives Mnemo a stronger theory of what that channel is for.

### 3. Add Honest Observability Around Personalization

TinyLoRA is currently framed as a retrieval-quality improvement. The document adds an important counter-question: when is personalization helping, and when is it pulling retrieval toward stale expectations?

Even if no gating is implemented, measuring raw-vs-adapted divergence would improve observability and help future evaluation.

### 4. Introduce Policy-Level Controls

The proposed `none`, `guided`, and `strict` deference modes are appealing because they map to different risk profiles.

- lower-risk conversational use can tolerate more blended context
- higher-risk uses may want more evidence-first behavior

That kind of policy surface is easier to explain than many lower-level retrieval internals.

### 5. Strengthen Mnemo's Long-Term Positioning

Many memory systems claim better recall or better retrieval. Fewer make explicit the boundary between prior-shaped relevance and evidence-bearing retrieval.

Whether or not Mnemo adopts these ideas directly, the document gives Mnemo a more distinctive lens for future development and public explanation.

## Where the Document Overstates or Compresses Reality

The discussion also surfaced several places where the document should be treated carefully.

### 1. Literal Retrieval Is Not the Same as Truth

The literal channel is less prior-shaped than adapted semantic retrieval, but it is not inherently authoritative.

Literal retrieval can still return:

- stale records
- superseded facts
- duplicate statements
- low-quality or noisy source material

If Mnemo uses literal retrieval as an evidence anchor, it still needs temporal and state-aware interpretation layered on top.

### 2. Disagreement Is Not the Same as Hallucination

Channel disagreement is an important signal, but it is not identical to hallucination.

- some hallucinations happen even when retrieval channels agree
- some channel disagreements are harmless or expected
- some high-quality answers rely on combining channels rather than separating them

The document is strongest when treated as targeting one failure class, not hallucination in general.

### 3. TinyLoRA Is Not the Only Source of Bias

The document focuses heavily on TinyLoRA. That is understandable, but incomplete.

Retrieval can also be shaped by:

- the base embedder
- indexing choices
- ingestion-time vectorization
- graph expansion logic
- reranking and fusion behavior
- the LLM's own trained priors

TinyLoRA may be an important amplifier, but it is not the entire story.

### 4. Strict Deference Is Not a Guarantee

Suppressing semantic context when disagreement is high is a meaningful software-level mitigation. It is not a proof that the model will obey evidence.

Even with a carefully controlled context window, the model can still:

- misread evidence
- overgeneralize from evidence
- fabricate beyond evidence

The document is correct to note that the deepest unsolved layer still lives inside the model.

### 5. Raw Embeddings Are Not Truly Neutral

The document sometimes risks making the raw query sound like pure evidence. It is only less personalized. It is still shaped by the base model and embedding system.

That does not invalidate the proposal. It just means the claim should be phrased as "less prior-shaped" rather than "prior-free."

## The Most Important Functional Gaps Between the Note and Mnemo Today

The note is not describing current Mnemo behavior. It is describing a future direction.

The most important gaps are:

- Mnemo does not currently return channel-separated retrieval output.
- Mnemo does not expose `retrieval_disagreement_score`.
- Mnemo does not expose `disagreement_details`.
- Mnemo does not support `deference_policy`.
- Mnemo does not expose `lora_rotation_magnitude` in context or retrieval responses.
- Mnemo does not gate TinyLoRA based on raw-vs-adapted divergence.
- Mnemo currently fuses channels before response, which can discard disagreement.

There is also a deeper architectural wrinkle: semantic vectors are adapted at ingest as well as retrieval, which means a truly neutral semantic lane does not exist today. That makes the document's proposal conceptually strong but more involved in practice.

## The Deepest Philosophical Gap

The clearest philosophical difference between Mnemo today and the document's vision is this:

- Mnemo assumes better retrieval blending usually improves answers.
- The document argues that in some important edge cases, blending is exactly what hides the problem.

That is the real divide.

Mnemo today is optimized around retrieval quality. The document asks Mnemo to sometimes privilege evidentiary clarity over blended relevance.

That is a legitimate shift in philosophy, but it should be adopted deliberately rather than accidentally.

## Why We Are Not Treating This as an Immediate Fix List

We do not currently have enough real-world usage data to justify a direct jump from note to implementation.

That restraint is appropriate for several reasons.

- The note identifies real risks, but they have not yet been quantified in Mnemo production use.
- Some proposed mechanisms sound clean conceptually but will require careful implementation and evaluation to avoid regressions.
- There are meaningful tradeoffs in recall, fluency, latency, API complexity, and operator interpretation.
- The current system already has strong foundations, so the right move is reflection and instrumentation before major architectural change.

In short: the note should influence Mnemo's development posture immediately, but not force immediate surgery on the retrieval stack.

## How This Should Influence Mnemo Going Forward

The discussion suggested a practical stance.

### 1. Use the Note as a Design Lens

Future work on retrieval, reranking, personalization, and context assembly should be reviewed through the lens of prior-shaping versus evidence visibility.

Questions worth asking:

- does this feature improve relevance by hiding disagreement?
- does this feature increase prior-shaped retrieval without exposing diagnostics?
- does this feature make evidence easier or harder to identify?

### 2. Prefer Instrumentation Before Control

Before introducing suppression or strict policies, Mnemo should first get better at observing:

- how often channels disagree
- when personalization meaningfully rotates retrieval behavior
- whether disagreement correlates with actual answer failures

This would make later policy choices more grounded.

### 3. Separate Insight from Product Promise

Mnemo can publicly say it is exploring evidence-first retrieval and disagreement-aware memory behavior.

Mnemo should not claim:

- that it has solved hallucination resistance
- that full-text is always authoritative
- that TinyLoRA is a defect rather than a tradeoff
- that the prior-signal architecture has already been fully implemented

### 4. Treat This as Institutional Memory

This discussion should remain available as future-facing context for roadmap decisions, research notes, community communication, and post-deployment reflection.

## Practical Near-Term Posture

The most credible near-term position is:

- preserve this critique as a strategic reference
- use it to inform future upgrades and evaluations
- communicate it carefully as exploration and reflection
- delay large architectural changes until instrumentation, testing, and live project usage provide sharper evidence

This is especially appropriate given the intent to test Mnemo in a real live project. That usage should inform whether these ideas become urgent, optional, or partially adopted.

## Summary

`notes/MNEMO_PRIOR_SIGNAL_GAPS.md` is valuable because it gives Mnemo a better theory of one important failure mode: retrieval that silently favors prior-shaped relevance over present evidence.

Its strongest contributions are:

- preserving disagreement rather than fusing it away
- giving full-text a clearer role as an evidence anchor
- making personalization more observable
- introducing the idea of evidence deference as a configurable policy

Its main limits are:

- literal retrieval is not truth by default
- disagreement is not the same as hallucination
- TinyLoRA is not the only source of prior influence
- software-level deference cannot fully override the model's internal priors

The right conclusion is not "fix Mnemo immediately." The right conclusion is that Mnemo now has a stronger conceptual framework for evaluating future retrieval, personalization, and context-assembly decisions.
