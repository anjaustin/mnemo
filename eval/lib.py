#!/usr/bin/env python3
"""eval/lib.py — shared foundations for the Mnemo eval framework.

Provides:
  - HttpClient          Raw HTTP helper (stdlib only, zero deps)
  - MemoryBackend       Abstract base class every backend must implement
  - MnemoBackend        Mnemo HTTP backend
  - ZepBackend          Zep HTTP backend (for cross-system comparison)
  - ResultWriter        Writes D1-format JSON result files to eval/results/
  - p_quantile()        Percentile helper used by all harnesses

All harnesses import from this module instead of duplicating code.
"""

from __future__ import annotations

import json
import os
import subprocess
import urllib.error
import urllib.parse
import urllib.request
import uuid
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

# ── Percentile helper ──────────────────────────────────────────────────────────


def p_quantile(values: list[float], q: float) -> float:
    """Return the q-th percentile of *values* (0 ≤ q ≤ 1).

    Uses linear interpolation between adjacent ranks, consistent with
    numpy.percentile(method='linear').  Returns 0.0 for empty lists.
    """
    if not values:
        return 0.0
    s = sorted(values)
    idx = q * (len(s) - 1)
    lo = int(idx)
    hi = min(lo + 1, len(s) - 1)
    frac = idx - lo
    return s[lo] + frac * (s[hi] - s[lo])


# ── HTTP client ────────────────────────────────────────────────────────────────


class HttpClient:
    """Minimal HTTP client built on urllib (zero runtime dependencies)."""

    def __init__(
        self, base_url: str, headers: dict[str, str] | None = None, timeout: int = 45
    ):
        self.base_url = base_url.rstrip("/")
        self.headers = headers or {}
        self.timeout = timeout

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
            with urllib.request.urlopen(request_obj, timeout=self.timeout) as resp:
                payload = resp.read().decode("utf-8")
                return resp.status, (json.loads(payload) if payload else {})
        except urllib.error.HTTPError as exc:
            payload = exc.read().decode("utf-8")
            try:
                parsed = json.loads(payload)
            except json.JSONDecodeError:
                parsed = {"raw": payload}
            return exc.code, parsed


# ── Backend ABC ────────────────────────────────────────────────────────────────


class MemoryBackend(ABC):
    """Abstract interface every memory system backend must implement.

    To evaluate a new system, subclass MemoryBackend and implement all four
    methods.  See eval/backends/custom_backend.py for a template.
    """

    #: Short identifier used in result files and output tables (e.g. "mnemo", "zep").
    name: str = "unnamed"

    @abstractmethod
    def setup_user(self, external_id: str) -> tuple[str, str]:
        """Create an isolated test user and session.

        Returns:
            (user_id, session_id) — opaque strings passed to subsequent calls.
        """

    @abstractmethod
    def ingest(
        self,
        user_id: str,
        session_id: str,
        content: str,
        created_at: str | None = None,
    ) -> bool:
        """Store one episode.  Returns True on success."""

    @abstractmethod
    def query(
        self,
        user_id: str,
        session_id: str,
        query: dict[str, Any],
        profile: str = "default",
    ) -> tuple[int, str, float]:
        """Retrieve memory context.

        Args:
            user_id:    From setup_user().
            session_id: From setup_user().
            query:      Dict with at minimum {"text": str}.  May include
                        "mode", "time_intent", "as_of" for temporal queries.
            profile:    Hint for backend-specific retrieval tuning.
                        Values: "temporal", "baseline", "default".

        Returns:
            (http_status, context_text, latency_ms)
        """

    @abstractmethod
    def cleanup(self, user_id: str, session_id: str) -> None:
        """Delete all test data created for this user/session."""


# ── Mnemo backend ──────────────────────────────────────────────────────────────


