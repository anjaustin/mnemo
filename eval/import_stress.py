#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import statistics
import time
import urllib.error
import urllib.request
import uuid
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class IterationResult:
    iteration: int
    user: str
    job_id: str
    status: str
    submit_http_status: int
    submit_ms: int
    total_ms: int
    total_messages: int
    imported_messages: int
    failed_messages: int
    sessions_touched: int
    error_count: int


class HttpClient:
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

        request = urllib.request.Request(
            f"{self.base_url}{path}",
            method=method,
            data=body,
            headers=headers,
        )

        try:
            with urllib.request.urlopen(request, timeout=180) as resp:
                payload = resp.read().decode("utf-8")
                return resp.status, (json.loads(payload) if payload else {})
        except urllib.error.HTTPError as exc:
            payload = exc.read().decode("utf-8")
            try:
                parsed = json.loads(payload)
            except json.JSONDecodeError:
                parsed = {"raw": payload}
            return exc.code, parsed
        except urllib.error.URLError as exc:
            return 0, {"raw": str(exc)}


def _extract_content(message: dict[str, Any]) -> str:
    content = message.get("content") or {}
    parts = content.get("parts") if isinstance(content, dict) else None
    if isinstance(parts, list):
        return "\n".join(str(p) for p in parts if isinstance(p, str)).strip()
    text = content.get("text") if isinstance(content, dict) else None
    return str(text).strip() if isinstance(text, str) else ""


def _role_supported(message: dict[str, Any]) -> bool:
    author = message.get("author") or {}
    role = author.get("role") if isinstance(author, dict) else None
    if not isinstance(role, str):
        return False
    return role.lower() in {
        "user",
        "assistant",
        "system",
        "tool",
        "function",
        "human",
        "ai",
    }


def load_chatgpt_export(zip_path: Path) -> tuple[list[dict[str, Any]], int]:
    with zipfile.ZipFile(zip_path) as archive:
        conversations = json.loads(archive.read("conversations.json"))

    message_count = 0
    for convo in conversations:
        mapping = convo.get("mapping") or {}
        for node in mapping.values():
            message = node.get("message")
            if (
                isinstance(message, dict)
                and _role_supported(message)
                and _extract_content(message)
            ):
                message_count += 1
    return conversations, message_count


def wait_for_job(
    http: HttpClient,
    job_id: str,
    poll_interval: float,
    timeout_sec: int,
) -> tuple[str, dict[str, Any], int]:
    started = time.time()
    while True:
        status_code, payload = http.req(f"/api/v1/import/jobs/{job_id}")
        if status_code != 200:
            return (
                "failed",
                {"errors": [f"job status fetch failed: http {status_code}"]},
                int((time.time() - started) * 1000),
            )

        state = payload.get("status", "failed")
        if state in ("completed", "failed"):
            return state, payload, int((time.time() - started) * 1000)

        if (time.time() - started) > timeout_sec:
            return (
                "failed",
                {"errors": ["timed out waiting for job"]},
                int((time.time() - started) * 1000),
            )

        time.sleep(poll_interval)


def run_iteration(
    http: HttpClient,
    iteration: int,
    conversations: list[dict[str, Any]],
    mode: str,
    user_prefix: str,
    session_name: str,
    idempotency_strategy: str,
    poll_interval: float,
    timeout_sec: int,
) -> IterationResult:
    user = f"{user_prefix}-{iteration}-{uuid.uuid4().hex[:8]}"
    if idempotency_strategy == "fixed":
        idempotency_key = f"{user_prefix}-fixed"
    else:
        idempotency_key = f"{user_prefix}-{iteration}-{uuid.uuid4().hex[:8]}"

    request_payload = {
        "user": user,
        "source": "chatgpt_export",
        "payload": {"conversations": conversations},
        "default_session": session_name,
        "dry_run": mode == "dry-run",
        "idempotency_key": idempotency_key,
    }

    submit_started = time.time()
    submit_status, submit_body = http.req(
        "/api/v1/import/chat-history",
        method="POST",
        data=request_payload,
    )
    submit_ms = int((time.time() - submit_started) * 1000)

    if submit_status not in (200, 202):
        return IterationResult(
            iteration=iteration,
            user=user,
            job_id="",
            status="failed",
            submit_http_status=submit_status,
            submit_ms=submit_ms,
            total_ms=submit_ms,
            total_messages=0,
            imported_messages=0,
            failed_messages=0,
            sessions_touched=0,
            error_count=1,
        )

    job_id = str(submit_body.get("job_id", ""))
    if not job_id:
        return IterationResult(
            iteration=iteration,
            user=user,
            job_id="",
            status="failed",
            submit_http_status=submit_status,
            submit_ms=submit_ms,
            total_ms=submit_ms,
            total_messages=0,
            imported_messages=0,
            failed_messages=0,
            sessions_touched=0,
            error_count=1,
        )

    status, job_payload, wait_ms = wait_for_job(
        http,
        job_id,
        poll_interval=poll_interval,
        timeout_sec=timeout_sec,
    )

    total_messages = int(job_payload.get("total_messages", 0))
    imported = int(job_payload.get("imported_messages", 0))
    failed = int(job_payload.get("failed_messages", 0))
    sessions_touched = int(job_payload.get("sessions_touched", 0))
    errors = job_payload.get("errors")
    error_count = len(errors) if isinstance(errors, list) else 0

    return IterationResult(
        iteration=iteration,
        user=user,
        job_id=job_id,
        status=status,
        submit_http_status=submit_status,
        submit_ms=submit_ms,
        total_ms=submit_ms + wait_ms,
        total_messages=total_messages,
        imported_messages=imported,
        failed_messages=failed,
        sessions_touched=sessions_touched,
        error_count=error_count,
    )


