# Troubleshooting

Common issues and solutions when running Mnemo.

## Server won't start

| Symptom | Cause | Fix |
|---------|-------|-----|
| `Connection refused` on Redis | Redis is not running or wrong URL | Check `MNEMO_REDIS_URL` (default: `redis://localhost:6379`). Run `redis-cli ping`. |
| `Connection refused` on Qdrant | Qdrant is not running or wrong URL | Check `MNEMO_QDRANT_URL` (default: `http://localhost:6334`). Mnemo expects Qdrant on port `6334`; avoid HTTP health probes against that port because they can fail even when Qdrant is healthy. |
| `Address already in use` | Another process on port 8080 | Set `MNEMO_SERVER_PORT=8081` or stop the conflicting process. |
| `MNEMO_LLM_API_KEY not set` warning | No LLM key configured | Set `MNEMO_LLM_PROVIDER` and `MNEMO_LLM_API_KEY` for entity extraction and summaries. Mnemo still supports immediate recall without an LLM, but enrichment is disabled. |

## Memory operations

| Symptom | Cause | Fix |
|---------|-------|-----|
| `POST /api/v1/memory` returns 401 | Auth is enabled but no key provided | Pass `Authorization: Bearer <key>` or `x-api-key: <key>` header. Keys are set via `MNEMO_AUTH_API_KEYS`. |
| Memory added but context returns empty | Query is racing async ingest or enrichment is disabled | Immediate recall usually returns freshly written text right away. If you are waiting on extracted entities, graph edges, or summaries, confirm `MNEMO_LLM_PROVIDER` is not `none`, then wait a few seconds and retry. |
| Context results seem stale | Cached digest or stale embeddings | Force a digest refresh: `POST /api/v1/memory/:user/digest`. |
| `NotFound` error for user | User hasn't been created yet | Users are auto-created on first `POST /api/v1/memory`. Verify the user name matches exactly (case-sensitive). |

## Knowledge graph

| Symptom | Cause | Fix |
|---------|-------|-----|
| Graph endpoints return empty entities | LLM extraction is disabled or failed | Check `MNEMO_LLM_PROVIDER` is not `none`. Check LLM spans for errors: `GET /api/v1/spans/user/:user_id`. |
| Community detection returns 1 community | Not enough entities or edges | Community detection needs a minimum of `community_min_size` entities (default: 3) with cross-connections. |

## Webhooks

| Symptom | Cause | Fix |
|---------|-------|-----|
| Webhook events not delivered | Circuit breaker is open | Check webhook stats: `GET /api/v1/memory/webhooks/:id/stats`. Look at `circuit_open` field. Wait for `circuit_open_until` to pass. |
| Events landing in dead-letter | Target URL is failing | Check dead-letter queue: `GET /api/v1/memory/webhooks/:id/events/dead-letter`. Verify target URL is reachable and returns 2xx. |
| HMAC verification failing | Clock skew or wrong secret | Ensure you're using the `secret` returned from webhook creation. Verify timestamp tolerance in your verification logic. |
| Webhook PATCH rejected | TLS enforcement or domain allowlist | If the user has a policy with `webhook_domain_allowlist`, the webhook URL domain must be in the list. `require_tls: true` rejects non-HTTPS URLs. |

## Deployment

| Symptom | Cause | Fix |
|---------|-------|-----|
| Docker image won't pull | Wrong image reference | Use `ghcr.io/anjaustin/mnemo/mnemo-server:latest` or a specific version tag like `:0.4.0`. |
| Qdrant out of memory | Large collection with many vectors | Increase Qdrant memory or use `MNEMO_QDRANT_PREFIX` to namespace collections. Consider switching to on-disk storage mode. |
| Redis memory growing | Many webhook events or spans | Webhook events are capped at `max_events_per_webhook` (default: 1000). LLM spans have a 7-day TTL. Check `redis-cli info memory`. |
| Health check fails after deploy | Services not ready yet | Redis and Qdrant may take 10-30 seconds to start. Most deploy guides include a readiness wait. |

## SDKs

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ConnectionError` from Python SDK | Server not reachable | Verify `base_url` parameter. Try `curl http://localhost:8080/health` directly. |
| `MnemoRateLimitError` | Server-side rate limiting | Increase `MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE` or reduce request frequency. The SDK's `max_retries` will auto-retry with backoff. |
| Import errors for adapters | Missing optional dependencies | Install with extras: `pip install mnemo-client[langchain]` or `pip install mnemo-client[llamaindex]`. |

## Local embeddings (fastembed)

| Symptom | Cause | Fix |
|---------|-------|-----|
| First request is slow (30-60s) | Model downloading on first use | The `AllMiniLML6V2` model (~23 MB) downloads on first embedding call. Subsequent calls are fast. The model is cached in `.fastembed_cache/`. |
| `MNEMO_EMBEDDING_PROVIDER=local` not working | Fastembed feature not compiled in | The default Docker image includes fastembed. Building from source requires the `fastembed` Cargo feature. |

## Getting more help

1. Check the [API reference](API.md) for endpoint details
2. Check LLM spans for extraction errors: `GET /api/v1/spans/user/:user_id`
3. Use the operator dashboard at `http://localhost:8080/_/` for visual debugging
4. Review the [testing guide](TESTING.md) for running diagnostics
5. Open an issue at [github.com/anjaustin/mnemo](https://github.com/anjaustin/mnemo/issues)
