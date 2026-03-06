"""Shared HTTP transport helpers for the Mnemo SDK."""

from __future__ import annotations

import json
import math
import random
import time
import uuid
from typing import Any
from urllib import error, request

from mnemo._errors import (
    MnemoConnectionError,
    MnemoHttpError,
    MnemoNotFoundError,
    MnemoRateLimitError,
    MnemoTimeoutError,
    MnemoValidationError,
)


class SyncTransport:
    """Synchronous HTTP transport using stdlib urllib."""

    def __init__(
        self,
        base_url: str,
        api_key: str | None,
        timeout_s: float,
        max_retries: int,
        retry_backoff_s: float,
        default_request_id: str | None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout_s = timeout_s
        self.max_retries = max_retries
        self.retry_backoff_s = retry_backoff_s
        self.default_request_id = default_request_id

    def _headers(self, request_id: str | None = None) -> dict[str, str]:
        headers: dict[str, str] = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        rid = request_id or self.default_request_id
        if rid:
            headers["x-mnemo-request-id"] = rid
        return headers

    def request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        *,
        request_id: str | None = None,
    ) -> tuple[dict[str, Any], str | None]:
        """Make an HTTP request. Returns (response_body, response_request_id)."""
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        headers = self._headers(request_id)
        url = f"{self.base_url}{path}"
        req = request.Request(url, data=data, method=method, headers=headers)

        attempt = 0
        while True:
            try:
                with request.urlopen(req, timeout=self.timeout_s) as resp:
                    raw = resp.read().decode("utf-8")
                    body: dict[str, Any] = {} if not raw else json.loads(raw)
                    resp_rid = resp.headers.get("x-mnemo-request-id")
                    return body, resp_rid

            except error.HTTPError as exc:
                parsed = _parse_error_body(exc)
                message = _extract_error_message(parsed)
                error_code = _extract_error_code(parsed)
                retry_after_ms = _extract_retry_after_ms(parsed, exc)
                resp_rid = exc.headers.get("x-mnemo-request-id")

                http_error: MnemoHttpError
                if exc.code == 429:
                    http_error = MnemoRateLimitError(
                        exc.code,
                        message,
                        retry_after_ms=retry_after_ms,
                        error_code=error_code,
                        body=parsed,
                        request_id=resp_rid,
                    )
                elif exc.code == 404:
                    http_error = MnemoNotFoundError(
                        exc.code,
                        message,
                        error_code=error_code,
                        body=parsed,
                        request_id=resp_rid,
                    )
                elif exc.code == 400:
                    http_error = MnemoValidationError(
                        exc.code,
                        message,
                        error_code=error_code,
                        body=parsed,
                        request_id=resp_rid,
                    )
                else:
                    http_error = MnemoHttpError(
                        exc.code,
                        message,
                        error_code=error_code,
                        body=parsed,
                        request_id=resp_rid,
                    )

                if not _should_retry(exc.code, attempt, self.max_retries):
                    raise http_error from exc

            except TimeoutError as exc:
                if attempt >= self.max_retries:
                    raise MnemoTimeoutError(
                        f"Request timed out after {attempt + 1} attempts"
                    ) from exc

            except error.URLError as exc:
                if attempt >= self.max_retries:
                    raise MnemoConnectionError(f"Connection failed: {exc}") from exc

            attempt += 1
            # Exponential backoff with full jitter:
            # delay = base * 2^attempt * uniform(0, 1)
            delay = self.retry_backoff_s * math.pow(2, attempt) * random.random()
            time.sleep(delay)


def _should_retry(status_code: int, attempt: int, max_retries: int) -> bool:
    if attempt >= max_retries:
        return False
    return status_code == 429 or status_code >= 500


def _parse_error_body(exc: error.HTTPError) -> dict[str, Any] | None:
    try:
        body = exc.read().decode("utf-8")
    except Exception:
        return None
    if not body:
        return None
    try:
        parsed = json.loads(body)
        return parsed if isinstance(parsed, dict) else {"raw": body}
    except json.JSONDecodeError:
        return {"raw": body}


def _extract_error_message(parsed: dict[str, Any] | None) -> str:
    if not parsed:
        return "request failed"
    error_payload = parsed.get("error")
    if isinstance(error_payload, dict):
        msg = error_payload.get("message")
        if isinstance(msg, str) and msg:
            return msg
    raw = parsed.get("raw")
    if isinstance(raw, str) and raw:
        return raw
    return "request failed"


def _extract_error_code(parsed: dict[str, Any] | None) -> str | None:
    if not parsed:
        return None
    error_payload = parsed.get("error")
    if isinstance(error_payload, dict):
        code = error_payload.get("code")
        if isinstance(code, str) and code:
            return code
    return None


def _extract_retry_after_ms(
    parsed: dict[str, Any] | None, exc: error.HTTPError
) -> int | None:
    if parsed:
        err = parsed.get("error", {})
        if isinstance(err, dict):
            retry = err.get("retry_after_ms")
            if isinstance(retry, int):
                return retry
    retry_after = exc.headers.get("Retry-After")
    if retry_after and retry_after.isdigit():
        return int(retry_after) * 1000
    return None


def opt(d: dict[str, Any], key: str, value: Any) -> None:
    """Set key in dict only if value is not None."""
    if value is not None:
        d[key] = value
