# mnemo-client (TypeScript)

Production-grade TypeScript/JavaScript client for the [Mnemo](https://github.com/anjaustin/mnemo) memory API.

Covers: memory, knowledge graph, LLM span tracing, memory digest, governance, webhooks, operator, import, and session message endpoints.
Zero runtime dependencies. Works in Node.js, Deno, Bun, and modern browsers (fetch-based).
Drop-in LangChain.js and Vercel AI SDK integrations included.

## Install

```bash
npm install mnemo-client
```

## Quick start

```ts
import { MnemoClient } from 'mnemo-client';

const mnemo = new MnemoClient({ baseUrl: 'http://localhost:8080' });

// Store a memory
const result = await mnemo.add('alice', 'I love hiking in Colorado and skiing in Utah.');
console.log(result.session_id);

// Retrieve context
const ctx = await mnemo.context('alice', 'What does Alice enjoy outdoors?');
console.log(ctx.text);
console.log(ctx.token_count);
```

## Production client options

```ts
const mnemo = new MnemoClient({
  baseUrl: 'https://mnemo.example.com',
  apiKey: 'sk-...',          // sent as Authorization: Bearer <key>
  timeoutMs: 20_000,
  maxRetries: 3,
  retryBackoffMs: 500,
  requestId: 'req-abc123',   // default x-mnemo-request-id header
});
```

## Memory operations

```ts
// Full context with all options
const ctx = await mnemo.context('alice', 'What is Alice working on?', {
  maxTokens: 500,
  mode: 'hybrid',
  contract: 'support_safe',
  retrievalPolicy: 'precision',
  timeIntent: 'current',
  temporalWeight: 0.7,
});
// ctx: { text, token_count, entities, facts, episodes, latency_ms, ... }

// Changes since a timestamp
const changes = await mnemo.changesSince('alice', {
  from: '2024-11-01T00:00:00Z',
  to: '2024-12-01T00:00:00Z',
});

// Conflict radar
const conflicts = await mnemo.conflictRadar('alice');

// Causal recall
const chains = await mnemo.causalRecall('alice', 'Why did Alice change jobs?');

// Time-travel trace
const trace = await mnemo.timeTravelTrace('alice', 'What changed?', {
  from: '2024-10-01T00:00:00Z',
  to: '2024-12-01T00:00:00Z',
});

// Time-travel summary
const summary = await mnemo.timeTravelSummary('alice', 'preference changes', {
  from: '2024-10-01T00:00:00Z',
  to: '2024-12-01T00:00:00Z',
});
```

## Knowledge Graph API

```ts
// List entities
const entities = await mnemo.graphEntities('alice', { limit: 50 });
for (const e of entities.data) {
  console.log(e.name, e.entity_type, e.mention_count);
}

// Get entity with adjacency
const entity = await mnemo.graphEntity('alice', '<entity-uuid>');

// List edges
const edges = await mnemo.graphEdges('alice', { validOnly: true });

// BFS neighborhood
const neighbors = await mnemo.graphNeighbors('alice', '<entity-uuid>', { depth: 2 });

// Community detection
const communities = await mnemo.graphCommunity('alice');
console.log(`${communities.community_count} communities`);

// Shortest path between two entities
const path = await mnemo.graphShortestPath('alice', '<entity-a>', '<entity-b>', { maxDepth: 5 });
console.log(`Path length: ${path.path.length}, hops: ${path.hop_count}`);
```

## Memory Digest (sleep-time compute)

```ts
// Get or generate a memory digest
const digest = await mnemo.memoryDigest('alice');
console.log(digest.summary);
console.log('Topics:', digest.dominant_topics);

// Force LLM regeneration
const fresh = await mnemo.memoryDigest('alice', { refresh: true });
```

## LLM Span Tracing

```ts
// Look up all LLM calls for a request
const spans = await mnemo.spansByRequest('019cc15a-5470-7711-8d51-a3af1ace5522');
console.log(`${spans.count} spans, ${spans.total_tokens} tokens`);
for (const s of spans.spans) {
  console.log(s.operation, s.model, s.total_tokens, s.latency_ms + 'ms');
}

// Look up spans for a user
const userSpans = await mnemo.spansByUser('alice', { limit: 20 });
```

## Governance

```ts
// Get policy
const policy = await mnemo.getPolicy('alice');

// Set policy
const updated = await mnemo.setPolicy('alice', {
  retentionDaysMessage: 90,
  retentionDaysText: 365,
  webhookDomainAllowlist: ['example.com'],
  defaultMemoryContract: 'support_safe',
  defaultRetrievalPolicy: 'precision',
});

// Preview impact before applying
const preview = await mnemo.previewPolicy('alice', { retentionDaysMessage: 30 });

// Audit log
const audit = await mnemo.getPolicyAudit('alice', { limit: 50 });
const violations = await mnemo.getPolicyViolations('alice', {
  from: '2024-11-01T00:00:00Z',
  to: '2024-12-01T00:00:00Z',
});
```

## Webhooks

```ts
// Create
const wh = await mnemo.createWebhook({
  user: 'alice',
  targetUrl: 'https://hooks.example.com/mnemo',
  events: ['fact_added', 'fact_superseded'],
  signingSecret: 'my-secret',
});

// Inspect
const webhook = await mnemo.getWebhook(wh.id);
const events = await mnemo.getWebhookEvents(wh.id, { limit: 50 });
const deadLetters = await mnemo.getDeadLetterEvents(wh.id);
const stats = await mnemo.getWebhookStats(wh.id, { windowSeconds: 300 });

// Update
await mnemo.updateWebhook(wh.id, { enabled: false });

// Replay + retry
const replay = await mnemo.replayEvents(wh.id, { afterEventId: 'evt-abc', limit: 100 });
const retry = await mnemo.retryEvent(wh.id, 'evt-xyz');

// Audit
const whAudit = await mnemo.getWebhookAudit(wh.id, { limit: 20 });

// Delete
await mnemo.deleteWebhook(wh.id);
```

## Operator

```ts
// Ops summary (live metrics)
const ops = await mnemo.opsSummary({ windowSeconds: 300 });

// Cross-pipeline trace lookup
const trace = await mnemo.traceLookup('req-abc123', {
  from: '2024-11-01T00:00:00Z',
  to: '2024-12-01T00:00:00Z',
  limit: 100,
});
```

## Import

```ts
// Start an async chat history import job
const job = await mnemo.importChatHistory({
  user: 'alice',
  source: 'ndjson',
  payloadData: { /* ... */ },
  idempotencyKey: 'import-2024-11',
  dryRun: false,
});

// Poll status
const status = await mnemo.getImportJob(job.id);
```

## Session Messages

```ts
// Get messages
const msgs = await mnemo.getMessages(sessionId, { limit: 100 });

// Clear all messages
await mnemo.clearMessages(sessionId);

// Delete a message at index
await mnemo.deleteMessage(sessionId, 1);
```

## Health

```ts
const h = await mnemo.health();
console.log(h.status, h.version);
```

## LangChain.js integration

```ts
import { MnemoChatMessageHistory } from 'mnemo-client/langchain';

const history = new MnemoChatMessageHistory({
  baseUrl: 'http://localhost:8080',
  user: 'alice',
  sessionId: 'my-session',
});

// Use with RunnableWithMessageHistory, ConversationChain, etc.
const messages = await history.getMessages();
```

## Vercel AI SDK integration

```ts
import { generateText } from 'ai';
import { openai } from '@ai-sdk/openai';
import { mnemoRemember, mnemoRecall, mnemoDigest } from 'mnemo-client/vercel-ai';

const result = await generateText({
  model: openai('gpt-4o'),
  tools: {
    remember: mnemoRemember({ baseUrl: 'http://localhost:8080', user: 'alice' }),
    recall: mnemoRecall({ baseUrl: 'http://localhost:8080', user: 'alice' }),
    digest: mnemoDigest({ baseUrl: 'http://localhost:8080', user: 'alice' }),
  },
  prompt: 'Remember that I love hiking, then recall my hobbies.',
});
```

## Error handling

```ts
import { MnemoClient, MnemoNotFoundError, MnemoRateLimitError } from 'mnemo-client';

try {
  await mnemo.context('unknown_user', 'query');
} catch (err) {
  if (err instanceof MnemoNotFoundError) {
    console.log('User not found');
  } else if (err instanceof MnemoRateLimitError) {
    console.log('Rate limited, retry after', err.retryAfterMs, 'ms');
  }
}
```
