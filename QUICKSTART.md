# Mnemo Quickstart (Python First)

Goal: get from zero to "it remembers" in under 10 minutes.

## 1) Start dependencies

```bash
docker compose up -d redis qdrant
```

## 2) Start Mnemo server

```bash
cargo run --bin mnemo-server
```

Keep this terminal open.

## 3) Install Python SDK from this repo

```bash
pip install -e sdk/python
```

## 4) Run this script

```python
from mnemo import Mnemo

m = Mnemo("http://localhost:8080")

m.add("kendra", "I love hiking in Colorado and my dog is named Bear")
m.add("kendra", "I just got back from camping near Breckenridge with Sarah")

ctx = m.context("kendra", "What are my recent trips and hobbies?")

print("--- Context ---")
print(ctx.text)
print("Latency (ms):", ctx.latency_ms)
```

If you want richer extraction, configure `MNEMO_LLM_API_KEY` and `MNEMO_EMBEDDING_API_KEY`.
