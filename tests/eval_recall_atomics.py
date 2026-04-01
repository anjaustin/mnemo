#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import time
import urllib.request

from eval_recall_quality import GOLD_FACTS, _get, _post


TARGETS = [
    ("alice", "Does Alice have any food allergies?", "shellfish"),
    ("bob", "What kind of dog does Bob have?", "Golden Retriever"),
    ("carol", "What is Carol's profession?", "cardiologist"),
    ("dave", "What is Dave's occupation?", "photographer"),
    ("dave", "Is Dave popular on social media?", "million"),
]


def query_context(base: str, user: str, query: str, timeout: float = 20.0) -> dict:
    return _post(
        base,
        f"/api/v1/memory/{user}/context",
        {"query": query, "max_tokens": 500, "min_relevance": 0.0},
        timeout=timeout,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Inspect Mnemo factual recall atomics")
    parser.add_argument("--server", default="http://127.0.0.1:8080")
    parser.add_argument("--ingest-wait", type=float, default=15.0)
    parser.add_argument("--startup-wait", type=float, default=20.0)
    args = parser.parse_args()

    base = args.server.rstrip("/")

    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            if _get(base, "/healthz", timeout=2).get("status") == "ok":
                break
        except Exception:
            pass
        time.sleep(1)
    else:
        print(json.dumps({"error": "server did not become healthy"}, indent=2))
        return 2

    time.sleep(args.startup_wait)

    user_ids: dict[str, str] = {}
    for fact in GOLD_FACTS:
        for attempt in range(5):
            try:
                body = _post(
                    base,
                    "/api/v1/memory",
                    {"user": fact.user, "text": fact.episode, "role": "user"},
                    timeout=30,
                )
                user_ids[fact.user] = body["user_id"]
                break
            except Exception:
                if attempt == 4:
                    raise
                time.sleep(2)

    time.sleep(args.ingest_wait)

    per_user = {}
    for user, user_id in user_ids.items():
        spans = _get(base, f"/api/v1/spans/user/{user_id}")
        entities = _get(base, f"/api/v1/users/{user_id}/entities")
        per_user[user] = {
            "span_count": spans.get("count", 0),
            "entity_count": entities.get("count", 0),
            "spans_sample": spans.get("spans", [])[:3],
            "entities_sample": entities.get("data", [])[:5],
        }

    probes = []
    for user, query, expected in TARGETS:
        body = query_context(base, user, query)
        context = body.get("context", "")
        probes.append(
            {
                "user": user,
                "query": query,
                "expected": expected,
                "hit": expected.lower() in context.lower(),
                "entities_returned": len(body.get("entities", [])),
                "facts_returned": len(body.get("facts", [])),
                "episodes_returned": len(body.get("episodes", [])),
                "context_excerpt": context[:500],
                "episodes": body.get("episodes", [])[:5],
            }
        )

    print(json.dumps({"per_user": per_user, "probes": probes}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
