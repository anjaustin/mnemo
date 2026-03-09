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
  ContextOptions,
  ContextResult,
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
  MemoryDigestOptions,
  MemoryDigestResult,
  MnemoClientOptions,
  PolicyResult,
  RememberResult,
  SpansOptions,
  SpansResult,
  WebhookEvent,
  WebhookResult,
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
        metadata: options.metadata,
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

  // ─── Knowledge Graph API ─────────────────────────────────────────

  /** List all entities in the user's knowledge graph. */
  async graphEntities(
    user: string,
    options: GraphEntitiesOptions = {},
  ): Promise<GraphEntitiesResult> {
    const limit = options.limit ?? 100;
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
    const limit = options.limit ?? 100;
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
      path: `/api/v1/spans/user/${userId}?limit=${limit}`,
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

  /** List webhook events for a webhook. */
  async listWebhookEvents(
    webhookId: string,
    requestId?: string,
  ): Promise<{ events: WebhookEvent[]; count: number }> {
    const [body] = await this.request<{ events: WebhookEvent[]; count: number }>({
      method: 'GET',
      path: `/api/v1/memory/webhooks/${webhookId}/events`,
      requestId,
    });
    return body;
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
    const [body, rid] = await this.request<{ job: ImportJobResult }>({
      method: 'GET',
      path: `/api/v1/import/jobs/${jobId}`,
      requestId,
    });
    return { ...body.job, request_id: rid };
  }
}

// ─── Helpers ───────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
