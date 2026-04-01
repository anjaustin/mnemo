#!/usr/bin/env python3
from __future__ import annotations

import argparse
import collections
import json
import time

from eval_recall_quality import GOLD_FACTS, _get, _post


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Inspect Mnemo ingest progress for a batch"
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
    for fact in GOLD_FACTS:
        for attempt in range(5):
            try:
                body = _post(
                    base,
                    "/api/v1/memory",
                    {"user": fact.user, "text": fact.episode, "role": "user"},
                    timeout=30,
                )
                break
            except Exception:
                if attempt == 4:
                    raise
                time.sleep(2)
        episodes.append(
            {
                "user": fact.user,
                "query": fact.query,
                "expected": fact.expected,
                "episode": fact.episode,
                "episode_id": body["episode_id"],
            }
        )

    snapshots = []
    started = time.time()
    while time.time() - started < args.duration:
        time.sleep(args.poll_interval)
        counts = collections.Counter()
        failed = []
        completed_without_atoms = []
        for item in episodes:
            ep = _get(base, f"/api/v1/episodes/{item['episode_id']}")
            status = ep.get("processing_status", "unknown")
            counts[status] += 1
            if status == "failed":
                failed.append(
                    {
                        "episode_id": item["episode_id"],
                        "user": item["user"],
                        "episode": item["episode"],
                        "retry_count": ep.get("retry_count"),
                        "processing_error": ep.get("processing_error"),
                    }
                )
            if (
                status == "completed"
                and not ep.get("entity_ids")
                and not ep.get("edge_ids")
            ):
                completed_without_atoms.append(
                    {
                        "episode_id": item["episode_id"],
                        "user": item["user"],
                        "episode": item["episode"],
                    }
                )

        snapshots.append(
            {
                "t": int(time.time() - started),
                "counts": dict(counts),
                "failed": failed[:10],
                "completed_without_atoms": completed_without_atoms[:10],
            }
        )

        if counts.get("completed", 0) + counts.get("failed", 0) + counts.get(
            "skipped", 0
        ) == len(episodes):
            break

    final_episodes = []
    for item in episodes:
        ep = _get(base, f"/api/v1/episodes/{item['episode_id']}")
        final_episodes.append(
            {
                "user": item["user"],
                "query": item["query"],
                "expected": item["expected"],
                "episode": item["episode"],
                "episode_id": item["episode_id"],
                "processing_status": ep.get("processing_status"),
                "retry_count": ep.get("retry_count"),
                "processing_error": ep.get("processing_error"),
                "entity_count": len(ep.get("entity_ids") or []),
                "edge_count": len(ep.get("edge_ids") or []),
            }
        )

    print(
        json.dumps({"snapshots": snapshots, "final_episodes": final_episodes}, indent=2)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
