"""Pytest fixtures for Mnemo SDK tests.

Provides a connected ``Mnemo`` client against a live server.

Environment variables:
    MNEMO_BASE_URL  — override the server URL (default: http://localhost:8080)

Usage:
    pytest tests/ -v

When running via ``make test``, the Makefile starts the Docker Compose stack
before invoking pytest and tears it down after.
"""

from __future__ import annotations

import os
import time

import pytest

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

BASE_URL = os.environ.get("MNEMO_BASE_URL", "http://localhost:8080")
_HEALTH_POLL_INTERVAL_S = 1.0
_HEALTH_POLL_TIMEOUT_S = 60.0


# ---------------------------------------------------------------------------
# Server readiness helper
# ---------------------------------------------------------------------------


def _wait_for_server(base_url: str, timeout_s: float = _HEALTH_POLL_TIMEOUT_S) -> None:
    """Block until the Mnemo server health endpoint returns 200, or raise."""
    import urllib.request
    import urllib.error

    url = f"{base_url}/healthz"
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=3) as resp:
                if resp.status == 200:
                    return
        except Exception:
            pass
        time.sleep(_HEALTH_POLL_INTERVAL_S)
    raise RuntimeError(
        f"Mnemo server at {base_url} did not become healthy within {timeout_s}s. "
        "Is Docker Compose running? Try: docker compose -f docker-compose.test.yml up -d"
    )


# ---------------------------------------------------------------------------
# Session-scoped client fixture (connects once per test run)
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def mnemo_base_url() -> str:
    """Return the base URL for the Mnemo server under test."""
    return BASE_URL


@pytest.fixture(scope="session")
def mnemo_client(mnemo_base_url: str):
    """Session-scoped sync Mnemo client connected to the live server.

    Waits up to 60 seconds for the server to become healthy before yielding.
    """
    import sys
    import os

    # Allow running from the sdk/python directory or the repo root
    sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

    from mnemo import Mnemo

    _wait_for_server(mnemo_base_url)
    client = Mnemo(mnemo_base_url, timeout_s=20.0)
    yield client


@pytest.fixture(scope="session")
def client(mnemo_client):
    """Backward-compatible alias for the session-scoped sync client fixture."""
    yield mnemo_client


@pytest.fixture(scope="session")
def async_mnemo_client(mnemo_base_url: str):
    """Session-scoped async Mnemo client for use with pytest-asyncio tests.

    Caller is responsible for opening/closing with ``async with``.
    """
    import sys
    import os

    sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

    from mnemo import AsyncMnemo

    return AsyncMnemo(mnemo_base_url, timeout_s=20.0)
