/**
 * Mnemo TypeScript SDK — type definitions
 */

// ─── Core results ──────────────────────────────────────────────────

export interface RememberResult {
  ok: boolean;
  user_id: string;
  session_id: string;
  episode_id: string;
  request_id?: string;
}

export interface ContextBlock {
  type: string;
  content: string;
  score?: number;
  source?: string;
  created_at?: string;
}

export interface ContextResult {
  text: string;
  token_count: number;
  entities: Record<string, unknown>[];
  facts: Record<string, unknown>[];
  episodes: Record<string, unknown>[];
  latency_ms: number;
  sources: string[];
  mode: string;
  head?: Record<string, unknown>;
  contract_applied?: string;
  retrieval_policy_applied?: string;
  temporal_diagnostics?: Record<string, unknown>;
  retrieval_policy_diagnostics?: Record<string, unknown>;
  request_id?: string;
}

export interface HealthResult {
  status: string;
  version: string;
  request_id?: string;
}

export interface DeleteResult {
  deleted: boolean;
  request_id?: string;
}

// ─── Knowledge Graph ───────────────────────────────────────────────

export interface GraphEntity {
  id: string;
  name: string;
  entity_type: string;
  summary?: string;
  mention_count: number;
  community_id?: string;
  created_at: string;
  updated_at: string;
}

export interface GraphEdge {
  id: string;
  source_entity_id: string;
  target_entity_id: string;
  label: string;
  fact: string;
  confidence: number;
  valid: boolean;
  valid_at: string;
  invalid_at?: string;
  created_at: string;
}

export interface GraphEntitiesResult {
  data: GraphEntity[];
  count: number;
  user_id: string;
  request_id?: string;
}

export interface GraphEdgesResult {
  data: GraphEdge[];
  count: number;
  user_id: string;
  request_id?: string;
}

export interface GraphNeighborNode {
  id: string;
  name: string;
  entity_type: string;
  summary?: string;
  depth: number;
}

export interface GraphNeighborEdge {
  id: string;
  source_entity_id: string;
  target_entity_id: string;
  label: string;
  fact: string;
  valid: boolean;
}

export interface GraphNeighborsResult {
  seed_entity_id: string;
  depth: number;
  nodes: GraphNeighborNode[];
  edges: GraphNeighborEdge[];
  entities_visited: number;
  request_id?: string;
}

export interface GraphCommunity {
  community_id: string;
  member_count: number;
  entity_ids: string[];
}

export interface GraphCommunityResult {
  user_id: string;
  total_entities: number;
  community_count: number;
  communities: GraphCommunity[];
  request_id?: string;
}

// ─── LLM Spans ─────────────────────────────────────────────────────

export interface LlmSpan {
  id: string;
  provider: string;
  model: string;
  operation: string;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  latency_ms: number;
  success: boolean;
  started_at: string;
  finished_at: string;
  request_id?: string;
  user_id?: string;
  error?: string;
}

export interface SpansResult {
  spans: LlmSpan[];
  count: number;
  total_tokens: number;
  total_latency_ms?: number;
  request_id?: string;
}

// ─── Memory Digest ─────────────────────────────────────────────────

export interface MemoryDigestResult {
  user_id: string;
  summary: string;
  entity_count: number;
  edge_count: number;
  dominant_topics: string[];
  generated_at: string;
  model: string;
  request_id?: string;
}

// ─── Governance / Policy ───────────────────────────────────────────

export interface PolicyResult {
  user_id: string;
  retention_days_message: number;
  retention_days_text: number;
  retention_days_json: number;
  webhook_domain_allowlist: string[];
  default_memory_contract: string;
  default_retrieval_policy: string;
  created_at: string;
  updated_at: string;
  request_id?: string;
}

// ─── Webhooks ──────────────────────────────────────────────────────

export interface WebhookResult {
  id: string;
  user_id: string;
  target_url: string;
  events: string[];
  enabled: boolean;
  created_at: string;
  updated_at: string;
  request_id?: string;
}

export interface WebhookEvent {
  id: string;
  webhook_id: string;
  event_type: string;
  user_id: string;
  payload: Record<string, unknown>;
  created_at: string;
  attempts: number;
  delivered: boolean;
  dead_letter: boolean;
  request_id?: string;
}

// ─── Import ────────────────────────────────────────────────────────

export interface ImportJobResult {
  id: string;
  source: string;
  user: string;
  dry_run: boolean;
  status: string;
  total_messages: number;
  imported_messages: number;
  failed_messages: number;
  sessions_touched: number;
  errors: string[];
  created_at: string;
  started_at?: string;
  finished_at?: string;
  request_id?: string;
}

// ─── Client options ────────────────────────────────────────────────

export interface MnemoClientOptions {
  /** Base URL of the Mnemo server. Default: http://localhost:8080 */
  baseUrl?: string;
  /** Bearer token for API key auth. */
  apiKey?: string;
  /** Request timeout in milliseconds. Default: 20_000 */
  timeoutMs?: number;
  /** Number of retries on transient errors. Default: 2 */
  maxRetries?: number;
  /** Base backoff in ms between retries. Default: 400 */
  retryBackoffMs?: number;
  /** Default x-mnemo-request-id header value. */
  requestId?: string;
}

export interface AddOptions {
  sessionId?: string;
  role?: 'user' | 'assistant' | 'system';
  metadata?: Record<string, unknown>;
  requestId?: string;
}

export interface ContextOptions {
  limit?: number;
  sessionId?: string;
  minScore?: number;
  includeEpisodes?: boolean;
  mode?: 'hybrid' | 'head' | 'historical';
  asOf?: string;
  contract?: string;
  policy?: string;
  requestId?: string;
}

export interface GraphEntitiesOptions {
  limit?: number;
  requestId?: string;
}

export interface GraphEdgesOptions {
  limit?: number;
  label?: string;
  validOnly?: boolean;
  requestId?: string;
}

export interface GraphNeighborsOptions {
  depth?: number;
  maxNodes?: number;
  validOnly?: boolean;
  requestId?: string;
}

export interface GraphCommunityOptions {
  maxIterations?: number;
  requestId?: string;
}

export interface SpansOptions {
  limit?: number;
  requestId?: string;
}

export interface MemoryDigestOptions {
  refresh?: boolean;
  requestId?: string;
}
