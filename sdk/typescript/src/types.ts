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

export interface AdjacencyEdge {
  id: string;
  source_entity_id: string;
  target_entity_id: string;
  label: string;
  fact: string;
  confidence: number;
  valid: boolean;
  valid_at: string;
  invalid_at?: string;
}

export interface GraphEntityDetail {
  id: string;
  name: string;
  entity_type: string;
  user_id: string;
  summary?: string;
  mention_count: number;
  community_id?: string;
  created_at: string;
  updated_at: string;
  outgoing_edges: AdjacencyEdge[];
  incoming_edges: AdjacencyEdge[];
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

// ─── Graph Shortest Path ───────────────────────────────────────────

export interface GraphPathStep {
  entity_id: string;
  entity_name: string;
  entity_type: string;
  depth: number;
  edge?: {
    id: string;
    source_entity_id: string;
    target_entity_id: string;
    label: string;
    fact: string;
    valid: boolean;
  };
}

export interface GraphPathResult {
  from: string;
  to: string;
  found: boolean;
  path_length: number;
  steps: GraphPathStep[];
  entities_visited: number;
  request_id?: string;
}

export interface GraphPathOptions {
  maxDepth?: number;
  validOnly?: boolean;
  requestId?: string;
}

// ─── Import Chat History ───────────────────────────────────────────

export interface ImportChatHistoryOptions {
  defaultSession?: string;
  dryRun?: boolean;
  idempotencyKey?: string;
  requestId?: string;
}

export interface ImportChatHistoryResult {
  ok: boolean;
  job_id: string;
  status: string;
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

// ─── Webhook Event Types ───────────────────────────────────────────

/**
 * All supported webhook event types.
 *
 * Use these constants when creating or updating webhook subscriptions
 * instead of raw strings to avoid typos.
 */
export const WebhookEventType = {
  FactAdded: 'fact_added',
  FactSuperseded: 'fact_superseded',
  HeadAdvanced: 'head_advanced',
  ConflictDetected: 'conflict_detected',
  RevalidationNeeded: 'revalidation_needed',
  ClarificationGenerated: 'clarification_generated',
  ClarificationResolved: 'clarification_resolved',
  NarrativeRefreshed: 'narrative_refreshed',
  PromotionProposed: 'promotion_proposed',
  PromotionApproved: 'promotion_approved',
  PromotionRejected: 'promotion_rejected',
  PromotionExpired: 'promotion_expired',
  PromotionConflictDetected: 'promotion_conflict_detected',
} as const;

export type WebhookEventTypeValue = typeof WebhookEventType[keyof typeof WebhookEventType];

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
  /**
   * Filter by content modality. Supported: 'text', 'image', 'audio', 'document'.
   * Empty array includes all modalities.
   */
  includeModalities?: Array<'text' | 'image' | 'audio' | 'document' | 'video'>;
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

// ─── Time Travel ───────────────────────────────────────────────────

export interface ChangesSinceOptions {
  fromDt: string;
  toDt: string;
  session?: string;
  requestId?: string;
}

export interface ChangesSinceResult {
  added_facts: Record<string, unknown>[];
  superseded_facts: Record<string, unknown>[];
  confidence_deltas: Record<string, unknown>[];
  head_changes: Record<string, unknown>[];
  added_episodes: Record<string, unknown>[];
  summary: string;
  from: string;
  to: string;
  request_id?: string;
}

export interface ConflictRadarResult {
  conflicts: Record<string, unknown>[];
  user_id: string;
  request_id?: string;
}

export interface CausalRecallResult {
  chains: Record<string, unknown>[];
  query: string;
  request_id?: string;
}

export interface TimeTravelTraceOptions {
  fromDt: string;
  toDt: string;
  session?: string;
  contract?: string;
  retrievalPolicy?: string;
  maxTokens?: number;
  minRelevance?: number;
  requestId?: string;
}

export interface TimeTravelTraceResult {
  snapshot_from: Record<string, unknown>;
  snapshot_to: Record<string, unknown>;
  gained_facts: Record<string, unknown>[];
  lost_facts: Record<string, unknown>[];
  gained_episodes: Record<string, unknown>[];
  lost_episodes: Record<string, unknown>[];
  timeline: Record<string, unknown>[];
  summary: string;
  from: string;
  to: string;
  request_id?: string;
}

export interface TimeTravelSummaryOptions {
  fromDt: string;
  toDt: string;
  session?: string;
  requestId?: string;
}

/** Server returns a flat object; the SDK passes it through as-is. */
export interface TimeTravelSummaryResult {
  [key: string]: unknown;
  request_id?: string;
}

// ─── Governance (extended) ─────────────────────────────────────────

export interface SetPolicyOptions {
  retentionDaysMessage?: number;
  retentionDaysText?: number;
  retentionDaysJson?: number;
  webhookDomainAllowlist?: string[];
  defaultMemoryContract?: string;
  defaultRetrievalPolicy?: string;
  requestId?: string;
}

export interface PolicyPreviewOptions {
  retentionDaysMessage?: number;
  retentionDaysText?: number;
  retentionDaysJson?: number;
  requestId?: string;
}

export interface PolicyPreviewResult {
  user_id: string;
  current_policy: Record<string, unknown>;
  preview_policy: Record<string, unknown>;
  estimated_affected_episodes_total: number;
  estimated_affected_message_episodes: number;
  estimated_affected_text_episodes: number;
  estimated_affected_json_episodes: number;
  confidence: string;
  request_id?: string;
}

export interface AuditRecord {
  id: string;
  event_type: string;
  user_id: string;
  details: Record<string, unknown>;
  created_at: string;
  request_id?: string;
}

// ─── Webhooks (extended) ───────────────────────────────────────────

export interface WebhookStats {
  webhook_id: string;
  total_events: number;
  delivered_events: number;
  pending_events: number;
  dead_letter_events: number;
  failed_events: number;
  recent_failures: number;
  circuit_open: boolean;
  circuit_open_until?: string;
  rate_limit_per_minute: number;
  request_id?: string;
}

export interface ReplayResult {
  webhook_id: string;
  count: number;
  next_after_event_id?: string;
  events: Record<string, unknown>[];
  request_id?: string;
}

export interface RetryResult {
  webhook_id: string;
  event_id: string;
  queued: boolean;
  reason: string;
  event?: Record<string, unknown>;
  request_id?: string;
}

// ─── Operator ──────────────────────────────────────────────────────

export interface OpsSummaryOptions {
  windowSeconds?: number;
  requestId?: string;
}

export interface OpsSummaryResult {
  window_seconds: number;
  http_requests_total: number;
  http_responses_2xx: number;
  http_responses_4xx: number;
  http_responses_5xx: number;
  policy_update_total: number;
  policy_violation_total: number;
  webhook_deliveries_success_total: number;
  webhook_deliveries_failure_total: number;
  webhook_dead_letter_total: number;
  active_webhooks: number;
  dead_letter_backlog: number;
  pending_webhook_events: number;
  governance_audit_events_in_window: number;
  webhook_audit_events_in_window: number;
  request_id?: string;
}

// ─── Trace Lookup ──────────────────────────────────────────────────

export interface TraceLookupOptions {
  fromDt?: string;
  toDt?: string;
  limit?: number;
  requestId?: string;
}

export interface TraceLookupResult {
  request_id: string;
  matched_episodes: Record<string, unknown>[];
  matched_webhook_events: Record<string, unknown>[];
  matched_webhook_audit: Record<string, unknown>[];
  matched_governance_audit: Record<string, unknown>[];
  summary: Record<string, unknown>;
}

// ─── Sessions ──────────────────────────────────────────────────────

export interface SessionInfo {
  id: string;
  user_id: string;
  name?: string;
  episode_count: number;
  created_at: string;
  updated_at: string;
  request_id?: string;
}

export interface SessionsResult {
  data: SessionInfo[];
  count: number;
  request_id?: string;
}

export interface ListSessionsOptions {
  limit?: number;
  requestId?: string;
}

export interface CreateSessionOptions {
  name?: string;
  requestId?: string;
}

// ─── Agent Identity ────────────────────────────────────────────────

export interface AgentIdentityResult {
  agent_id: string;
  version: number;
  /** Opaque JSON identity blob (mission, style, boundaries, etc.) */
  core: Record<string, unknown>;
  updated_at: string;
  request_id?: string;
}

export interface ExperienceEventResult {
  id: string;
  agent_id: string;
  user_id: string;
  session_id: string;
  category: string;
  signal: string;
  confidence: number;
  weight: number;
  decay_half_life_days: number;
  evidence_episode_ids: string[];
  created_at: string;
  request_id?: string;
}

export interface AgentIdentityAuditResult {
  id: string;
  agent_id: string;
  /** One of: created, updated, rolled_back */
  action: string;
  from_version?: number;
  to_version?: number;
  rollback_to_version?: number;
  reason?: string;
  created_at: string;
  request_id?: string;
}

export interface PromotionProposalResult {
  id: string;
  agent_id: string;
  proposal: string;
  candidate_core: Record<string, unknown>;
  reason: string;
  risk_level: string;
  /** One of: pending, approved, rejected */
  status: string;
  source_event_ids: string[];
  created_at: string;
  approved_at?: string;
  rejected_at?: string;
  request_id?: string;
}

export interface AgentContextResult {
  /** Flattened context block (entities, facts, episodes, token_count) */
  context: Record<string, unknown>;
  identity: AgentIdentityResult;
  identity_version: number;
  experience_events_used: number;
  experience_weight_sum: number;
  user_memory_items_used: number;
  attribution_guards: Record<string, boolean>;
  request_id?: string;
}

export interface AddExperienceOptions {
  userId: string;
  sessionId: string;
  category: string;
  signal: string;
  confidence?: number;
  weight?: number;
  decayHalfLifeDays?: number;
  evidenceEpisodeIds?: string[];
  requestId?: string;
}

export interface CreatePromotionOptions {
  proposal: string;
  candidateCore: Record<string, unknown>;
  reason: string;
  sourceEventIds: string[];
  riskLevel?: string;
  requestId?: string;
}

export interface AgentContextOptions {
  session?: string;
  maxTokens?: number;
  minRelevance?: number;
  mode?: string;
  requestId?: string;
}

export interface AgentListOptions {
  limit?: number;
  requestId?: string;
}

export interface RollbackOptions {
  reason?: string;
  requestId?: string;
}

// ─── Multi-modal attachments ───────────────────────────────────────

export type AttachmentType = 'image' | 'audio' | 'video' | 'document';
export type Modality = 'text' | 'image' | 'audio' | 'video' | 'document';

export interface Attachment {
  id: string;
  episode_id: string;
  user_id: string;
  attachment_type: AttachmentType;
  mime_type: string;
  filename?: string;
  size_bytes: number;
  storage_key: string;
  processing_status: 'pending' | 'processing' | 'completed' | 'failed';
  description?: string;
  transcript?: string;
  duration_secs?: number;
  created_at: string;
  updated_at: string;
  processed_at?: string;
  processor_model?: string;
  error?: string;
}

export interface AttachmentResult {
  attachment: Attachment;
  download_url?: string;
  thumbnail_url?: string;
  request_id?: string;
}

export interface AttachmentSource {
  attachment_id: string;
  episode_id: string;
  modality: Modality;
  mime_type: string;
  download_url?: string;
  thumbnail_url?: string;
  description?: string;
  relevance_score: number;
}

export interface UploadAttachmentOptions {
  requestId?: string;
}
