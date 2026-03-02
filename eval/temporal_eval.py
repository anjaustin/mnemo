#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import statistics
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class EvalResult:
    system: str
    profile: str
    total: int
    passed: int
    stale_failures: int
    errors: int
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


class HttpClient:
    def __init__(self, base_url: str, headers: dict[str, str] | None = None):
        self.base_url = base_url.rstrip("/")
        self.headers = headers or {}

    def req(
        self,
        path: str,
        method: str = "GET",
        data: dict[str, Any] | None = None,
        query: dict[str, Any] | None = None,
    ) -> tuple[int, dict[str, Any]]:
        body = None
        headers = dict(self.headers)
        if data is not None:
            body = json.dumps(data).encode("utf-8")
            headers["Content-Type"] = "application/json"

        url = f"{self.base_url}{path}"
        if query:
            url = f"{url}?{urllib.parse.urlencode(query)}"

        request_obj = urllib.request.Request(
            url, method=method, data=body, headers=headers
        )

        try:
            with urllib.request.urlopen(request_obj, timeout=45) as resp:
                payload = resp.read().decode("utf-8")
                return resp.status, (json.loads(payload) if payload else {})
        except urllib.error.HTTPError as exc:
            payload = exc.read().decode("utf-8")
            try:
                parsed = json.loads(payload)
            except json.JSONDecodeError:
                parsed = {"raw": payload}
            return exc.code, parsed


class Backend:
    name: str

    def remember(
        self, user_id: str, session_id: str, content: str, created_at: str
    ) -> bool:
        raise NotImplementedError

    def retrieve(
        self, user_id: str, session_id: str, query: dict[str, Any], profile: str
    ) -> tuple[int, str, int]:
        raise NotImplementedError

    def create_user_session(self, external_id: str) -> tuple[str, str]:
        raise NotImplementedError

    def cleanup(self, user_id: str, session_id: str) -> None:
        raise NotImplementedError


class MnemoBackend(Backend):
    name = "mnemo"

    def __init__(self, base_url: str):
        self.http = HttpClient(base_url)

    def create_user_session(self, external_id: str) -> tuple[str, str]:
        status, user = self.http.req(
            "/api/v1/users",
            "POST",
            {"name": external_id, "external_id": external_id, "metadata": {}},
        )
        if status != 201:
            raise RuntimeError(f"mnemo user create failed: {status} {user}")
        user_id = user["id"]

        status, session = self.http.req(
            "/api/v1/sessions",
            "POST",
            {"user_id": user_id, "name": "default"},
        )
        if status != 201:
            raise RuntimeError(f"mnemo session create failed: {status} {session}")
        return user_id, session["id"]

    def remember(
        self, user_id: str, session_id: str, content: str, created_at: str
    ) -> bool:
        status, _ = self.http.req(
            f"/api/v1/sessions/{session_id}/episodes",
            "POST",
            {
                "type": "message",
                "role": "user",
                "content": content,
                "created_at": created_at,
            },
        )
        return status == 201

    def retrieve(
        self, user_id: str, session_id: str, query: dict[str, Any], profile: str
    ) -> tuple[int, str, int]:
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
        status, response = self.http.req(
            f"/api/v1/memory/{user_id}/context", "POST", payload
        )
        elapsed_ms = int((time.time() - started) * 1000)
        if status != 200:
            return status, "", elapsed_ms
        return status, response.get("context", ""), elapsed_ms

    def cleanup(self, user_id: str, session_id: str) -> None:
        self.http.req(f"/api/v1/users/{user_id}", "DELETE")


class ZepBackend(Backend):
    name = "zep"

    def __init__(self, base_url: str, api_key: str):
        self.http = HttpClient(
            base_url,
            headers={
                "Authorization": f"Bearer {api_key}",
                "x-api-key": api_key,
            },
        )

    def create_user_session(self, external_id: str) -> tuple[str, str]:
        status, user = self.http.req(
            "/users",
            "POST",
            {
                "user_id": external_id,
                "first_name": external_id,
                "last_name": "Eval",
            },
        )
        if status not in (200, 201):
            raise RuntimeError(f"zep user create failed: {status} {user}")

        session_id = f"{external_id}-session"
        status, session = self.http.req(
            "/sessions",
            "POST",
            {
                "session_id": session_id,
                "user_id": external_id,
            },
        )
        if status not in (200, 201):
            raise RuntimeError(f"zep session create failed: {status} {session}")
        return external_id, session_id

    def remember(
        self, user_id: str, session_id: str, content: str, created_at: str
    ) -> bool:
        status, _ = self.http.req(
            f"/sessions/{session_id}/memory",
            "POST",
            {
                "messages": [
                    {
                        "role": user_id,
                        "role_type": "user",
                        "content": content,
                        "created_at": created_at,
                    }
                ],
                "return_context": False,
            },
        )
        return status in (200, 201)

    def retrieve(
        self, user_id: str, session_id: str, query: dict[str, Any], profile: str
    ) -> tuple[int, str, int]:
        # Zep Memory API derives relevance from latest session messages.
        # We append the query text as a fresh user message, then fetch memory context.
        self.http.req(
            f"/sessions/{session_id}/memory",
            "POST",
            {
                "messages": [
                    {
                        "role": user_id,
                        "role_type": "user",
                        "content": query["text"],
                    }
                ],
                "return_context": False,
            },
        )

        started = time.time()
        status, response = self.http.req(f"/sessions/{session_id}/memory", "GET")
        elapsed_ms = int((time.time() - started) * 1000)
        if status != 200:
            return status, "", elapsed_ms
        return status, response.get("context", ""), elapsed_ms

    def cleanup(self, user_id: str, session_id: str) -> None:
        self.http.req(f"/sessions/{session_id}/memory", "DELETE")
        self.http.req(f"/users/{user_id}", "DELETE")


