# Importing Chat History

Mnemo provides an async importer for migrating existing conversation history.

## Endpoints

- `POST /api/v1/import/chat-history`
- `GET /api/v1/import/jobs/:job_id`

See `docs/API.md` for full request/response schemas.

## Supported sources (v1)

- `ndjson`
- `chatgpt_export`
- `gemini_export`

Request body size limit is currently configured to `64 MiB`.

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

## Gemini export format

Importer expects an object with `chunkedPrompt.chunks` where each chunk includes:

- `role` (`user` or `model`)
- `text` or `parts[].text`

Behavior:

- `model` is normalized to `assistant`
- chunks with `isThought: true` are skipped

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

## Stress test with a real export zip

Use the importer stress harness with a ChatGPT export archive:

```bash
python3 eval/import_stress.py \
  --zip-path downloads/6957c8e02c797beeb082b42e1f53a0d4f97ed813369f7b25376485225dded6b4-2025-10-21-02-29-50-e815fa493cfa481c941b2165f06911b9.zip \
  --mode dry-run \
  --iterations 2 \
  --base-url http://localhost:8080
```

For full-write stress, switch to:

```bash
python3 eval/import_stress.py --mode import --iterations 1 --base-url http://localhost:8080
```
