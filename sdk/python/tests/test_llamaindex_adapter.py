"""Unit tests for the LlamaIndex adapter (mock-based, no server required).

Tests cover:
- All 7 BaseChatStore interface methods (sync + async)
- UUID cache management and _ensure_uuid discovery
- Server-side get_keys() with list_sessions fallback
- Role mapping (including enum path)
- _safe_content handling (None, list, string)
- delete_message uses server-side idx field
- delete_last_message single-fetch (no double-fetch race)
- async lock safety
- Edge cases (empty session, out-of-bounds delete, no writes yet)

Run:
    pytest tests/test_llamaindex_adapter.py -v
"""

from __future__ import annotations

import asyncio
import enum
import sys
import os
from dataclasses import dataclass
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

# Ensure the SDK is importable
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))


# ---------------------------------------------------------------------------
# Minimal LlamaIndex stubs (closely matching real API shape)
# ---------------------------------------------------------------------------


class _MessageRole(str, enum.Enum):
    """Stub matching llama_index.core.llms.MessageRole (str enum)."""

    USER = "user"
    ASSISTANT = "assistant"
    SYSTEM = "system"
    TOOL = "tool"


class _ChatMessage:
    """Minimal stub matching llama_index.core.llms.ChatMessage."""

    def __init__(
        self, role: _MessageRole | str = "user", content: str | None = ""
    ) -> None:
        self.role = role if isinstance(role, _MessageRole) else role
        self.content = content

    def __repr__(self) -> str:
        return f"ChatMessage(role={self.role!r}, content={self.content!r})"


# Patch the llama_index imports before importing the adapter
_mock_llama_core = MagicMock()
_mock_llama_core.llms.MessageRole = _MessageRole
_mock_llama_core.llms.ChatMessage = _ChatMessage

sys.modules["llama_index"] = MagicMock()
sys.modules["llama_index.core"] = _mock_llama_core
sys.modules["llama_index.core.llms"] = _mock_llama_core.llms

# Now import the adapter (it will find our stubs)
from mnemo.ext.llamaindex import (
    MnemoChatStore,
    _mnemo_role_to_llamaindex,
    _mnemo_to_llamaindex,
    _role_value,
    _safe_content,
)
from mnemo._models import (
    Message,
    MessagesResult,
    DeleteResult,
    RememberResult,
    SessionInfo,
    SessionsResult,
)
from mnemo._errors import MnemoError


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_mock_client(
    session_id: str = "ses-uuid-1",
    user_id: str = "usr-uuid-1",
) -> MagicMock:
    """Create a mock Mnemo client with predictable return values."""
    client = MagicMock()
    client.add.return_value = RememberResult(
        ok=True, user_id=user_id, session_id=session_id, episode_id="ep-1"
    )
    client.get_messages.return_value = MessagesResult(
        messages=[], count=0, session_id=session_id
    )
    client.clear_messages.return_value = DeleteResult(deleted=True)
    client.delete_message.return_value = DeleteResult(deleted=True)
    client.list_sessions.return_value = SessionsResult(sessions=[], count=0)
    return client


def _make_async_mock_client(
    session_id: str = "ses-uuid-1",
    user_id: str = "usr-uuid-1",
) -> MagicMock:
    """Create an async mock client."""
    client = MagicMock()
    client.add = AsyncMock(
        return_value=RememberResult(
            ok=True, user_id=user_id, session_id=session_id, episode_id="ep-1"
        )
    )
    client.get_messages = AsyncMock(
        return_value=MessagesResult(messages=[], count=0, session_id=session_id)
    )
    client.clear_messages = AsyncMock(return_value=DeleteResult(deleted=True))
    client.delete_message = AsyncMock(return_value=DeleteResult(deleted=True))
    client.list_sessions = AsyncMock(return_value=SessionsResult(sessions=[], count=0))
    return client


def _msg(role: str | _MessageRole, content: str | None = "") -> _ChatMessage:
    """Shorthand to create a stub ChatMessage."""
    return _ChatMessage(role=role, content=content)


# ---------------------------------------------------------------------------
# Tests: Constructor
# ---------------------------------------------------------------------------


