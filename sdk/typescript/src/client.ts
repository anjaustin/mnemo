/**
 * Mnemo TypeScript client.
 *
 * Covers: memory, knowledge graph, LLM span tracing, memory digest,
 * governance, webhooks, import, and session-message endpoints.
 *
 * Works in Node.js, Deno, Bun, and modern browsers (fetch-based, zero deps).
 *
 * @example
 * ```ts
 * import { MnemoClient } from 'mnemo-client';
 *
 * const mnemo = new MnemoClient({ baseUrl: 'http://localhost:8080', apiKey: 'sk-...' });
 *
 * await mnemo.add('alice', 'I love hiking in Colorado');
 * const ctx = await mnemo.context('alice', 'What does Alice enjoy?');
 * console.log(ctx.text);
 * ```
 */

import type {
  AddOptions,
  AuditRecord,
  CausalRecallResult,
  ChangesSinceOptions,
  ChangesSinceResult,
  ConflictRadarResult,
  ContextOptions,
  ContextResult,
  CreateSessionOptions,
  DeleteResult,
  GraphCommunityOptions,
  GraphCommunityResult,
  GraphEdgesOptions,
  GraphEdgesResult,
  GraphEntitiesOptions,
  GraphEntitiesResult,
  GraphNeighborsOptions,
  GraphNeighborsResult,
  HealthResult,
  ImportJobResult,
  ListSessionsOptions,
  MemoryDigestOptions,
  MemoryDigestResult,
  MnemoClientOptions,
  OpsSummaryOptions,
  OpsSummaryResult,
  PolicyPreviewOptions,
  PolicyPreviewResult,
  PolicyResult,
  RememberResult,
  ReplayResult,
  RetryResult,
  SessionInfo,
  SessionsResult,
  SetPolicyOptions,
  SpansOptions,
  SpansResult,
  TimeTravelSummaryOptions,
  TimeTravelSummaryResult,
  TimeTravelTraceOptions,
  TimeTravelTraceResult,
  TraceLookupOptions,
  TraceLookupResult,
  WebhookEvent,
  WebhookResult,
  WebhookStats,
} from './types.js';

export class MnemoError extends Error {
  constructor(
    message: string,
    public readonly statusCode?: number,
    public readonly code?: string,
  ) {
    super(message);
    this.name = 'MnemoError';
  }
}

export class MnemoNotFoundError extends MnemoError {
  constructor(message: string) {
    super(message, 404, 'not_found');
    this.name = 'MnemoNotFoundError';
  }
}

export class MnemoRateLimitError extends MnemoError {
  constructor(
    message: string,
    public readonly retryAfterMs?: number,
  ) {
    super(message, 429, 'rate_limited');
    this.name = 'MnemoRateLimitError';
  }
}

export class MnemoValidationError extends MnemoError {
  constructor(message: string) {
    super(message, 422, 'validation_error');
    this.name = 'MnemoValidationError';
  }
}

// ─── Transport layer ───────────────────────────────────────────────

interface RequestOptions {
  method: string;
  path: string;
  body?: unknown;
  requestId?: string;
}

export class MnemoClient {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly timeoutMs: number;
  private readonly maxRetries: number;
  private readonly retryBackoffMs: number;
  private readonly defaultRequestId?: string;