def run_profile(
    backend: Backend, cases: list[dict[str, Any]], profile: str
) -> EvalResult:
    total = len(cases)
    passed = 0
    stale_failures = 0
    errors = 0
    latencies_ms: list[int] = []

    for case in cases:
        external_id = f"eval-{backend.name}-{profile}-{uuid.uuid4().hex[:8]}"
        user_id = ""
        session_id = ""

        try:
            user_id, session_id = backend.create_user_session(external_id)

            memories_ok = True
            for memory in case.get("memories", []):
                ok = backend.remember(
                    user_id, session_id, memory["content"], memory["created_at"]
                )
                memories_ok = memories_ok and ok

            if not memories_ok:
                errors += 1
                continue

            status, context_text, latency_ms = backend.retrieve(
                user_id, session_id, case["query"], profile
            )
            latencies_ms.append(latency_ms)
            if status != 200:
                errors += 1
                continue

            top_line = extract_top_context_line(context_text)
            expect = case.get("expect", {})
            contains = all(token in top_line for token in expect.get("contains", []))
            stale = any(token in top_line for token in expect.get("not_contains", []))

            if stale:
                stale_failures += 1
            if contains and not stale:
                passed += 1
        except Exception:
            errors += 1
        finally:
            if user_id and session_id:
                backend.cleanup(user_id, session_id)

    return EvalResult(
        system=backend.name,
        profile=profile,
        total=total,
        passed=passed,
        stale_failures=stale_failures,
        errors=errors,
        latencies_ms=latencies_ms,
    )


def extract_top_context_line(context_text: str) -> str:
    for line in context_text.splitlines():
        if line.startswith("- ["):
            return line
    return context_text


def print_markdown(results: list[EvalResult]) -> None:
    print(
        "| System | Profile | Accuracy | Stale Fact Rate | Errors | p50 Latency (ms) | p95 Latency (ms) |"
    )
    print("|---|---|---:|---:|---:|---:|---:|")
    for result in results:
        print(
            f"| {result.system} | {result.profile} | {result.accuracy * 100:.1f}% | "
            f"{result.stale_rate * 100:.1f}% | {result.errors} | {result.p50_ms} | {result.p95_ms} |"
        )


def load_key(path: str) -> str:
    with open(path, "r", encoding="utf-8") as f:
        return f.read().strip()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run Mnemo/Zep temporal evaluation harness"
    )
    parser.add_argument(
        "--cases", default=str(Path(__file__).with_name("temporal_cases.json"))
    )
    parser.add_argument("--target", choices=["mnemo", "zep", "both"], default="mnemo")
    parser.add_argument("--mnemo-base-url", default="http://localhost:8080")
    parser.add_argument("--zep-base-url", default="https://api.getzep.com/api/v2")
    parser.add_argument("--zep-api-key-file", default="zep_api.key")
    args = parser.parse_args()

    with open(args.cases, "r", encoding="utf-8") as f:
        cases = json.load(f)

    results: list[EvalResult] = []

    if args.target in ("mnemo", "both"):
        mnemo = MnemoBackend(args.mnemo_base_url)
        results.append(run_profile(mnemo, cases, "temporal"))
        results.append(run_profile(mnemo, cases, "baseline"))

    if args.target in ("zep", "both"):
        key = load_key(args.zep_api_key_file)
        zep = ZepBackend(args.zep_base_url, key)
        # Zep Memory API does not expose direct equivalents for Mnemo temporal controls.
        # We run baseline-style retrieval for comparison.
        results.append(run_profile(zep, cases, "baseline"))

    print_markdown(results)


if __name__ == "__main__":
    main()
