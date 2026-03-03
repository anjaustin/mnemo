# Importing Chat History

Mnemo provides an async importer for migrating existing conversation history.

## Endpoints

- `POST /api/v1/import/chat-history`
- `GET /api/v1/import/jobs/:job_id`

See `docs/API.md` for full request/response schemas.

## Supported sources (v1)

- `ndjson`
- `chatgpt_export`

## NDJSON format

Accepts one of:

- JSON array of message objects
- object with `messages` array
- newline-delimited JSON string

Per row fields:

- `role` (`user|assistant|system|tool`, required)
- `content` (required)
- `created_at` (optional, RFC3339 or unix-seconds string)
- `session` (optional)

## ChatGPT export format

Importer expects the standard conversation export shape with a `mapping` tree and message nodes. It extracts:

- author role from `message.author.role`
- text from `message.content.parts`
- timestamps from `message.create_time`
- session name from conversation `title`

## Idempotency and replay safety

Use `idempotency_key` to prevent duplicate imports on retries.

- same `user` + `idempotency_key` returns the original `job_id`
- duplicate request does not re-import episodes

## Dry run mode

Set `dry_run: true` to parse and validate payloads without writing episodes.

Useful for:

- schema validation
- row count verification
- pre-flight checks before large imports

## Failure behavior

- malformed payloads fail the job with parse errors
- row-level ingest issues are counted in `failed_messages`
- first 20 detailed errors are captured in `errors`

## Minimal example

```bash
curl -X POST http://localhost:8080/api/v1/import/chat-history \
  -H "Content-Type: application/json" \
  -d '{
    "user": "kendra",
    "source": "ndjson",
    "idempotency_key": "kendra-import-001",
    "default_session": "Imported History",
    "payload": [
      {"role":"user","content":"I switched to Nike.","created_at":"2025-02-01T10:00:00Z"},
      {"role":"assistant","content":"Noted.","created_at":"2025-02-01T10:00:05Z"}
    ]
  }'
```

Then poll:

```bash
curl http://localhost:8080/api/v1/import/jobs/JOB_ID
```