def print_results(
    mode: str, expected_messages: int, results: list[IterationResult]
) -> None:
    print(f"Mode: {mode}")
    print(f"Expected messages per run (from export): {expected_messages}")
    print("")
    print(
        "| Iteration | Status | HTTP | Submit (ms) | Total (ms) | Total Msgs | Imported | Failed | Sessions | Errors |"
    )
    print("|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for r in results:
        print(
            f"| {r.iteration} | {r.status} | {r.submit_http_status} | {r.submit_ms} | {r.total_ms} | {r.total_messages} | {r.imported_messages} | {r.failed_messages} | {r.sessions_touched} | {r.error_count} |"
        )

    succeeded = [r for r in results if r.status == "completed"]
    if not succeeded:
        print("\nNo successful iterations.")
        return

    total_time_s = sum(r.total_ms for r in succeeded) / 1000.0
    total_imported = sum(r.imported_messages for r in succeeded)
    throughput = (total_imported / total_time_s) if total_time_s > 0 else 0.0

    print("\nSummary")
    print(f"- successful iterations: {len(succeeded)}/{len(results)}")
    print(
        f"- median total job time: {int(statistics.median([r.total_ms for r in succeeded]))} ms"
    )
    print(
        f"- p95 total job time: {sorted([r.total_ms for r in succeeded])[max(0, int(0.95 * (len(succeeded) - 1)))]} ms"
    )
    print(f"- aggregate imported messages: {total_imported}")
    print(f"- aggregate throughput: {throughput:.2f} messages/sec")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Stress test Mnemo chat-history importer using a ChatGPT export zip"
    )
    parser.add_argument(
        "--zip-path",
        default="downloads/6957c8e02c797beeb082b42e1f53a0d4f97ed813369f7b25376485225dded6b4-2025-10-21-02-29-50-e815fa493cfa481c941b2165f06911b9.zip",
    )
    parser.add_argument("--base-url", default="http://localhost:8080")
    parser.add_argument("--mode", choices=["dry-run", "import"], default="dry-run")
    parser.add_argument("--iterations", type=int, default=2)
    parser.add_argument("--user-prefix", default="import-stress")
    parser.add_argument("--session-name", default="Imported Stress Session")
    parser.add_argument(
        "--idempotency-strategy", choices=["unique", "fixed"], default="unique"
    )
    parser.add_argument("--poll-interval", type=float, default=0.5)
    parser.add_argument("--timeout-sec", type=int, default=1800)
    args = parser.parse_args()

    zip_path = Path(args.zip_path)
    if not zip_path.exists():
        raise SystemExit(f"zip not found: {zip_path}")

    conversations, expected_messages = load_chatgpt_export(zip_path)
    http = HttpClient(args.base_url)

    # quick health check
    health_status, _ = http.req("/health")
    if health_status != 200:
        raise SystemExit(f"mnemo health check failed: http {health_status}")

    results: list[IterationResult] = []
    for i in range(1, args.iterations + 1):
        result = run_iteration(
            http=http,
            iteration=i,
            conversations=conversations,
            mode=args.mode,
            user_prefix=args.user_prefix,
            session_name=args.session_name,
            idempotency_strategy=args.idempotency_strategy,
            poll_interval=args.poll_interval,
            timeout_sec=args.timeout_sec,
        )
        results.append(result)

    print_results(args.mode, expected_messages, results)


if __name__ == "__main__":
    main()
