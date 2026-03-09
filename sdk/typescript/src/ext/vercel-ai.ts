/**
 * Vercel AI SDK integration for Mnemo.
 *
 * Provides a `mnemoTool` for adding and retrieving memories
 * inside a Vercel AI SDK `generateText` / `streamText` pipeline.
 *
 * @example
 * ```ts
 * import { generateText } from 'ai';
 * import { openai } from '@ai-sdk/openai';
 * import { mnemoRemember, mnemoRecall } from 'mnemo-client/vercel-ai';
 *
 * const result = await generateText({
 *   model: openai('gpt-4o'),
 *   tools: {
 *     remember: mnemoRemember({ baseUrl: 'http://localhost:8080', user: 'alice' }),
 *     recall: mnemoRecall({ baseUrl: 'http://localhost:8080', user: 'alice' }),
 *   },
 *   prompt: 'Remember that I love hiking, then recall my hobbies.',
 * });
 * ```
 */

import { tool } from 'ai';
import { z } from 'zod';
import { MnemoClient } from '../client.js';
import type { MnemoClientOptions } from '../types.js';

export interface MnemoToolOptions extends MnemoClientOptions {
  /** Mnemo user identifier. */
  user: string;
}

/**
 * A Vercel AI SDK tool that stores a memory in Mnemo.
 */
export function mnemoRemember(options: MnemoToolOptions) {
  const client = new MnemoClient(options);
  const user = options.user;

  return tool({
    description: 'Store a fact or piece of information in long-term memory.',
    parameters: z.object({
      text: z.string().describe('The fact or information to remember'),
      role: z.enum(['user', 'assistant', 'system']).optional().default('user'),
    }),
    execute: async ({ text, role }) => {
      const result = await client.add(user, text, { role });
      return {
        stored: true,
        episode_id: result.episode_id,
        session_id: result.session_id,
      };
    },
  });
}

/**
 * A Vercel AI SDK tool that retrieves context from Mnemo.
 */
export function mnemoRecall(options: MnemoToolOptions) {
  const client = new MnemoClient(options);
  const user = options.user;

  return tool({
    description:
      'Retrieve relevant context and memories for a query from long-term memory.',
    parameters: z.object({
      query: z.string().describe('The query to search memories for'),
      limit: z.number().optional().default(10),
    }),
    execute: async ({ query, limit }) => {
      const ctx = await client.context(user, query, { limit });
      return {
        text: ctx.text,
        token_count: ctx.token_count,
        entities: ctx.entities,
        facts: ctx.facts,
        sources: ctx.sources,
      };
    },
  });
}

/**
 * A Vercel AI SDK tool that generates a memory digest for a user.
 */
export function mnemoDigest(options: MnemoToolOptions) {
  const client = new MnemoClient(options);
  const user = options.user;

  return tool({
    description:
      "Generate a prose summary of everything known about the user's memory graph.",
    parameters: z.object({
      refresh: z.boolean().optional().default(false),
    }),
    execute: async ({ refresh }) => {
      const digest = await client.memoryDigest(user, { refresh });
      return {
        summary: digest.summary,
        entity_count: digest.entity_count,
        edge_count: digest.edge_count,
        topics: digest.dominant_topics,
      };
    },
  });
}
