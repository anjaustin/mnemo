#!/usr/bin/env python3
"""eval/backends/custom_backend.py — template for adding a new memory backend.

To evaluate your own memory system against the Mnemo eval framework:

1. Copy this file and rename it (e.g. eval/backends/my_system_backend.py)
2. Fill in the four abstract methods below
3. Pass your backend to any harness:

    from backends.my_system_backend import MySystemBackend
    backend = MySystemBackend(base_url="http://localhost:9000")

    # Or via the CLI:
    python -m mnemo_eval --backend custom --backend-module backends.my_system_backend \\
        --backend-class MySystemBackend --base-url http://localhost:9000

The only requirement: Python 3.10+, stdlib only (no third-party packages needed
to implement the interface — though your backend may use whatever it likes
internally, e.g. requests, httpx, etc.)
"""

from __future__ import annotations

import sys
import time
from pathlib import Path
from typing import Any

# Allow running this file directly from the eval/ directory
sys.path.insert(0, str(Path(__file__).parent.parent))

from lib import HttpClient, MemoryBackend  # noqa: E402


class CustomBackend(MemoryBackend):
    """Skeleton implementation of MemoryBackend for a custom memory system.

    Replace every ``raise NotImplementedError`` with your system's HTTP calls
    (or SDK calls, or whatever the system exposes).

    The four methods must be:
    - Idempotent where possible (setup_user should create fresh state each call)
    - Side-effect-free on error (cleanup should be resilient)
    - Thread-safe per-instance (each harness creates its own backend instance)

    Args:
        base_url: Base URL of your memory system's API.
        **kwargs: Any additional constructor arguments your system needs
                  (e.g. api_key, namespace, tenant_id).
    """

    #: Short identifier used in result file names and output tables.
    #: Change this to your system's name, e.g. "mem0", "memgpt", "letta".
    name = "custom"

    def __init__(self, base_url: str, **kwargs: Any):
        self.base_url = base_url.rstrip("/")
        # You can use the built-in HttpClient for zero-dependency HTTP:
        #   self.http = HttpClient(base_url, headers={"Authorization": f"Bearer {api_key}"})
        # Or use any HTTP library you prefer.
        self.http = HttpClient(base_url)

    # ── Required: four abstract methods ───────────────────────────────────────

    def setup_user(self, external_id: str) -> tuple[str, str]:
        """Create an isolated test user and session for this eval run.

        Each harness calls this once per test case, using a unique external_id
        (e.g. "eval_temporal_001"). Isolation is critical: facts stored in one
        test case must not bleed into another.

        Returns:
            (user_id, session_id) — any opaque strings your system uses
            to scope subsequent ingest/query/cleanup calls.

        Example for a system with /users and /sessions endpoints::

            status, user = self.http.req(
                "/users", "POST",
                {"external_id": external_id, "name": external_id}
            )
            if status != 201:
                raise RuntimeError(f"user create failed: {status} {user}")
            user_id = user["id"]

            status, session = self.http.req(
                "/sessions", "POST",
                {"user_id": user_id}
            )
            session_id = session["id"]
            return user_id, session_id
        """
        raise NotImplementedError(
            "Implement setup_user(): create a user+session and return (user_id, session_id)"
        )

    def ingest(
        self,
        user_id: str,
        session_id: str,
        content: str,
        created_at: str | None = None,
    ) -> bool:
        """Store one episode (a message, document, or event) for the given user.

        Args:
            user_id:    From setup_user().
            session_id: From setup_user().
            content:    Natural-language text to store.
            created_at: ISO 8601 timestamp if the episode has a specific time
                        (important for temporal cases). None = now.

        Returns:
            True on success, False on failure.

        Example::

            payload = {"role": "user", "content": content}
            if created_at:
                payload["created_at"] = created_at
            status, _ = self.http.req(
                f"/sessions/{session_id}/messages", "POST", payload
            )
            return status in (200, 201)
        """
        raise NotImplementedError(
            "Implement ingest(): store content for user_id/session_id, return True on success"
        )

    def query(
        self,
        user_id: str,
        session_id: str,
        query: dict[str, Any],
        profile: str = "default",
    ) -> tuple[int, str, float]:
        """Retrieve memory context relevant to a query.

        Args:
            user_id:    From setup_user().
            session_id: From setup_user().
            query:      Dict with at minimum ``{"text": str}``. May also include:
                        - ``"mode"``: "temporal" | "semantic" | "hybrid"
                        - ``"time_intent"``: "recent" | "before:{date}" | "after:{date}"
                        - ``"as_of"``: ISO 8601 timestamp for point-in-time queries
            profile:    Retrieval tuning hint. Values: "temporal", "baseline", "default".
                        Harnesses use this to enable/disable temporal features.

        Returns:
            (http_status, context_text, latency_ms)

            context_text is the memory context your system returns — the raw
            text that will be checked against expected keywords/facts.
            latency_ms must be measured from the start of the network call to
            the end (do NOT include test setup).

        Example::

            payload = {"query": query["text"], "user_id": user_id}
            started = time.perf_counter()
            status, response = self.http.req("/context", "POST", payload)
            latency_ms = (time.perf_counter() - started) * 1000.0
            if status != 200:
                return status, "", latency_ms
            return status, response.get("context", ""), latency_ms
        """
        raise NotImplementedError(
            "Implement query(): retrieve context and return (http_status, context_text, latency_ms)"
        )

    def cleanup(self, user_id: str, session_id: str) -> None:
        """Delete all test data created by this eval run for the given user.

        This is called after every test case. Failures here should be logged
        but should not cause the test to fail — best-effort cleanup.

        Example::

            self.http.req(f"/sessions/{session_id}", "DELETE")
            self.http.req(f"/users/{user_id}", "DELETE")
        """
        raise NotImplementedError(
            "Implement cleanup(): delete test data for user_id/session_id"
        )


# ── Optional: override these for richer output ─────────────────────────────────

# Uncomment and implement if your system has a health check endpoint:
#
#   def health_check(self) -> bool:
#       status, _ = self.http.req("/health", "GET")
#       return status == 200


# ── Quick smoke-test ───────────────────────────────────────────────────────────

if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(
        description="Smoke-test a CustomBackend implementation"
    )
    parser.add_argument("--base-url", default="http://localhost:8080")
    args = parser.parse_args()

    backend = CustomBackend(base_url=args.base_url)
    print(f"Backend: {backend.name}")
    print(f"Base URL: {backend.base_url}")
    print()
    print("Implement the four abstract methods, then run:")
    print(
        f"  python -m mnemo_eval --backend custom --base-url {args.base_url} --packs temporal"
    )
    print()
    print("Or run a single harness directly:")
    print(f"  python eval/temporal_eval.py --target custom --base-url {args.base_url}")