class MnemoBackend(MemoryBackend):
    """Mnemo HTTP API backend."""

    name = "mnemo"

    def __init__(self, base_url: str):
        self.http = HttpClient(base_url)

    def setup_user(self, external_id: str) -> tuple[str, str]:
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

    def ingest(
        self,
        user_id: str,
        session_id: str,
        content: str,
        created_at: str | None = None,
    ) -> bool:
        payload: dict[str, Any] = {
            "type": "message",
            "role": "user",
            "content": content,
        }
        if created_at:
            payload["created_at"] = created_at
        status, _ = self.http.req(
            f"/api/v1/sessions/{session_id}/episodes", "POST", payload
        )
        return status == 201

    def ingest_tracked(
        self,
        user_id: str,
        session_id: str,
        content: str,
        created_at: str | None = None,
    ) -> tuple[bool, str | None]:
        """Like ingest() but returns (success, episode_id) for polling."""
        payload: dict[str, Any] = {
            "type": "message",
            "role": "user",
            "content": content,
        }
        if created_at:
            payload["created_at"] = created_at
        status, body = self.http.req(
            f"/api/v1/sessions/{session_id}/episodes", "POST", payload
        )
        if status == 201:
            return True, body.get("id")
        return False, None

    def wait_for_processing(
        self,
        episode_ids: list[str],
        timeout_s: float = 45.0,
        poll_interval_s: float = 0.5,
    ) -> None:
        """Block until all episodes reach a terminal processing status.

        Terminal statuses: completed, failed, skipped.
        Episodes that cannot be found are treated as done (may have been
        deleted or never enqueued).  Times out silently after timeout_s.
        """
        import time

        terminal = {"completed", "failed", "skipped"}
        pending = set(episode_ids)
        deadline = time.monotonic() + timeout_s

        while pending and time.monotonic() < deadline:
            still_pending = set()
            for eid in pending:
                s, body = self.http.req(f"/api/v1/episodes/{eid}")
                if s == 404:
                    continue  # gone — treat as done
                if s != 200:
                    still_pending.add(eid)
                    continue
                ps = body.get("processing_status", "")
                if ps not in terminal:
                    still_pending.add(eid)
            pending = still_pending
            if pending:
                time.sleep(poll_interval_s)

    def query(
        self,
        user_id: str,
        session_id: str,
        query: dict[str, Any],
        profile: str = "default",
    ) -> tuple[int, str, float]:
        import time

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

        started = time.perf_counter()
        status, response = self.http.req(
            f"/api/v1/memory/{user_id}/context", "POST", payload
        )
        latency_ms = (time.perf_counter() - started) * 1000.0
        if status != 200:
            return status, "", latency_ms
        return status, response.get("context", ""), latency_ms

    def cleanup(self, user_id: str, session_id: str) -> None:
        self.http.req(f"/api/v1/users/{user_id}", "DELETE")

    # Backwards-compatibility aliases used by existing harnesses
    def create_user_session(self, external_id: str) -> tuple[str, str]:
        return self.setup_user(external_id)

    def remember(
        self, user_id: str, session_id: str, content: str, created_at: str
    ) -> bool:
        return self.ingest(user_id, session_id, content, created_at)

    def retrieve(
        self, user_id: str, session_id: str, query: dict[str, Any], profile: str
    ) -> tuple[int, str, int]:
        status, text, lat = self.query(user_id, session_id, query, profile)
        return status, text, int(lat)


# ── Zep backend ────────────────────────────────────────────────────────────────


class ZepBackend(MemoryBackend):
    """Zep Cloud/CE HTTP backend for cross-system comparison."""

    name = "zep"

    def __init__(self, base_url: str, api_key: str):
        self.http = HttpClient(
            base_url,
            headers={
                "Authorization": f"Bearer {api_key}",
                "x-api-key": api_key,
            },
        )

    def setup_user(self, external_id: str) -> tuple[str, str]:
        status, user = self.http.req(
            "/users",
            "POST",
            {"user_id": external_id, "first_name": external_id, "last_name": "Eval"},
        )
        if status not in (200, 201):
            raise RuntimeError(f"zep user create failed: {status} {user}")

        session_id = f"{external_id}-session"
        status, session = self.http.req(
            "/sessions",
            "POST",
            {"session_id": session_id, "user_id": external_id},
        )
        if status not in (200, 201):
            raise RuntimeError(f"zep session create failed: {status} {session}")
        return external_id, session_id

    def ingest(
        self,
        user_id: str,
        session_id: str,
        content: str,
        created_at: str | None = None,
    ) -> bool:
        msg: dict[str, Any] = {
            "role": user_id,
            "role_type": "user",
            "content": content,
        }
        if created_at:
            msg["created_at"] = created_at
        status, _ = self.http.req(
            f"/sessions/{session_id}/memory",
            "POST",
            {"messages": [msg], "return_context": False},
        )
        return status in (200, 201)

    def query(
        self,
        user_id: str,
        session_id: str,
        query: dict[str, Any],
        profile: str = "default",
    ) -> tuple[int, str, float]:
        import time

        self.http.req(
            f"/sessions/{session_id}/memory",
            "POST",
            {
                "messages": [
                    {"role": user_id, "role_type": "user", "content": query["text"]}
                ],
                "return_context": False,
            },
        )
        started = time.perf_counter()
        status, response = self.http.req(f"/sessions/{session_id}/memory", "GET")
        latency_ms = (time.perf_counter() - started) * 1000.0
        if status != 200:
            return status, "", latency_ms
        return status, response.get("context", ""), latency_ms

    def cleanup(self, user_id: str, session_id: str) -> None:
        self.http.req(f"/sessions/{session_id}/memory", "DELETE")
        self.http.req(f"/users/{user_id}", "DELETE")

    # Backwards-compatibility aliases
    def create_user_session(self, external_id: str) -> tuple[str, str]:
        return self.setup_user(external_id)

    def remember(
        self, user_id: str, session_id: str, content: str, created_at: str
    ) -> bool:
        return self.ingest(user_id, session_id, content, created_at)

    def retrieve(
        self, user_id: str, session_id: str, query: dict[str, Any], profile: str
    ) -> tuple[int, str, int]:
        status, text, lat = self.query(user_id, session_id, query, profile)
        return status, text, int(lat)


