#!/usr/bin/env python3
import argparse
import json
import sys
import time
import urllib.error
import urllib.request
import uuid


def req(base: str, method: str, path: str, payload=None, timeout: int = 20):
    body = None if payload is None else json.dumps(payload).encode()
    request = urllib.request.Request(base + path, data=body, method=method)
    if body is not None:
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request, timeout=timeout) as resp:
            text = resp.read().decode()
            return True, resp.status, json.loads(text) if text else None
    except urllib.error.HTTPError as exc:
        text = exc.read().decode()
        try:
            parsed = json.loads(text) if text else None
        except Exception:
            parsed = text
        return False, exc.code, parsed
    except Exception as exc:  # noqa: BLE001
        return False, None, {"error": type(exc).__name__, "detail": str(exc)}


def main() -> int:
    parser = argparse.ArgumentParser(description="Mnemo Ollama/LFM2 smoke harness")
    parser.add_argument("--server", default="http://127.0.0.1:8080")
    parser.add_argument("--timeout-seconds", type=int, default=180)
    parser.add_argument("--poll-interval", type=int, default=5)
    args = parser.parse_args()

    base = args.server.rstrip("/")
    user = f"ollama-smoke-{uuid.uuid4().hex[:8]}"

    deadline = time.time() + 120
    while time.time() < deadline:
        ok, status, _ = req(base, "GET", "/healthz", timeout=2)
        if ok and status == 200:
            break
        time.sleep(1)
    else:
        print(json.dumps({"error": "server did not become healthy"}, indent=2))
        return 2

    facts = [
        "My dog Atlas hates thunderstorms.",
        "I work at Orbit Harbor on robotics safety.",
        "My partner Mira loves jasmine tea.",
    ]
    writes = []
    user_id = None
    for fact in facts:
        ok, status, body = req(
            base,
            "POST",
            "/api/v1/memory",
            {"user": user, "text": fact, "role": "user"},
            timeout=30,
        )
        writes.append({"ok": ok, "status": status, "body": body})
        if not ok:
            print(json.dumps({"writes": writes}, indent=2))
            return 3
        user_id = body["user_id"]

    ok, status, body = req(
        base,
        "POST",
        f"/api/v1/memory/{user}/context",
        {
            "query": "What do you know about Atlas and Mira?",
            "max_tokens": 400,
            "min_relevance": 0.0,
        },
        timeout=20,
    )
    immediate = {"ok": ok, "status": status, "body": body}
    if not ok:
        print(json.dumps({"writes": writes, "immediate": immediate}, indent=2))
        return 4

    context = (body.get("context") or "").lower()
    if "atlas" not in context or "mira" not in context:
        print(json.dumps({"writes": writes, "immediate": immediate}, indent=2))
        return 5

    polls = []
    started = time.time()
    while time.time() - started < args.timeout_seconds:
        time.sleep(args.poll_interval)
        ok_s, status_s, spans = req(
            base, "GET", f"/api/v1/spans/user/{user_id}", timeout=10
        )
        ok_e, status_e, entities = req(
            base, "GET", f"/api/v1/users/{user_id}/entities", timeout=10
        )
        ok_c, status_c, delayed = req(
            base,
            "POST",
            f"/api/v1/memory/{user}/context",
            {
                "query": "Where do I work and what tea does Mira love?",
                "max_tokens": 400,
                "min_relevance": 0.0,
            },
            timeout=20,
        )
        poll = {
            "t": int(time.time() - started),
            "spans_ok": ok_s,
            "spans_status": status_s,
            "span_count": spans.get("count", 0)
            if ok_s and isinstance(spans, dict)
            else 0,
            "entities_ok": ok_e,
            "entities_status": status_e,
            "entity_count": entities.get("count", 0)
            if ok_e and isinstance(entities, dict)
            else 0,
            "context_ok": ok_c,
            "context_status": status_c,
            "context_excerpt": (
                delayed.get("context", "")[:160]
                if ok_c and isinstance(delayed, dict)
                else delayed
            ),
        }
        polls.append(poll)
        if poll["span_count"] > 0 or poll["entity_count"] > 0:
            print(
                json.dumps(
                    {"writes": writes, "immediate": immediate, "polls": polls}, indent=2
                )
            )
            return 0

    print(
        json.dumps({"writes": writes, "immediate": immediate, "polls": polls}, indent=2)
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