class TestMnemoChatStoreInit:
    def test_basic_init(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")
        assert store.user_id == "alice"
        assert store._uuid_cache == {}
        assert store._user_uuid is None

    def test_has_async_lock(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")
        assert isinstance(store._async_lock, asyncio.Lock)


# ---------------------------------------------------------------------------
# Tests: _safe_content
# ---------------------------------------------------------------------------


class TestSafeContent:
    def test_none_returns_empty(self) -> None:
        assert _safe_content(None) == ""

    def test_string_passthrough(self) -> None:
        assert _safe_content("hello") == "hello"

    def test_list_multimodal(self) -> None:
        content = [{"text": "hello"}, {"text": "world"}]
        assert _safe_content(content) == "hello world"

    def test_list_mixed(self) -> None:
        content = [{"text": "a"}, "b", {"image": "url"}]
        assert _safe_content(content) == "a b"

    def test_empty_string(self) -> None:
        assert _safe_content("") == ""


# ---------------------------------------------------------------------------
# Tests: _role_value
# ---------------------------------------------------------------------------


class TestRoleValue:
    def test_enum_role(self) -> None:
        assert _role_value(_MessageRole.ASSISTANT) == "assistant"

    def test_string_role(self) -> None:
        assert _role_value("user") == "user"

    def test_uppercase_normalized(self) -> None:
        assert _role_value("ASSISTANT") == "assistant"

    def test_enum_value_lowercased(self) -> None:
        assert _role_value(_MessageRole.USER) == "user"


# ---------------------------------------------------------------------------
# Tests: add_message
# ---------------------------------------------------------------------------


class TestAddMessage:
    def test_add_message_caches_uuid(self) -> None:
        client = _make_mock_client(session_id="uuid-abc")
        store = MnemoChatStore(client=client, user_id="alice")

        store.add_message("session-1", _msg("user", "hello"))

        client.add.assert_called_once_with(
            "alice", "hello", session="session-1", role="user"
        )
        assert store._uuid_cache["session-1"] == "uuid-abc"

    def test_add_message_resolves_user_uuid(self) -> None:
        client = _make_mock_client(user_id="usr-real-uuid")
        store = MnemoChatStore(client=client, user_id="alice")

        store.add_message("s1", _msg(_MessageRole.ASSISTANT, "hi"))
        assert store._user_uuid == "usr-real-uuid"

    def test_add_message_none_content(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        store.add_message("s1", _msg("user", None))
        # Should not send "None" string
        client.add.assert_called_once_with("alice", "", session="s1", role="user")

    def test_add_message_no_idx_param(self) -> None:
        """add_message should not accept idx (removed in hardening)."""
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")
        # Should work with just key + message
        store.add_message("s1", _msg("user", "x"))
        assert client.add.called


# ---------------------------------------------------------------------------
# Tests: get_messages
# ---------------------------------------------------------------------------


class TestGetMessages:
    def test_get_messages_empty_no_uuid(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        result = store.get_messages("unknown-session")
        assert result == []

    def test_get_messages_discovers_uuid_via_ensure(self) -> None:
        """get_messages should try to discover UUID from server."""
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[SessionInfo(id="uuid-discovered", name="target-session")],
            count=1,
        )
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="found", created_at="t")
            ],
            count=1,
            session_id="uuid-discovered",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid"  # simulate prior write resolved this

        msgs = store.get_messages("target-session")
        assert len(msgs) == 1
        client.list_sessions.assert_called_once()

    def test_get_messages_returns_converted(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="hello", created_at="t1"),
                Message(
                    idx=1, id="m2", role="assistant", content="hi", created_at="t2"
                ),
            ],
            count=2,
            session_id="ses-1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["my-key"] = "ses-1"

        msgs = store.get_messages("my-key")
        assert len(msgs) == 2
        assert isinstance(msgs[0], _ChatMessage)
        assert msgs[0].content == "hello"
        assert msgs[0].role == _MessageRole.USER
        assert msgs[1].role == _MessageRole.ASSISTANT


# ---------------------------------------------------------------------------
# Tests: set_messages
# ---------------------------------------------------------------------------


