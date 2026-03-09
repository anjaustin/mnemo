# mnemo-client (TypeScript)

Production-grade TypeScript/JavaScript client for the [Mnemo](https://github.com/anomalyco/mnemo) memory API.

Covers: memory, knowledge graph, LLM span tracing, memory digest, governance, webhooks, and import endpoints.
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
