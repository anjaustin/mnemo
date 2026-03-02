#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import statistics
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class EvalResult:
    profile: str
    total: int
    passed: int
    stale_failures: int
    latencies_ms: list[int]

    @property
    def accuracy(self) -> float:
        return (self.passed / self.total) if self.total else 0.0

    @property
    def stale_rate(self) -> float:
        return (self.stale_failures / self.total) if self.total else 0.0

    @property
    def p50_ms(self) -> int:
        if not self.latencies_ms:
            return 0
        return int(statistics.median(self.latencies_ms))

    @property
    def p95_ms(self) -> int:
        if not self.latencies_ms:
            return 0
        ordered = sorted(self.latencies_ms)
        idx = min(len(ordered) - 1, int(0.95 * (len(ordered) - 1)))
        return int(ordered[idx])


class Client:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")

    def req(
        self, path: str, method: str = "GET", data: dict[str, Any] | None = None
    ) -> tuple[int, dict[str, Any]]:
        body = None
        headers: dict[str, str] = {}
        if data is not None:
            body = json.dumps(data).encode("utf-8")
            headers["Content-Type"] = "application/json"

        request_obj = urllib.request.Request(
            f"{self.base_url}{path}", method=method, data=body, headers=headers
        )

        try:
            with urllib.request.urlopen(request_obj, timeout=30) as resp:
                payload = resp.read().decode("utf-8")
                return resp.status, (json.loads(payload) if payload else {})
        except urllib.error.HTTPError as exc:
            payload = exc.read().decode("utf-8")
            try:
                parsed = json.loads(payload)
            except json.JSONDecodeError:
                parsed = {"raw": payload}
            return exc.code, parsed


def run_profile(
    client: Client, cases: list[dict[str, Any]], profile: str
) -> EvalResult:
    total = len(cases)
    passed = 0
    stale_failures = 0
    latencies_ms: list[int] = []

    for case in cases:
        external_id = f"eval-{profile}-{uuid.uuid4().hex[:8]}"
        status, user = client.req(
            "/api/v1/users",
            "POST",
            {"name": external_id, "external_id": external_id, "metadata": {}},
        )
        if status != 201:
            continue
        user_id = user["id"]

        try:
            status, session = client.req(
                "/api/v1/sessions", "POST", {"user_id": user_id, "name": "default"}
            )
            if status != 201:
                continue
            session_id = session["id"]

            for memory in case.get("memories", []):
                client.req(
                    f"/api/v1/sessions/{session_id}/episodes",
                    "POST",
                    {
                        "type": "message",
                        "role": "user",
                        "content": memory["content"],
                        "created_at": memory["created_at"],
                    },
                )

            query = case["query"]
            payload: dict[str, Any] = {
                "query": query["text"],
                "session": "default",
                "max_tokens": 600,
            }

            if profile == "temporal":
                for key in ("mode", "time_intent", "as_of"):
                    if key in query:
                        payload[key] = query[key]
                payload["temporal_weight"] = 0.9

            started = time.time()
            status, context = client.req(
                f"/api/v1/memory/{external_id}/context", "POST", payload
            )
            elapsed_ms = int((time.time() - started) * 1000)
            latencies_ms.append(elapsed_ms)

            if status != 200:
                continue

            text = context.get("context", "")
            top_line = extract_top_context_line(text)
            expect = case.get("expect", {})
            contains = all(token in top_line for token in expect.get("contains", []))
            stale = any(token in top_line for token in expect.get("not_contains", []))
            if stale:
                stale_failures += 1
            if contains and not stale:
                passed += 1
        finally:
            client.req(f"/api/v1/users/{user_id}", "DELETE")

    return EvalResult(
        profile=profile,
        total=total,
        passed=passed,
        stale_failures=stale_failures,
        latencies_ms=latencies_ms,
    )


def print_markdown(results: list[EvalResult]) -> None:
    print(
        "| Profile | Accuracy | Stale Fact Rate | p50 Latency (ms) | p95 Latency (ms) |"
    )
    print("|---|---:|---:|---:|---:|")
    for result in results:
        print(
            f"| {result.profile} | {result.accuracy * 100:.1f}% | "
            f"{result.stale_rate * 100:.1f}% | {result.p50_ms} | {result.p95_ms} |"
        )


def extract_top_context_line(context_text: str) -> str:
    for line in context_text.splitlines():
        if line.startswith("- ["):
            return line
    return context_text


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run Mnemo temporal evaluation harness"
    )
    parser.add_argument("--base-url", default="http://localhost:8080")
    parser.add_argument(
        "--cases",
        default=str(Path(__file__).with_name("temporal_cases.json")),
    )
    args = parser.parse_args()

    with open(args.cases, "r", encoding="utf-8") as f:
        cases = json.load(f)

    client = Client(args.base_url)
    temporal = run_profile(client, cases, "temporal")
    baseline = run_profile(client, cases, "baseline")
    print_markdown([temporal, baseline])


if __name__ == "__main__":
    main()
