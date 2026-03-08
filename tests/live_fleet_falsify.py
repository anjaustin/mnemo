#!/usr/bin/env python3
"""Repeated live-fleet falsification checks.

Runs a semantic create/write/recall/cleanup cycle against each live target.
Uses only the Python standard library so it can run anywhere `python3` exists.
"""

from __future__ import annotations

import json
import sys
import time
import uuid
from dataclasses import dataclass
from typing import Any
from urllib import error, request


@dataclass
class Target:
    name: str
    base_url: str


TARGETS = [
    Target("render", "https://mnemo-m70w.onrender.com"),
    Target("northflank", "https://http--mnemo-server--blcxq2rhfzbr.code.run"),
    Target("railway", "https://mnemo-production-be62.up.railway.app"),
    Target("digitalocean", "http://157.230.213.155:8080"),
    Target("vultr", "http://173.199.127.234:8080"),
    Target("aws", "http://3.238.130.59:8080"),
    Target("gcp", "http://34.133.58.28:8080"),
    Target("linode", "http://172.232.7.137:8080"),
]


def http_json(
    method: str, url: str, body: dict[str, Any] | None = None
) -> tuple[int, dict[str, Any]]:
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = request.Request(url, data=data, headers=headers, method=method)
    try:
        with request.urlopen(req, timeout=30) as resp:
            text = resp.read().decode("utf-8")
            return resp.getcode(), json.loads(text) if text else {}
    except error.HTTPError as e:
        text = e.read().decode("utf-8")
        payload = json.loads(text) if text else {}
        return e.code, payload


def http_status(method: str, url: str) -> int:
    req = request.Request(url, method=method)
    try:
        with request.urlopen(req, timeout=30) as resp:
            return resp.getcode()
    except error.HTTPError as e:
        return e.code


def run_target(target: Target, iterations: int = 2) -> None:
    print(f"== {target.name} :: {target.base_url} ==")
    health = http_status("GET", f"{target.base_url}/health")
    if health != 200:
        raise RuntimeError(f"health failed: {health}")

    for i in range(iterations):
        uid = None
        ext = f"fleet-{target.name}-{i}-{uuid.uuid4()}"
        try:
            status, body = http_json(
                "POST",
                f"{target.base_url}/api/v1/users",
                {
                    "name": "Fleet Falsify",
                    "email": f"{ext}@test.com",
                    "external_id": ext,
                },
            )
            if status != 201:
                raise RuntimeError(f"user create failed: {status} {body}")
            uid = body["id"]

            status, body = http_json(
                "POST",
                f"{target.base_url}/api/v1/sessions",
                {"user_id": uid, "name": "Fleet Session"},
            )
            if status != 201:
                raise RuntimeError(f"session create failed: {status} {body}")

            fact = "I like tea in the afternoon."
            status, body = http_json(
                "POST",
                f"{target.base_url}/api/v1/memory",
                {
                    "user": uid,
                    "session": "Fleet Session",
                    "text": fact,
                    "role": "user",
                    "name": "Fleet",
                },
            )
            if status != 201:
                raise RuntimeError(f"memory write failed: {status} {body}")

            time.sleep(1.5)
            status, body = http_json(
                "POST",
                f"{target.base_url}/api/v1/memory/{uid}/context",
                {"query": "What did I say about tea?", "max_tokens": 300},
            )
            if status != 200:
                raise RuntimeError(f"context failed: {status} {body}")
            context = (body.get("context") or "").lower()
            if "tea" not in context:
                raise RuntimeError(f"semantic recall failed: {context!r}")
            print(f"  pass iteration {i + 1}: semantic recall contains tea")
        finally:
            if uid:
                cleanup = http_status("DELETE", f"{target.base_url}/api/v1/users/{uid}")
                print(f"  cleanup status: {cleanup}")


def main() -> int:
    failures: list[str] = []
    for target in TARGETS:
        try:
            run_target(target)
        except Exception as exc:  # noqa: BLE001
            failures.append(f"{target.name}: {exc}")
            print(f"FAIL {target.name}: {exc}", file=sys.stderr)

    print()
    if failures:
        print("Fleet falsification failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"Fleet falsification passed for {len(TARGETS)} target(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
