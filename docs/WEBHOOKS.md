# Webhook Delivery Guide

Mnemo can emit memory lifecycle events to external systems through `POST /api/v1/memory/webhooks` subscriptions.

This guide describes what is delivered, how retries work, and how to verify request signatures.

## Event types

Supported event subscriptions:

- `head_advanced`
- `fact_added`
- `fact_superseded`
- `conflict_detected`

If `events` is omitted during registration, Mnemo subscribes the webhook to all event types.

## Delivery model

- Delivery is asynchronous after an event is recorded.
- Mnemo retries non-2xx responses with exponential backoff.
- Webhook subscriptions and event rows are persisted in Redis and restored at startup.
- Default runtime delivery settings:
  - `max_attempts=3`
  - `base_backoff_ms=200`
  - `request_timeout_ms=3000`
  - `rate_limit_per_minute=120`
  - `circuit_breaker_threshold=5`
  - `circuit_breaker_cooldown_ms=60000`
- Delivery telemetry is retained in-memory and queryable via:
  - `GET /api/v1/memory/webhooks/:id/events`
  - `GET /api/v1/memory/webhooks/:id/events/replay`
  - `POST /api/v1/memory/webhooks/:id/events/:event_id/retry`
  - `GET /api/v1/memory/webhooks/:id/events/dead-letter`
  - `GET /api/v1/memory/webhooks/:id/stats`
  - `GET /api/v1/memory/webhooks/:id/audit`

Each event record includes:

- `attempts`
- `delivered`
- `dead_letter`
- `delivered_at` (if successful)
- `last_error` (if most recent attempt failed)

## Delivery guarantees

- Delivery is at-least-once.
- A failed event is marked `dead_letter=true` after `max_attempts`.
- Per-webhook rate limiting protects downstream endpoints.
- Circuit breaker opens after repeated failures and pauses sends during cooldown.

## Replay and operator controls

- Replay with cursor semantics using `after_event_id`.
- Filter replay stream by delivered/dead-letter flags.
- Manually retry a specific event (`.../retry`) when downstream recovers.
- Audit endpoint records operator and delivery lifecycle actions for investigations.

## Outbound headers

Mnemo includes these headers on outbound webhook requests:

- `content-type: application/json`
- `x-mnemo-event-id`
- `x-mnemo-delivery-id`
- `x-mnemo-event-type`
- `x-mnemo-timestamp`
- `x-mnemo-request-id` (when source request had correlation id)
- `x-mnemo-signature` (only when `signing_secret` is configured)

Signature format:

- `x-mnemo-signature: t=<unix_timestamp>,v1=<hex_hmac_sha256>`
- Signed payload string: `"<timestamp>.<raw_body>"`

## Verify signature (Python)

```python
import hashlib
import hmac


def verify_mnemo_signature(raw_body: bytes, timestamp: str, signature_header: str, secret: str) -> bool:
    parts = dict(item.split("=", 1) for item in signature_header.split(",") if "=" in item)
    provided = parts.get("v1", "")
    signed = f"{timestamp}.{raw_body.decode('utf-8')}".encode("utf-8")
    expected = hmac.new(secret.encode("utf-8"), signed, hashlib.sha256).hexdigest()
    return hmac.compare_digest(provided, expected)
```

## Verify signature (Node.js)

```javascript
import crypto from "node:crypto";

export function verifyMnemoSignature(rawBody, timestamp, signatureHeader, secret) {
  const pairs = Object.fromEntries(
    signatureHeader
      .split(",")
      .map((part) => part.trim().split("="))
      .filter((kv) => kv.length === 2)
  );
  const provided = pairs.v1 || "";
  const payload = `${timestamp}.${rawBody}`;
  const expected = crypto.createHmac("sha256", secret).update(payload, "utf8").digest("hex");
  if (provided.length !== expected.length) {
    return false;
  }
  return crypto.timingSafeEqual(Buffer.from(provided), Buffer.from(expected));
}
```

## Security recommendations

- Use HTTPS `target_url` endpoints.
- Store `signing_secret` in a secret manager.
- Reject webhook calls with stale `x-mnemo-timestamp` values (for example, older than 5 minutes).
- Verify signature before processing payload.
- Treat event delivery as at-least-once; handlers should be idempotent using `x-mnemo-event-id`.
