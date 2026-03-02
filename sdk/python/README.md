# mnemo-client

Lightweight Python SDK for Mnemo's high-level memory API.

## Install (local repo)

```bash
pip install -e sdk/python
```

## Example

```python
from mnemo import Mnemo

m = Mnemo("http://localhost:8080")
m.add("kendra", "I love hiking in Colorado and my dog is named Bear")

ctx = m.context("kendra", "What are my hobbies?")
print(ctx.text)

# Prefer the current thread HEAD
head_ctx = m.context_head("kendra", "What am I working on right now?")
print(head_ctx.mode, head_ctx.head)
```

## Production client options

```python
from mnemo import Mnemo

m = Mnemo(
    "http://localhost:8080",
    timeout_s=15.0,
    max_retries=3,
    retry_backoff_s=0.5,
)
```

## Errors

- `MnemoHttpError` for non-2xx API responses
- `MnemoRateLimitError` for `429` with `retry_after_ms` when available
- `MnemoConnectionError` for network failures
- `MnemoTimeoutError` for request timeouts