class TestSetMessages:
    def test_set_messages_clears_then_writes(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        msgs = [_msg("user", "a"), _msg("assistant", "b")]
        store.set_messages("s1", msgs)

        client.clear_messages.assert_called_once_with("uuid-s1")
        assert client.add.call_count == 2

    def test_set_messages_discovers_uuid(self) -> None:
        """set_messages should use _ensure_uuid to find existing sessions."""
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[SessionInfo(id="uuid-existing", name="existing-session")],
            count=1,
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid"

        store.set_messages("existing-session", [_msg("user", "replaced")])
        # Should have discovered UUID and cleared
        client.clear_messages.assert_called_once_with("uuid-existing")

    def test_set_messages_no_prior_uuid_no_clear(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        store.set_messages("brand-new-session", [_msg("user", "hi")])
        client.clear_messages.assert_not_called()
        client.add.assert_called_once()


# ---------------------------------------------------------------------------
# Tests: delete_message (uses server-side idx)
# ---------------------------------------------------------------------------


class TestDeleteMessage:
    def test_delete_message_out_of_bounds(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[], count=0, session_id="s1"
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        result = store.delete_message("s1", 5)
        assert result is None

    def test_delete_message_negative_index(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="x", created_at="t")
            ],
            count=1,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        result = store.delete_message("s1", -1)
        assert result is None

    def test_delete_message_uses_server_idx(self) -> None:
        """delete_message should use the server's idx field, not list position."""
        client = _make_mock_client()
        # Simulate non-contiguous indices (after prior deletions)
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="first", created_at="t1"),
                Message(
                    idx=3, id="m4", role="assistant", content="fourth", created_at="t4"
                ),
            ],
            count=2,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        # Delete list position 1 (which has server idx=3)
        removed = store.delete_message("s1", 1)
        assert removed is not None
        # Should pass server idx 3, not list position 1
        client.delete_message.assert_called_once_with("uuid-s1", 3)

    def test_delete_message_no_uuid(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        result = store.delete_message("unknown", 0)
        assert result is None


# ---------------------------------------------------------------------------
# Tests: delete_last_message (single fetch, no double-fetch race)
# ---------------------------------------------------------------------------


class TestDeleteLastMessage:
    def test_delete_last_message_empty(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        result = store.delete_last_message("unknown")
        assert result is None

    def test_delete_last_message_uses_server_idx(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="a", created_at="t"),
                Message(idx=5, id="m6", role="assistant", content="b", created_at="t"),
            ],
            count=2,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        removed = store.delete_last_message("s1")
        assert removed is not None
        # Should use server idx 5, and only one get_messages call
        client.delete_message.assert_called_once_with("uuid-s1", 5)
        client.get_messages.assert_called_once()

    def test_delete_last_message_single_fetch(self) -> None:
        """Verify only 1 get_messages call (not 2 like before fix)."""
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="x", created_at="t")
            ],
            count=1,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        store.delete_last_message("s1")
        assert client.get_messages.call_count == 1


# ---------------------------------------------------------------------------
# Tests: delete_messages
# ---------------------------------------------------------------------------


