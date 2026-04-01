#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def wait_for_url(url: str, timeout_s: int, label: str) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as resp:
                if resp.status == 200:
                    return
        except Exception:
            pass
        time.sleep(1)
    raise RuntimeError(f"Timed out waiting for {label}: {url}")


def run(
    cmd: list[str], *, env: dict[str, str] | None = None, capture: bool = False
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        text=True,
        check=True,
        capture_output=capture,
    )


def wait_for_compose_service(service: str, timeout_s: int) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        result = run(
            ["docker", "compose", "ps", "--format", "json", service], capture=True
        )
        lines = [line for line in result.stdout.splitlines() if line.strip()]
        if lines:
            data = json.loads(lines[0])
            state = data.get("State", "")
            health = data.get("Health", "")
            if state == "running" and health in {"", "healthy"}:
                return
        time.sleep(1)
    raise RuntimeError(f"Timed out waiting for compose service: {service}")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run local Ollama Mnemo validation reliably"
    )
    parser.add_argument("--port", type=int, default=18083)
    parser.add_argument("--ingest-wait", type=int, default=15)
    parser.add_argument("--startup-timeout", type=int, default=240)
    parser.add_argument("--llm-model", default="lfm25")
    parser.add_argument("--llm-timeout-ms", type=int, default=15000)
    args = parser.parse_args()

    server = f"http://127.0.0.1:{args.port}"
    log_path = Path("/tmp/mnemo-local-validation.log")
    pid = None

    env = os.environ.copy()
    env.update(
        {
            "MNEMO_SERVER_PORT": str(args.port),
            "MNEMO_AUTH_ENABLED": "false",
            "MNEMO_LLM_PROVIDER": "ollama",
            "MNEMO_LLM_BASE_URL": "http://localhost:11434/v1",
            "MNEMO_LLM_MODEL": args.llm_model,
            "MNEMO_LLM_MAX_TOKENS": "256",
            "MNEMO_LLM_REQUEST_TIMEOUT_MS": str(args.llm_timeout_ms),
            "MNEMO_EMBEDDING_PROVIDER": "local",
            "MNEMO_EMBEDDING_MODEL": "AllMiniLML6V2",
            "MNEMO_EMBEDDING_DIMENSIONS": "384",
        }
    )
    ort_dylib = Path(
        "/tmp/ort/onnxruntime-osx-arm64-1.23.0/lib/libonnxruntime.1.23.0.dylib"
    )
    if ort_dylib.exists():
        env["ORT_DYLIB_PATH"] = str(ort_dylib)

    try:
        run(["docker", "compose", "down", "-v"])
        run(["docker", "compose", "up", "-d", "redis", "qdrant"])
        wait_for_compose_service("redis", 180)
        wait_for_compose_service("qdrant", 180)

        with log_path.open("w", encoding="utf-8") as log_file:
            proc = subprocess.Popen(
                ["./target/debug/mnemo-server"],
                cwd=ROOT,
                env=env,
                stdout=log_file,
                stderr=subprocess.STDOUT,
                text=True,
            )
        pid = proc.pid

        wait_for_url(f"{server}/healthz", args.startup_timeout, "Mnemo server")

        run(
            [
                sys.executable,
                "tests/eval_recall_quality.py",
                "--server",
                server,
                "--ingest-wait",
                str(args.ingest_wait),
            ]
        )
        run(
            [
                sys.executable,
                "tests/eval_recall_atomics.py",
                "--server",
                server,
                "--ingest-wait",
                str(args.ingest_wait),
                "--startup-wait",
                "0",
            ]
        )
        print(f"Validation completed successfully. Server log: {log_path}")
        return 0
    finally:
        if pid is not None:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        try:
            run(["docker", "compose", "down", "-v"])
        except Exception:
            pass


if __name__ == "__main__":
    raise SystemExit(main())
