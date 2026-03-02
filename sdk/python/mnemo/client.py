from __future__ import annotations

import json
import time
from dataclasses import dataclass
from typing import Any
from urllib import error, request


class MnemoError(Exception):
    pass


class MnemoConnectionError(MnemoError):
    pass


class MnemoTimeoutError(MnemoConnectionError):
    pass


class MnemoHttpError(MnemoError):
    def __init__(
        self,
        status_code: int,
        message: str,
        *,
        error_code: str | None = None,
        body: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(f"HTTP {status_code}: {message}")
        self.status_code = status_code
        self.error_code = error_code
        self.body = body


class MnemoRateLimitError(MnemoHttpError):
    def __init__(
        self,
        status_code: int,
        message: str,
        *,
        retry_after_ms: int | None,
        error_code: str | None = None,
        body: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(status_code, message, error_code=error_code, body=body)
        self.retry_after_ms = retry_after_ms


@dataclass(slots=True)
class RememberResult:
    ok: bool
    user_id: str
    session_id: str
    episode_id: str


@dataclass(slots=True)
class ContextResult:
    text: str
    token_count: int
    entities: list[dict[str, Any]]
    facts: list[dict[str, Any]]
    episodes: list[dict[str, Any]]
    latency_ms: int
    sources: list[str]
    mode: str
    head: dict[str, Any] | None


class Mnemo:
    def __init__(
        self,
        base_url: str = "http://localhost:8080",
        api_key: str | None = None,
        *,
        timeout_s: float = 20.0,
        max_retries: int = 2,
        retry_backoff_s: float = 0.4,
    ):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout_s = timeout_s
        self.max_retries = max_retries
        self.retry_backoff_s = retry_backoff_s

    def add(
        self,
        user: str,
        text: str,
        *,
        session: str | None = None,
        role: str = "user",
    ) -> RememberResult:
        payload: dict[str, Any] = {"user": user, "text": text, "role": role}
        if session is not None:
            payload["session"] = session

        body = self._request_json("POST", "/api/v1/memory", payload)
        return RememberResult(
            ok=bool(body.get("ok", False)),
            user_id=str(body.get("user_id", "")),
            session_id=str(body.get("session_id", "")),
            episode_id=str(body.get("episode_id", "")),
        )

    def context(
        self,
        user: str,
        query: str,
        *,
        session: str | None = None,
        max_tokens: int | None = None,
        min_relevance: float | None = None,
        mode: str | None = None,
        time_intent: str | None = None,
        as_of: str | None = None,
        temporal_weight: float | None = None,
    ) -> ContextResult:
        payload: dict[str, Any] = {"query": query}
        if session is not None:
            payload["session"] = session
        if max_tokens is not None:
            payload["max_tokens"] = max_tokens
        if min_relevance is not None:
            payload["min_relevance"] = min_relevance
        if mode is not None:
            payload["mode"] = mode
        if time_intent is not None:
            payload["time_intent"] = time_intent
        if as_of is not None:
            payload["as_of"] = as_of
        if temporal_weight is not None:
            payload["temporal_weight"] = temporal_weight

        body = self._request_json("POST", f"/api/v1/memory/{user}/context", payload)
        return ContextResult(
            text=str(body.get("context", "")),
            token_count=int(body.get("token_count", 0)),
            entities=list(body.get("entities", [])),
            facts=list(body.get("facts", [])),
            episodes=list(body.get("episodes", [])),
            latency_ms=int(body.get("latency_ms", 0)),
            sources=list(body.get("sources", [])),
            mode=str(body.get("mode", "hybrid")),
            head=(body.get("head") if isinstance(body.get("head"), dict) else None),
        )

    def context_head(
        self,
        user: str,
        query: str,
        *,
        session: str | None = None,
        max_tokens: int | None = None,
        min_relevance: float | None = None,
        time_intent: str | None = None,
        temporal_weight: float | None = None,
    ) -> ContextResult:
        return self.context(
            user,
            query,
            session=session,
            max_tokens=max_tokens,
            min_relevance=min_relevance,
            mode="head",
            time_intent=time_intent,
            temporal_weight=temporal_weight,
        )

    def _request_json(
        self, method: str, path: str, payload: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"

        req = request.Request(
            f"{self.base_url}{path}", data=data, method=method, headers=headers
        )

        attempt = 0
        while True:
            try:
                with request.urlopen(req, timeout=self.timeout_s) as resp:
                    raw = resp.read().decode("utf-8")
                    return {} if not raw else json.loads(raw)
            except error.HTTPError as exc:
                parsed = _parse_error_body(exc)
                message = _extract_error_message(parsed)
                error_code = _extract_error_code(parsed)
                retry_after_ms = _extract_retry_after_ms(parsed, exc)

                if exc.code == 429:
                    http_error = MnemoRateLimitError(
                        exc.code,
                        message,
                        retry_after_ms=retry_after_ms,
                        error_code=error_code,
                        body=parsed,
                    )
                else:
                    http_error = MnemoHttpError(
                        exc.code,
                        message,
                        error_code=error_code,
                        body=parsed,
                    )

                if not self._should_retry_http(exc.code, attempt):
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
            time.sleep(self.retry_backoff_s * attempt)

    def _should_retry_http(self, status_code: int, attempt: int) -> bool:
        if attempt >= self.max_retries:
            return False
        return status_code == 429 or status_code >= 500


def _parse_error_body(exc: error.HTTPError) -> dict[str, Any] | None:
    body = exc.read().decode("utf-8")
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
        details = parsed.get("error", {}).get("details")
        if isinstance(details, dict):
            retry = details.get("retry_after_ms")
            if isinstance(retry, int):
                return retry
    retry_after = exc.headers.get("Retry-After")
    if retry_after and retry_after.isdigit():
        return int(retry_after) * 1000
    return None
