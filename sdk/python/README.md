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
head_ctx = m.context("kendra", "What am I working on right now?", mode="head")
print(head_ctx.mode, head_ctx.head)
```
