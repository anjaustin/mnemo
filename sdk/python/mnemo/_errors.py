"""Mnemo SDK exception hierarchy."""

from __future__ import annotations

from typing import Any


class MnemoError(Exception):
    """Base class for all Mnemo SDK errors."""


class MnemoConnectionError(MnemoError):
    """Raised when the SDK cannot reach the Mnemo server."""


class MnemoTimeoutError(MnemoConnectionError):
    """Raised when a request exceeds the configured timeout."""


class MnemoHttpError(MnemoError):
    """Raised for non-2xx HTTP responses."""

    def __init__(
        self,
        status_code: int,
        message: str,
        *,
        error_code: str | None = None,
        body: dict[str, Any] | None = None,
        request_id: str | None = None,
    ) -> None:
        super().__init__(f"HTTP {status_code}: {message}")
        self.status_code = status_code
        self.error_code = error_code
        self.body = body
        self.request_id = request_id


class MnemoRateLimitError(MnemoHttpError):
    """Raised on HTTP 429 responses. Contains retry_after_ms when available."""

    def __init__(
        self,
        status_code: int,
        message: str,
        *,
        retry_after_ms: int | None,
        error_code: str | None = None,
        body: dict[str, Any] | None = None,
        request_id: str | None = None,
    ) -> None:
        super().__init__(
            status_code,
            message,
            error_code=error_code,
            body=body,
            request_id=request_id,
        )
        self.retry_after_ms = retry_after_ms


class MnemoNotFoundError(MnemoHttpError):
    """Raised on HTTP 404 responses."""


class MnemoValidationError(MnemoHttpError):
    """Raised on HTTP 400 responses."""