# ── D1: Result schema and writer ───────────────────────────────────────────────

_RESULTS_DIR = Path(__file__).parent / "results"

RESULT_SCHEMA_VERSION = 1


@dataclass
class EvalResultFile:
    """D1 result file — one per harness run.

    Written to eval/results/{workflow}_{commit}_{timestamp}.json and uploaded
    as a GitHub Actions artifact for cross-run comparison (D2).
    """

    workflow: str
    system: str
    metrics: dict[str, float] = field(default_factory=dict)
    gates: dict[str, dict[str, Any]] = field(default_factory=dict)
    commit: str = field(default_factory=lambda: _git_commit())
    branch: str = field(default_factory=lambda: _git_branch())
    timestamp: str = field(
        default_factory=lambda: datetime.now(timezone.utc).isoformat()
    )

    def add_metric(self, name: str, value: float) -> None:
        self.metrics[name] = round(value, 6)

    def add_gate(self, name: str, value: float, threshold: float, passed: bool) -> None:
        self.gates[name] = {
            "value": round(value, 6),
            "threshold": round(threshold, 6),
            "passed": passed,
        }

    def all_gates_pass(self) -> bool:
        return all(g["passed"] for g in self.gates.values())

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": RESULT_SCHEMA_VERSION,
            "commit": self.commit,
            "branch": self.branch,
            "timestamp": self.timestamp,
            "workflow": self.workflow,
            "system": self.system,
            "metrics": self.metrics,
            "gates": self.gates,
        }

    def write(self, path: Path | None = None) -> Path:
        """Write result JSON to *path* (or auto-named file in eval/results/).

        Returns the path written.
        """
        _RESULTS_DIR.mkdir(parents=True, exist_ok=True)
        if path is None:
            ts = self.timestamp.replace(":", "-").replace("+", "Z").split(".")[0]
            fname = f"{self.workflow}_{self.commit[:8]}_{ts}.json"
            path = _RESULTS_DIR / fname
        path.write_text(json.dumps(self.to_dict(), indent=2), encoding="utf-8")
        return path


class ResultWriter:
    """Convenience wrapper: accumulate metrics then write once."""

    def __init__(self, workflow: str, system: str):
        self._result = EvalResultFile(workflow=workflow, system=system)

    def metric(self, name: str, value: float) -> "ResultWriter":
        self._result.add_metric(name, value)
        return self

    def gate(
        self, name: str, value: float, threshold: float, passed: bool | None = None
    ) -> "ResultWriter":
        if passed is None:
            passed = value >= threshold
        self._result.add_gate(name, value, threshold, passed)
        self._result.add_metric(name, value)
        return self

    def write(self, path: Path | None = None) -> Path:
        return self._result.write(path)

    def all_pass(self) -> bool:
        return self._result.all_gates_pass()


# ── Git helpers ────────────────────────────────────────────────────────────────


def _git_commit() -> str:
    try:
        return (
            subprocess.check_output(
                ["git", "rev-parse", "--short", "HEAD"],
                stderr=subprocess.DEVNULL,
            )
            .decode()
            .strip()
        )
    except Exception:
        return os.environ.get("GITHUB_SHA", "unknown")[:8]


def _git_branch() -> str:
    try:
        return (
            subprocess.check_output(
                ["git", "rev-parse", "--abbrev-ref", "HEAD"],
                stderr=subprocess.DEVNULL,
            )
            .decode()
            .strip()
        )
    except Exception:
        return os.environ.get("GITHUB_REF_NAME", "unknown")


# ── Table printer ──────────────────────────────────────────────────────────────


def print_table(
    rows: list[dict[str, Any]],
    columns: list[tuple[str, str, int]],  # (key, label, width)
) -> None:
    """Print an aligned text table to stdout.

    columns: list of (key, label, min_width) tuples.
    """
    header = "  ".join(f"{label:<{w}}" for _, label, w in columns)
    sep = "  ".join("-" * w for _, _, w in columns)
    print(header)
    print(sep)
    for row in rows:
        line = "  ".join(
            f"{str(row.get(k, '')):<{w}}" for k, w in ((k, w) for k, _, w in columns)
        )
        print(line)