class TestDeleteMessages:
    def test_delete_messages_returns_existing(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="hello", created_at="t1")
            ],
            count=1,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        result = store.delete_messages("s1")
        assert result is not None
        assert len(result) == 1
        client.clear_messages.assert_called_once_with("uuid-s1")

    def test_delete_messages_empty_returns_none(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        result = store.delete_messages("unknown")
        assert result is None


# ---------------------------------------------------------------------------
# Tests: get_keys (server-side)
# ---------------------------------------------------------------------------


class TestGetKeys:
    def test_get_keys_no_writes_returns_empty(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        keys = store.get_keys()
        assert keys == []
        client.list_sessions.assert_not_called()

    def test_get_keys_server_side_after_write(self) -> None:
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[
                SessionInfo(id="uuid-1", name="session-a", user_id="usr-1"),
                SessionInfo(id="uuid-2", name="session-b", user_id="usr-1"),
            ],
            count=2,
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"

        keys = store.get_keys()
        assert "session-a" in keys
        assert "session-b" in keys

    def test_get_keys_merges_local_and_server(self) -> None:
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[SessionInfo(id="uuid-1", name="server-session")],
            count=1,
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"
        store._uuid_cache["local-only"] = "uuid-local"

        keys = store.get_keys()
        assert "server-session" in keys
        assert "local-only" in keys

    def test_get_keys_fallback_on_mnemo_error(self) -> None:
        """Should catch MnemoError specifically, not bare Exception."""
        client = _make_mock_client()
        client.list_sessions.side_effect = MnemoError("network error")
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"
        store._uuid_cache["cached-key"] = "uuid-cached"

        keys = store.get_keys()
        assert keys == ["cached-key"]

    def test_get_keys_does_not_catch_programming_error(self) -> None:
        """TypeError/KeyError should NOT be swallowed."""
        client = _make_mock_client()
        client.list_sessions.side_effect = TypeError("bug in parsing")
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"

        with pytest.raises(TypeError):
            store.get_keys()


# ---------------------------------------------------------------------------
# Tests: Role mapping
# ---------------------------------------------------------------------------


class TestRoleMapping:
    def test_mnemo_role_to_llamaindex_user(self) -> None:
        assert _mnemo_role_to_llamaindex("user") == _MessageRole.USER

    def test_mnemo_role_to_llamaindex_human(self) -> None:
        assert _mnemo_role_to_llamaindex("human") == _MessageRole.USER

    def test_mnemo_role_to_llamaindex_ai(self) -> None:
        assert _mnemo_role_to_llamaindex("ai") == _MessageRole.ASSISTANT

    def test_mnemo_role_to_llamaindex_bot(self) -> None:
        assert _mnemo_role_to_llamaindex("bot") == _MessageRole.ASSISTANT

    def test_mnemo_role_to_llamaindex_assistant(self) -> None:
        assert _mnemo_role_to_llamaindex("assistant") == _MessageRole.ASSISTANT

    def test_mnemo_role_to_llamaindex_system(self) -> None:
        assert _mnemo_role_to_llamaindex("system") == _MessageRole.SYSTEM

    def test_mnemo_role_to_llamaindex_tool(self) -> None:
        assert _mnemo_role_to_llamaindex("tool") == _MessageRole.TOOL

    def test_mnemo_role_to_llamaindex_function(self) -> None:
        assert _mnemo_role_to_llamaindex("function") == _MessageRole.TOOL

    def test_mnemo_role_to_llamaindex_none_defaults_user(self) -> None:
        assert _mnemo_role_to_llamaindex(None) == _MessageRole.USER

    def test_mnemo_role_to_llamaindex_unknown_defaults_user(self) -> None:
        assert _mnemo_role_to_llamaindex("narrator") == _MessageRole.USER

    def test_mnemo_role_case_insensitive(self) -> None:
        assert _mnemo_role_to_llamaindex("ASSISTANT") == _MessageRole.ASSISTANT


# ---------------------------------------------------------------------------
# Tests: _mnemo_to_llamaindex converter
# ---------------------------------------------------------------------------


class TestMnemoToLlamaindex:
    def test_converts_message(self) -> None:
        msg = Message(idx=0, id="m1", role="user", content="hello", created_at="t")
        result = _mnemo_to_llamaindex(msg)
        assert isinstance(result, _ChatMessage)
        assert result.content == "hello"
        assert result.role == _MessageRole.USER


# ---------------------------------------------------------------------------
# Tests: Async variants
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestAsyncVariants:
    async def test_aadd_message(self) -> None:
        client = _make_async_mock_client(session_id="async-uuid", user_id="async-usr")
        store = MnemoChatStore(client=client, user_id="alice")

        await store.aadd_message("s1", _msg("user", "async hello"))
        client.add.assert_awaited_once()
        assert store._uuid_cache["s1"] == "async-uuid"
        assert store._user_uuid == "async-usr"

    async def test_async_add_message_alias(self) -> None:
        """LlamaIndex uses async_add_message as canonical name."""
        # Check that the class-level attribute points to the same function
        assert MnemoChatStore.async_add_message is MnemoChatStore.aadd_message

    async def test_aget_messages_empty(self) -> None:
        client = _make_async_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        msgs = await store.aget_messages("unknown")
        assert msgs == []

    async def test_aget_messages_returns_converted(self) -> None:
        client = _make_async_mock_client()
        client.get_messages = AsyncMock(
            return_value=MessagesResult(
                messages=[
                    Message(idx=0, id="m1", role="user", content="hi", created_at="t")
                ],
                count=1,
                session_id="s1",
            )
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        msgs = await store.aget_messages("s1")
        assert len(msgs) == 1
        assert isinstance(msgs[0], _ChatMessage)

    async def test_aset_messages(self) -> None:
        client = _make_async_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        await store.aset_messages("s1", [_msg("user", "replaced")])
        client.clear_messages.assert_awaited_once_with("uuid-s1")
        client.add.assert_awaited_once()

    async def test_adelete_messages(self) -> None:
        client = _make_async_mock_client()
        client.get_messages = AsyncMock(
            return_value=MessagesResult(
                messages=[
                    Message(idx=0, id="m1", role="user", content="x", created_at="t")
                ],
                count=1,
                session_id="s1",
            )
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        result = await store.adelete_messages("s1")
        assert result is not None
        assert len(result) == 1
        client.clear_messages.assert_awaited_once()

    async def test_adelete_message_uses_server_idx(self) -> None:
        client = _make_async_mock_client()
        client.get_messages = AsyncMock(
            return_value=MessagesResult(
                messages=[
                    Message(idx=0, id="m1", role="user", content="a", created_at="t"),
                    Message(
                        idx=7, id="m8", role="assistant", content="b", created_at="t"
                    ),
                ],
                count=2,
                session_id="s1",
            )
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        removed = await store.adelete_message("s1", 1)
        assert removed is not None
        client.delete_message.assert_awaited_once_with("uuid-s1", 7)

    async def test_adelete_last_message(self) -> None:
        client = _make_async_mock_client()
        client.get_messages = AsyncMock(
            return_value=MessagesResult(
                messages=[
                    Message(idx=0, id="m1", role="user", content="a", created_at="t"),
                    Message(
                        idx=2, id="m3", role="assistant", content="b", created_at="t"
                    ),
                ],
                count=2,
                session_id="s1",
            )
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        removed = await store.adelete_last_message("s1")
        assert removed is not None
        client.delete_message.assert_awaited_once_with("uuid-s1", 2)

    async def test_aget_keys_server_side(self) -> None:
        client = _make_async_mock_client()
        client.list_sessions = AsyncMock(
            return_value=SessionsResult(
                sessions=[SessionInfo(id="uuid-1", name="async-session")],
                count=1,
            )
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid"

        keys = await store.aget_keys()
        assert "async-session" in keys

    async def test_aget_keys_fallback_on_mnemo_error(self) -> None:
        client = _make_async_mock_client()
        client.list_sessions = AsyncMock(side_effect=MnemoError("fail"))
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid"
        store._uuid_cache["local"] = "uuid-local"

        keys = await store.aget_keys()
        assert keys == ["local"]

    async def test_aadd_message_none_content(self) -> None:
        client = _make_async_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        await store.aadd_message("s1", _msg("user", None))
        client.add.assert_awaited_once_with("alice", "", session="s1", role="user")


# ---------------------------------------------------------------------------
# Tests: Full workflow (integration-style with mocks)
# ---------------------------------------------------------------------------


class TestFullWorkflow:
    def test_write_read_delete_cycle(self) -> None:
        """Simulate a full write -> read -> delete -> get_keys cycle."""
        client = _make_mock_client(session_id="ses-uuid-1", user_id="usr-uuid-1")

        written_messages: list[Message] = []

        def mock_add(
            user: str, content: str, session: str = "", role: str = "user"
        ) -> RememberResult:
            written_messages.append(
                Message(
                    idx=len(written_messages),
                    id=f"m{len(written_messages)}",
                    role=role,
                    content=content,
                    created_at="t",
                )
            )
            return RememberResult(
                ok=True,
                user_id="usr-uuid-1",
                session_id="ses-uuid-1",
                episode_id="ep-1",
            )

        client.add.side_effect = mock_add

        def mock_get_messages(session_id: str) -> MessagesResult:
            return MessagesResult(
                messages=list(written_messages),
                count=len(written_messages),
                session_id=session_id,
            )

        client.get_messages.side_effect = mock_get_messages

        client.list_sessions.return_value = SessionsResult(
            sessions=[
                SessionInfo(id="ses-uuid-1", name="my-chat", user_id="usr-uuid-1")
            ],
            count=1,
        )

        store = MnemoChatStore(client=client, user_id="alice")

        store.add_message("my-chat", _msg("user", "Hello"))
        store.add_message("my-chat", _msg(_MessageRole.ASSISTANT, "Hi there"))

        assert store._uuid_cache["my-chat"] == "ses-uuid-1"
        assert store._user_uuid == "usr-uuid-1"

        msgs = store.get_messages("my-chat")
        assert len(msgs) == 2

        keys = store.get_keys()
        assert "my-chat" in keys
        client.list_sessions.assert_called_once_with("usr-uuid-1")