  constructor(options: MnemoClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? 'http://localhost:8080').replace(/\/$/, '');
    this.apiKey = options.apiKey;
    this.timeoutMs = options.timeoutMs ?? 20_000;
    this.maxRetries = options.maxRetries ?? 2;
    this.retryBackoffMs = options.retryBackoffMs ?? 400;
    this.defaultRequestId = options.requestId;
  }

  private async request<T>(opts: RequestOptions): Promise<[T, string | undefined]> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (this.apiKey) headers['Authorization'] = `Bearer ${this.apiKey}`;
    const rid = opts.requestId ?? this.defaultRequestId;
    if (rid) headers['x-mnemo-request-id'] = rid;

    let lastError: unknown;
    for (let attempt = 0; attempt <= this.maxRetries; attempt++) {
      if (attempt > 0) {
        await sleep(this.retryBackoffMs * Math.pow(2, attempt - 1));
      }
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), this.timeoutMs);
      try {
        const res = await fetch(`${this.baseUrl}${opts.path}`, {
          method: opts.method,
          headers,
          body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
          signal: controller.signal,
        });
        clearTimeout(timer);
        const responseRid = res.headers.get('x-mnemo-request-id') ?? rid;
        if (res.ok) {
          const json = (await res.json()) as T;
          return [json, responseRid ?? undefined];
        }
        let errBody: Record<string, unknown> = {};
        try {
          errBody = (await res.json()) as Record<string, unknown>;
        } catch {
          // ignore parse failures
        }
        const errMsg =
          (errBody['error'] as Record<string, unknown> | undefined)?.['message'] as string ??
          res.statusText;
        const code =
          (errBody['error'] as Record<string, unknown> | undefined)?.['code'] as string | undefined;
        if (res.status === 404) throw new MnemoNotFoundError(errMsg);
        if (res.status === 422) throw new MnemoValidationError(errMsg);
        if (res.status === 429) {
          const retryMs = (errBody['error'] as Record<string, unknown> | undefined)?.[
            'retry_after_ms'
          ] as number | undefined;
          throw new MnemoRateLimitError(errMsg, retryMs);
        }
        if (res.status >= 500 && attempt < this.maxRetries) {
          lastError = new MnemoError(errMsg, res.status, code);
          continue;
        }
        throw new MnemoError(errMsg, res.status, code);
      } catch (err) {
        clearTimeout(timer);
        if (err instanceof MnemoError) throw err;
        if (err instanceof Error && err.name === 'AbortError') {
          throw new MnemoError('Request timed out', undefined, 'timeout');
        }
        lastError = err;
        if (attempt >= this.maxRetries) throw err;
      }
    }
    throw lastError;
  }

  // ─── Memory API ─────────────────────────────────────────────────

  /** Check server health. */
  async health(requestId?: string): Promise<HealthResult> {
    const [body, rid] = await this.request<HealthResult>({
      method: 'GET',
      path: '/health',
      requestId,
    });
    return { ...body, request_id: rid };
  }

  /**
   * Store a memory for a user.
   *
   * @param user - Username or UUID.
   * @param text - The memory text.
   */
  async add(user: string, text: string, options: AddOptions = {}): Promise<RememberResult> {
    const [body, rid] = await this.request<RememberResult>({
      method: 'POST',
      path: '/api/v1/memory',
      body: {
        user,
        text,
        role: options.role ?? 'user',
        session: options.sessionId,
      },
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /**
   * Retrieve context for a user given a query.
   *
   * @param user - Username or UUID.
   * @param query - The query to retrieve context for.
   */
  async context(user: string, query: string, options: ContextOptions = {}): Promise<ContextResult> {
    const [body, rid] = await this.request<ContextResult>({
      method: 'POST',
      path: `/api/v1/memory/${encodeURIComponent(user)}/context`,
      body: {
        query,
        max_tokens: options.limit,
        session: options.sessionId,
        min_relevance: options.minScore,
        mode: options.mode,
        as_of: options.asOf,
        contract: options.contract,
        retrieval_policy: options.policy,
      },
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  // ─── Time Travel ─────────────────────────────────────────────────

  /** Get memory changes (added/superseded facts) between two timestamps. */
  async changesSince(
    user: string,
    options: ChangesSinceOptions,
  ): Promise<ChangesSinceResult> {
    const body: Record<string, unknown> = { from: options.fromDt, to: options.toDt };
    if (options.session) body['session'] = options.session;
    const [res, rid] = await this.request<ChangesSinceResult>({
      method: 'POST',
      path: `/api/v1/memory/${encodeURIComponent(user)}/changes_since`,
      body,
      requestId: options.requestId,
    });
    return { ...res, request_id: rid };
  }

  /** Detect conflicting facts in a user's memory. */
  async conflictRadar(user: string, requestId?: string): Promise<ConflictRadarResult> {
    const [body, rid] = await this.request<ConflictRadarResult>({
      method: 'POST',
      path: `/api/v1/memory/${encodeURIComponent(user)}/conflict_radar`,
      body: {},
      requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Retrieve causal reasoning chains for a query. */
  async causalRecall(
    user: string,
    query: string,
    requestId?: string,
  ): Promise<CausalRecallResult> {
    const [body, rid] = await this.request<CausalRecallResult>({
      method: 'POST',
      path: `/api/v1/memory/${encodeURIComponent(user)}/causal_recall`,
      body: { query },
      requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Diff memory snapshots over a time window. */
  async timeTravelTrace(
    user: string,
    query: string,
    options: TimeTravelTraceOptions,
  ): Promise<TimeTravelTraceResult> {
    const body: Record<string, unknown> = {
      query,
      from: options.fromDt,
      to: options.toDt,
    };
    if (options.session) body['session'] = options.session;
    if (options.contract) body['contract'] = options.contract;
    if (options.retrievalPolicy) body['retrieval_policy'] = options.retrievalPolicy;
    if (options.maxTokens !== undefined) body['max_tokens'] = options.maxTokens;
    if (options.minRelevance !== undefined) body['min_relevance'] = options.minRelevance;
    const [res, rid] = await this.request<TimeTravelTraceResult>({
      method: 'POST',
      path: `/api/v1/memory/${encodeURIComponent(user)}/time_travel/trace`,
      body,
      requestId: options.requestId,
    });
    return { ...res, request_id: rid };
  }

  /** Lightweight snapshot delta counts for fast rendering. */
  async timeTravelSummary(
    user: string,
    query: string,
    options: TimeTravelSummaryOptions,
  ): Promise<TimeTravelSummaryResult> {
    const body: Record<string, unknown> = {
      query,
      from: options.fromDt,
      to: options.toDt,
    };
    if (options.session) body['session'] = options.session;
    const [res, rid] = await this.request<TimeTravelSummaryResult>({
      method: 'POST',
      path: `/api/v1/memory/${encodeURIComponent(user)}/time_travel/summary`,
      body,
      requestId: options.requestId,
    });
    return { ...res, request_id: rid };
  }

  // ─── Knowledge Graph API ─────────────────────────────────────────

  /** List all entities in the user's knowledge graph. */
  async graphEntities(
    user: string,
    options: GraphEntitiesOptions = {},
  ): Promise<GraphEntitiesResult> {
    const limit = options.limit ?? 20;
    const [body, rid] = await this.request<GraphEntitiesResult>({
      method: 'GET',
      path: `/api/v1/graph/${encodeURIComponent(user)}/entities?limit=${limit}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Get a single entity by ID with adjacency information. */
  async graphEntity(
    user: string,
    entityId: string,
    requestId?: string,
  ): Promise<Record<string, unknown>> {
    const [body] = await this.request<Record<string, unknown>>({
      method: 'GET',
      path: `/api/v1/graph/${encodeURIComponent(user)}/entities/${entityId}`,
      requestId,
    });
    return body;
  }

  /** List edges in the user's knowledge graph. */
  async graphEdges(user: string, options: GraphEdgesOptions = {}): Promise<GraphEdgesResult> {
    const limit = options.limit ?? 20;
    const validOnly = options.validOnly ?? true;
    let path = `/api/v1/graph/${encodeURIComponent(user)}/edges?limit=${limit}&valid_only=${validOnly}`;
    if (options.label) path += `&label=${encodeURIComponent(options.label)}`;
    const [body, rid] = await this.request<GraphEdgesResult>({
      method: 'GET',
      path,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Return the BFS neighborhood around an entity. */
  async graphNeighbors(
    user: string,
    entityId: string,
    options: GraphNeighborsOptions = {},
  ): Promise<GraphNeighborsResult> {
    const depth = options.depth ?? 1;
    const maxNodes = options.maxNodes ?? 50;
    const validOnly = options.validOnly ?? true;
    const [body, rid] = await this.request<GraphNeighborsResult>({
      method: 'GET',
      path: `/api/v1/graph/${encodeURIComponent(user)}/neighbors/${entityId}?depth=${depth}&max_nodes=${maxNodes}&valid_only=${validOnly}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Run community detection on the user's knowledge graph. */
  async graphCommunity(
    user: string,
    options: GraphCommunityOptions = {},
  ): Promise<GraphCommunityResult> {
    const maxIterations = options.maxIterations ?? 20;
    const [body, rid] = await this.request<GraphCommunityResult>({
      method: 'GET',
      path: `/api/v1/graph/${encodeURIComponent(user)}/community?max_iterations=${maxIterations}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  // ─── LLM Span Tracing ────────────────────────────────────────────

  /** Return all LLM call spans for a given request ID. */
  async spansByRequest(requestIdToLookup: string, options: SpansOptions = {}): Promise<SpansResult> {
    const [body, rid] = await this.request<SpansResult>({
      method: 'GET',
      path: `/api/v1/spans/request/${encodeURIComponent(requestIdToLookup)}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Return recent LLM spans for a user (by UUID). */
  async spansByUser(userId: string, options: SpansOptions = {}): Promise<SpansResult> {
    const limit = options.limit ?? 100;
    const [body, rid] = await this.request<SpansResult>({
      method: 'GET',
      path: `/api/v1/spans/user/${encodeURIComponent(userId)}?limit=${limit}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  // ─── Memory Digest (sleep-time compute) ──────────────────────────

  /**
   * Get or generate a memory digest for a user.
   *
   * @param user - Username or UUID.
   * @param options.refresh - Force LLM regeneration (POST). Default: false (GET, generate if missing).
   */
  async memoryDigest(user: string, options: MemoryDigestOptions = {}): Promise<MemoryDigestResult> {
    const { refresh = false, requestId } = options;
    if (refresh) {
      const [body, rid] = await this.request<MemoryDigestResult>({
        method: 'POST',
        path: `/api/v1/memory/${encodeURIComponent(user)}/digest`,
        requestId,
      });
      return { ...body, request_id: rid };
    }
    try {
      const [body, rid] = await this.request<MemoryDigestResult>({
        method: 'GET',
        path: `/api/v1/memory/${encodeURIComponent(user)}/digest`,
        requestId,
      });
      return { ...body, request_id: rid };
    } catch (err) {
      if (err instanceof MnemoNotFoundError) {
        const [body, rid] = await this.request<MemoryDigestResult>({
          method: 'POST',
          path: `/api/v1/memory/${encodeURIComponent(user)}/digest`,
          requestId,
        });
        return { ...body, request_id: rid };
      }
      throw err;
    }
  }

  // ─── Governance / Policy ─────────────────────────────────────────

  /** Get policy for a user. */
  async getPolicy(user: string, requestId?: string): Promise<PolicyResult> {
    const [body, rid] = await this.request<{ policy: PolicyResult }>({
      method: 'GET',
      path: `/api/v1/policies/${encodeURIComponent(user)}`,
      requestId,
    });
    return { ...body.policy, request_id: rid };
  }

  /** Create or update a governance policy for a user. */
  async setPolicy(user: string, options: SetPolicyOptions = {}): Promise<PolicyResult> {
    const body: Record<string, unknown> = {};
    if (options.retentionDaysMessage !== undefined) body['retention_days_message'] = options.retentionDaysMessage;
    if (options.retentionDaysText !== undefined) body['retention_days_text'] = options.retentionDaysText;
    if (options.retentionDaysJson !== undefined) body['retention_days_json'] = options.retentionDaysJson;
    if (options.webhookDomainAllowlist) body['webhook_domain_allowlist'] = options.webhookDomainAllowlist;
    if (options.defaultMemoryContract) body['default_memory_contract'] = options.defaultMemoryContract;
    if (options.defaultRetrievalPolicy) body['default_retrieval_policy'] = options.defaultRetrievalPolicy;
    const [res, rid] = await this.request<{ policy: PolicyResult }>({
      method: 'PUT',
      path: `/api/v1/policies/${encodeURIComponent(user)}`,
      body,
      requestId: options.requestId,
    });
    return { ...res.policy, request_id: rid };
  }

  /** Preview the impact of a policy change without applying it. */
  async previewPolicy(user: string, options: PolicyPreviewOptions = {}): Promise<PolicyPreviewResult> {
    const body: Record<string, unknown> = {};
    if (options.retentionDaysMessage !== undefined) body['retention_days_message'] = options.retentionDaysMessage;
    if (options.retentionDaysText !== undefined) body['retention_days_text'] = options.retentionDaysText;
    if (options.retentionDaysJson !== undefined) body['retention_days_json'] = options.retentionDaysJson;
    const [res, rid] = await this.request<PolicyPreviewResult>({
      method: 'POST',
      path: `/api/v1/policies/${encodeURIComponent(user)}/preview`,
      body,
      requestId: options.requestId,
    });
    return { ...res, request_id: rid };
  }

  /** List governance audit events for a user's policy. */
  async getPolicyAudit(
    user: string,
    options: { limit?: number; requestId?: string } = {},
  ): Promise<AuditRecord[]> {
    const limit = options.limit ?? 50;
    const [body] = await this.request<{ audit: AuditRecord[] }>({
      method: 'GET',
      path: `/api/v1/policies/${encodeURIComponent(user)}/audit?limit=${limit}`,
      requestId: options.requestId,
    });
    return body.audit ?? [];
  }

  // ─── Webhooks ────────────────────────────────────────────────────

  /** Register a webhook subscription. */
  async createWebhook(
    user: string,
    targetUrl: string,
    events: string[],
    options?: { signingSecret?: string; requestId?: string },
  ): Promise<WebhookResult> {
    const [body, rid] = await this.request<{ webhook: WebhookResult }>({
      method: 'POST',
      path: '/api/v1/memory/webhooks',
      body: { user, target_url: targetUrl, events, signing_secret: options?.signingSecret },
      requestId: options?.requestId,
    });
    return { ...body.webhook, request_id: rid };
  }

  /** Get a webhook by ID. */
  async getWebhook(webhookId: string, requestId?: string): Promise<WebhookResult> {
    const [body, rid] = await this.request<WebhookResult>({
      method: 'GET',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}`,
      requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Update a webhook subscription (partial update). */
  async updateWebhook(
    webhookId: string,
    updates: {
      targetUrl?: string;
      events?: string[];
      enabled?: boolean;
      signingSecret?: string;
    },
    requestId?: string,
  ): Promise<WebhookResult> {
    const body: Record<string, unknown> = {};
    if (updates.targetUrl !== undefined) body.target_url = updates.targetUrl;
    if (updates.events !== undefined) body.events = updates.events;
    if (updates.enabled !== undefined) body.enabled = updates.enabled;
    if (updates.signingSecret !== undefined) body.signing_secret = updates.signingSecret;
    const [resp, rid] = await this.request<{ webhook: WebhookResult }>({
      method: 'PATCH',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}`,
      body,
      requestId,
    });
    return { ...resp.webhook, request_id: rid };
  }

  /** Delete a webhook. */
  async deleteWebhook(webhookId: string, requestId?: string): Promise<DeleteResult> {
    const [body, rid] = await this.request<DeleteResult>({
      method: 'DELETE',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}`,
      requestId,
    });
    return { deleted: body.deleted, request_id: rid };
  }

  /** List webhook events for a webhook. */
  async listWebhookEvents(
    webhookId: string,
    options: { limit?: number; requestId?: string } = {},
  ): Promise<{ events: WebhookEvent[]; count: number }> {
    const limit = options.limit ?? 20;
    const [body] = await this.request<{ events: WebhookEvent[]; count: number }>({
      method: 'GET',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}/events?limit=${limit}`,
      requestId: options.requestId,
    });
    return body;
  }

  /** List dead-letter events for a webhook. */
  async listDeadLetterEvents(
    webhookId: string,
    options: { limit?: number; requestId?: string } = {},
  ): Promise<{ events: WebhookEvent[]; count: number }> {
    const limit = options.limit ?? 20;
    const [body] = await this.request<{ events: WebhookEvent[]; count: number }>({
      method: 'GET',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}/events/dead-letter?limit=${limit}`,
      requestId: options.requestId,
    });
    return body;
  }

  /** Replay webhook events from a cursor. */
  async replayEvents(
    webhookId: string,
    options: {
      afterEventId?: string;
      limit?: number;
      includeDelivered?: boolean;
      includeDeadLetter?: boolean;
      requestId?: string;
    } = {},
  ): Promise<ReplayResult> {
    const limit = options.limit ?? 100;
    const includeDelivered = options.includeDelivered ?? true;
    const includeDeadLetter = options.includeDeadLetter ?? true;
    let path = `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}/events/replay?limit=${limit}&include_delivered=${includeDelivered}&include_dead_letter=${includeDeadLetter}`;
    if (options.afterEventId) path += `&after_event_id=${encodeURIComponent(options.afterEventId)}`;
    const [body, rid] = await this.request<ReplayResult>({
      method: 'GET',
      path,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Manually retry a failed webhook event. */
  async retryEvent(
    webhookId: string,
    eventId: string,
    options: { force?: boolean; requestId?: string } = {},
  ): Promise<RetryResult> {
    const [body, rid] = await this.request<RetryResult>({
      method: 'POST',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}/events/${encodeURIComponent(eventId)}/retry`,
      body: { force: options.force ?? false },
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Get delivery stats for a webhook. */
  async getWebhookStats(
    webhookId: string,
    options: { windowSeconds?: number; requestId?: string } = {},
  ): Promise<WebhookStats> {
    const windowSeconds = options.windowSeconds ?? 300;
    const [body, rid] = await this.request<WebhookStats>({
      method: 'GET',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}/stats?window_seconds=${windowSeconds}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /** List audit events for a webhook. */
  async getWebhookAudit(
    webhookId: string,
    options: { limit?: number; requestId?: string } = {},
  ): Promise<AuditRecord[]> {
    const limit = options.limit ?? 20;
    const [body] = await this.request<{ audit: AuditRecord[] }>({
      method: 'GET',
      path: `/api/v1/memory/webhooks/${encodeURIComponent(webhookId)}/audit?limit=${limit}`,
      requestId: options.requestId,
    });
    return body.audit ?? [];
  }

  // ─── Operator ────────────────────────────────────────────────────

  /** Get operator dashboard metrics summary. */
  async opsSummary(options: OpsSummaryOptions = {}): Promise<OpsSummaryResult> {
    const windowSeconds = options.windowSeconds ?? 300;
    const [body, rid] = await this.request<OpsSummaryResult>({
      method: 'GET',
      path: `/api/v1/ops/summary?window_seconds=${windowSeconds}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  // ─── Trace Lookup ────────────────────────────────────────────────

  /** Look up cross-pipeline trace by request correlation ID. */
  async traceLookup(
    requestIdToFind: string,
    options: TraceLookupOptions = {},
  ): Promise<TraceLookupResult> {
    const limit = options.limit ?? 100;
    let path = `/api/v1/traces/${encodeURIComponent(requestIdToFind)}?limit=${limit}`;
    if (options.fromDt) path += `&from=${encodeURIComponent(options.fromDt)}`;
    if (options.toDt) path += `&to=${encodeURIComponent(options.toDt)}`;
    const [body] = await this.request<TraceLookupResult>({
      method: 'GET',
      path,
      requestId: options.requestId,
    });
    return body;
  }

  // ─── Sessions ────────────────────────────────────────────────────

  /**
   * List sessions for a user.
   *
   * @param userId - The user's UUID (not a name — server requires a UUID path parameter).
   */
  async listSessions(
    userId: string,
    options: ListSessionsOptions = {},
  ): Promise<SessionsResult> {
    const limit = options.limit ?? 20;
    const [body, rid] = await this.request<SessionsResult>({
      method: 'GET',
      path: `/api/v1/users/${encodeURIComponent(userId)}/sessions?limit=${limit}`,
      requestId: options.requestId,
    });
    return { ...body, request_id: rid };
  }

  /**
   * Create a new session.
   *
   * @param userId - The user's UUID.
   */
  async createSession(
    userId: string,
    options: CreateSessionOptions = {},
  ): Promise<SessionInfo> {
    const body: Record<string, unknown> = { user_id: userId };
    if (options.name) body['name'] = options.name;
    const [res, rid] = await this.request<SessionInfo>({
      method: 'POST',
      path: '/api/v1/sessions',
      body,
      requestId: options.requestId,
    });
    return { ...res, request_id: rid };
  }

  /** Get a session by ID. */
  async getSession(sessionId: string, requestId?: string): Promise<SessionInfo> {
    const [body, rid] = await this.request<SessionInfo>({
      method: 'GET',
      path: `/api/v1/sessions/${encodeURIComponent(sessionId)}`,
      requestId,
    });
    return { ...body, request_id: rid };
  }

  /** Delete a session. */
  async deleteSession(sessionId: string, requestId?: string): Promise<DeleteResult> {
    const [body, rid] = await this.request<DeleteResult>({
      method: 'DELETE',
      path: `/api/v1/sessions/${encodeURIComponent(sessionId)}`,
      requestId,
    });
    return { deleted: body.deleted, request_id: rid };
  }

  // ─── Session Messages ─────────────────────────────────────────────

  /** Get messages for a session (chronological order). */
  async getMessages(
    sessionId: string,
    options: { limit?: number; requestId?: string } = {},
  ): Promise<{ messages: Array<{ role: string; content: string; [key: string]: unknown }>; count: number }> {
    const limit = options.limit ?? 100;
    const [body] = await this.request<{
      messages: Array<{ role: string; content: string; [key: string]: unknown }>;
      count: number;
    }>({
      method: 'GET',
      path: `/api/v1/sessions/${encodeURIComponent(sessionId)}/messages?limit=${limit}`,
      requestId: options.requestId,
    });
    return body;
  }

  /** Delete all messages for a session. */
  async clearMessages(sessionId: string, requestId?: string): Promise<void> {
    await this.request<unknown>({
      method: 'DELETE',
      path: `/api/v1/sessions/${encodeURIComponent(sessionId)}/messages`,
      requestId,
    });
  }

  /** Delete a specific message by index. */
  async deleteMessage(sessionId: string, index: number, requestId?: string): Promise<void> {
    await this.request<unknown>({
      method: 'DELETE',
      path: `/api/v1/sessions/${encodeURIComponent(sessionId)}/messages/${index}`,
      requestId,
    });
  }

  // ─── Import ──────────────────────────────────────────────────────

  /** Get status of an import job. */
  async getImportJob(jobId: string, requestId?: string): Promise<ImportJobResult> {
    const [body, rid] = await this.request<ImportJobResult>({
      method: 'GET',
      path: `/api/v1/import/jobs/${encodeURIComponent(jobId)}`,
      requestId,
    });
    return { ...body, request_id: rid };
  }
}

// ─── Helpers ───────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
