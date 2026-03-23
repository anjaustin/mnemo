# Evidence-First Memory

Mnemo is a hybrid memory system. It combines semantic search, full-text search, and graph traversal to assemble useful context for agents.

That hybrid approach is powerful, but it raises an important question: what should happen when different retrieval paths disagree?

## Why This Matters

In long-running systems, past relevance is not always the same as present truth.

- a user changes preferences
- a fact is superseded
- a relationship changes over time
- personalization keeps favoring what used to be relevant

In those cases, a memory system can become too confident in its own historical patterns.

## Mnemo's Current Strength

Mnemo already treats time seriously. Facts can be time-bounded and later superseded rather than deleted. It also combines multiple retrieval strategies instead of depending on a single scoring path.

That gives Mnemo a useful foundation for more evidence-aware memory behavior.

## The Opportunity

One promising direction for future work is disagreement-aware retrieval.

That means asking questions like:

- when semantic and literal retrieval disagree, should that be visible?
- when personalization strongly shapes retrieval, should that be measured?
- should some applications prefer evidence-first context over blended context?

These are not solved problems. They are active design questions.

## What We Believe

In this area, Mnemo should aim over time to be:

- transparent about how context is assembled
- careful about stale or superseded information
- honest about the difference between relevance and evidence
- leave room for different risk profiles to be evaluated over time

## What We Do Not Claim

We do not claim that memory software alone can eliminate hallucination. Models still carry priors in their weights, and software-level retrieval controls do not erase that.

What memory infrastructure can help do is make evidence more visible, disagreement less silent, and operator choices more principled.

## Looking Ahead

Possible future areas of exploration include:

- richer retrieval diagnostics
- disagreement-aware evaluation
- evidence-first context assembly patterns
- clearer operator controls for high-risk use cases

That work will be guided by real-world usage, testing, and feedback rather than theory alone.
