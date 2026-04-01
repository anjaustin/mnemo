#!/usr/bin/env python3
from __future__ import annotations

import argparse
import collections
import json
import time

from eval_recall_quality import GOLD_FACTS, _get, _post


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Inspect per-user ingest fairness over time"
    )
    parser.add_argument("--server", default="http://127.0.0.1:8080")
    parser.add_argument("--startup-wait", type=float, default=20.0)
    parser.add_argument("--duration", type=float, default=60.0)
    parser.add_argument("--poll-interval", type=float, default=5.0)
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

    episodes = []
    per_user_total = collections.Counter()
    for fact in GOLD_FACTS:
        body = _post(
            base,
            "/api/v1/memory",
            {"user": fact.user, "text": fact.episode, "role": "user"},
            timeout=30,
        )
        episodes.append((fact.user, body["episode_id"], fact.episode))
        per_user_total[fact.user] += 1

    started = time.time()
    snapshots = []
    while time.time() - started < args.duration:
        time.sleep(args.poll_interval)
        per_user = {u: collections.Counter() for u in per_user_total}
        for user, episode_id, _episode in episodes:
            ep = _get(base, f"/api/v1/episodes/{episode_id}")
            per_user[user][ep.get("processing_status", "unknown")] += 1
        snapshots.append(
            {
                "t": int(time.time() - started),
                "users": {
                    user: {
                        "completed": counts.get("completed", 0),
                        "processing": counts.get("processing", 0),
                        "pending": counts.get("pending", 0),
                    }
                    for user, counts in per_user.items()
                },
            }
        )
    print(json.dumps({"totals": per_user_total, "snapshots": snapshots}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
