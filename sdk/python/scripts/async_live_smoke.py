#!/usr/bin/env python3
"""Async SDK smoke against one or more live Mnemo deployments."""

from __future__ import annotations

import asyncio
import json
import os
import sys
import uuid
from typing import Any

from mnemo import AsyncMnemo


DEFAULT_TARGETS = [
    "https://mnemo-production-be62.up.railway.app",
]


def _targets() -> list[str]:
    raw = os.getenv("MNEMO_ASYNC_SMOKE_URLS", "").strip()
    if not raw:
        return DEFAULT_TARGETS
    return [part.strip() for part in raw.split(",") if part.strip()]


async def _json_request(
    session: Any, method: str, url: str, payload: dict[str, Any] | None = None
):
    kwargs: dict[str, Any] = {"headers": {"Content-Type": "application/json"}}
    if payload is not None:
        kwargs["data"] = json.dumps(payload)
    async with session.request(method, url, **kwargs) as resp:
        text = await resp.text()
        body = json.loads(text) if text else {}
        return resp.status, body


async def smoke(base_url: str) -> None:
    import aiohttp

    print(f"== async smoke :: {base_url} ==")
    async with AsyncMnemo(base_url) as client:
        health = await client.health()
        if health.status != "ok":
            raise RuntimeError(f"health not ok: {health}")

        external_id = f"async-smoke-{uuid.uuid4()}"
        uid = None
        async with aiohttp.ClientSession(
            timeout=aiohttp.ClientTimeout(total=30)
        ) as session:
            status, body = await _json_request(
                session,
                "POST",
                f"{base_url}/api/v1/users",
                {
                    "name": "Async Smoke",
                    "email": f"{external_id}@test.com",
                    "external_id": external_id,
                },
            )
            if status != 201:
                raise RuntimeError(f"user create failed: {status} {body}")
            uid = body["id"]

            status, body = await _json_request(
                session,
                "POST",
                f"{base_url}/api/v1/sessions",
                {"user_id": uid, "name": "Async Session"},
            )
            if status != 201:
                raise RuntimeError(f"session create failed: {status} {body}")

        try:
            await client.add(
                uid,
                "I like tea in the afternoon.",
                session="Async Session",
                role="user",
            )
            ctx = await client.context(uid, "What did I say about tea?", max_tokens=300)
            if "tea" not in (ctx.text or "").lower():
                raise RuntimeError(f"semantic recall failed: {ctx.text!r}")
            print("  pass: semantic recall contains tea")
        finally:
            if uid is not None:
                async with aiohttp.ClientSession(
                    timeout=aiohttp.ClientTimeout(total=30)
                ) as session:
                    async with session.delete(f"{base_url}/api/v1/users/{uid}") as resp:
                        if resp.status not in {200, 404}:
                            raise RuntimeError(f"cleanup failed: {resp.status}")


async def main() -> int:
    failures: list[str] = []
    targets = _targets()
    for base_url in targets:
        try:
            await smoke(base_url)
        except Exception as exc:  # noqa: BLE001
            failures.append(f"{base_url}: {exc}")
            print(f"FAIL {base_url}: {exc}", file=sys.stderr)

    if failures:
        print("Async live smoke failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"Async live smoke passed for {len(targets)} target(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
