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
```
