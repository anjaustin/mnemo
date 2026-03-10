/**
 * Mnemo TypeScript SDK
 *
 * @example
 * ```ts
 * import { MnemoClient } from 'mnemo-client';
 *
 * const mnemo = new MnemoClient({ baseUrl: 'http://localhost:8080' });
 * await mnemo.add('alice', 'I love hiking in Colorado');
 * const ctx = await mnemo.context('alice', 'What does Alice enjoy?');
 * console.log(ctx.text);
 * ```
 */

export { MnemoClient } from './client.js';
export {
  MnemoError,
  MnemoNotFoundError,
  MnemoRateLimitError,
  MnemoValidationError,
} from './client.js';

export type {
  // Client options
  MnemoClientOptions,
  AddOptions,
  ContextOptions,
  GraphEntitiesOptions,
  GraphEdgesOptions,
  GraphNeighborsOptions,
  GraphCommunityOptions,
  SpansOptions,
  MemoryDigestOptions,
  // Results
  RememberResult,
  ContextResult,
  ContextBlock,
  HealthResult,
  DeleteResult,
  // Knowledge Graph
  GraphEntity,
  GraphEdge,
  GraphEntitiesResult,
  GraphEdgesResult,
  GraphNeighborNode,
  GraphNeighborEdge,
  GraphNeighborsResult,
  GraphCommunity,
  GraphCommunityResult,
  GraphPathStep,
  GraphPathResult,
  GraphPathOptions,
  // LLM Spans
  LlmSpan,
  SpansResult,
  // Memory Digest
  MemoryDigestResult,
  // Governance
  PolicyResult,
  SetPolicyOptions,
  PolicyPreviewOptions,
  PolicyPreviewResult,
  AuditRecord,
  // Webhooks
  WebhookResult,
  WebhookEvent,
  WebhookStats,
  ReplayResult,
  RetryResult,
  // Time Travel
  ChangesSinceOptions,
  ChangesSinceResult,
  ConflictRadarResult,
  CausalRecallResult,
  TimeTravelTraceOptions,
  TimeTravelTraceResult,
  TimeTravelSummaryOptions,
  TimeTravelSummaryResult,
  // Operator
  OpsSummaryOptions,
  OpsSummaryResult,
  // Trace Lookup
  TraceLookupOptions,
  TraceLookupResult,
  // Sessions
  SessionInfo,
  SessionsResult,
  ListSessionsOptions,
  CreateSessionOptions,
  // Import
  ImportJobResult,
  ImportChatHistoryOptions,
  ImportChatHistoryResult,
} from './types.js';
