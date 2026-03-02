from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from urllib import error, request


class MnemoError(Exception):
    pass


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
        self, base_url: str = "http://localhost:8080", api_key: str | None = None
    ):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key

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
        try:
            with request.urlopen(req, timeout=20) as resp:
                raw = resp.read().decode("utf-8")
                return {} if not raw else json.loads(raw)
        except error.HTTPError as exc:
            body = exc.read().decode("utf-8")
            try:
                parsed = json.loads(body)
                msg = parsed.get("error", {}).get("message", body)
            except json.JSONDecodeError:
                msg = body
            raise MnemoError(f"HTTP {exc.code}: {msg}") from exc
        except error.URLError as exc:
            raise MnemoError(f"Connection failed: {exc}") from exc
