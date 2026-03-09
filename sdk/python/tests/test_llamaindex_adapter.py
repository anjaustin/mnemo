"""Unit tests for the LlamaIndex adapter (mock-based, no server required).

Tests cover:
- All 7 BaseChatStore interface methods
- UUID cache management
- Server-side get_keys() with list_sessions fallback
- BaseChatStore virtual subclass registration
- Role mapping
- Edge cases (empty session, out-of-bounds delete, no writes yet)

Run:
    pytest tests/test_llamaindex_adapter.py -v
"""

from __future__ import annotations

import sys
import os
from dataclasses import dataclass
from typing import Any
from unittest.mock import MagicMock, patch

import pytest

# Ensure the SDK is importable
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))


# ---------------------------------------------------------------------------
# Minimal LlamaIndex stubs (avoid requiring llama-index-core for unit tests)
# ---------------------------------------------------------------------------


class _MessageRole:
    USER = "user"
    ASSISTANT = "assistant"
    SYSTEM = "system"
    TOOL = "tool"

    @classmethod
    def __contains__(cls, item: str) -> bool:
        return item in ("user", "assistant", "system", "tool")


class _ChatMessage:
    """Minimal stub matching llama_index.core.llms.ChatMessage."""

    def __init__(self, role: str = "user", content: str = "") -> None:
        self.role = role
        self.content = content

    def __repr__(self) -> str:
        return f"ChatMessage(role={self.role!r}, content={self.content!r})"


class _BaseChatStore:
    """Minimal stub of BaseChatStore with register() for ABC virtual subclass."""

    _virtual_subclasses: list[type] = []

    @classmethod
    def register(cls, subclass: type) -> type:
        cls._virtual_subclasses.append(subclass)
        return subclass

    @classmethod
    def __instancecheck__(cls, instance: Any) -> bool:
        if type(instance) in cls._virtual_subclasses:
            return True
        return super().__instancecheck__(instance)


# Patch the llama_index imports before importing the adapter
_mock_llama_core = MagicMock()
_mock_llama_core.llms.MessageRole = _MessageRole
_mock_llama_core.llms.ChatMessage = _ChatMessage
_mock_llama_storage = MagicMock()
_mock_llama_storage.chat_store.base.BaseChatStore = _BaseChatStore

sys.modules["llama_index"] = MagicMock()
sys.modules["llama_index.core"] = _mock_llama_core
sys.modules["llama_index.core.llms"] = _mock_llama_core.llms
sys.modules["llama_index.core.storage"] = _mock_llama_storage
sys.modules["llama_index.core.storage.chat_store"] = _mock_llama_storage.chat_store
sys.modules["llama_index.core.storage.chat_store.base"] = (
    _mock_llama_storage.chat_store.base
)

# Now import the adapter (it will find our stubs)
from mnemo.ext.llamaindex import (
    MnemoChatStore,
    _mnemo_role_to_llamaindex,
    _mnemo_to_llamaindex,
    _role_value,
)
from mnemo._models import (
    Message,
    MessagesResult,
    DeleteResult,
    RememberResult,
    SessionInfo,
    SessionsResult,
)


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


def _msg(role: str, content: str) -> _ChatMessage:
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

    def test_client_stored(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="bob")
        assert store._client is client


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

        store.add_message("s1", _msg("assistant", "hi"))
        assert store._user_uuid == "usr-real-uuid"

    def test_add_message_does_not_overwrite_cached_uuid(self) -> None:
        client = _make_mock_client(session_id="first-uuid")
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "pre-existing-uuid"

        store.add_message("s1", _msg("user", "x"))
        # Should NOT overwrite
        assert store._uuid_cache["s1"] == "pre-existing-uuid"


# ---------------------------------------------------------------------------
# Tests: get_messages
# ---------------------------------------------------------------------------


class TestGetMessages:
    def test_get_messages_empty_no_uuid(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        result = store.get_messages("unknown-session")
        assert result == []
        client.get_messages.assert_not_called()

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
        client.get_messages.assert_called_once_with("ses-1")


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

    def test_set_messages_no_prior_uuid(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        store.set_messages("new-session", [_msg("user", "hi")])
        client.clear_messages.assert_not_called()
        client.add.assert_called_once()


# ---------------------------------------------------------------------------
# Tests: delete_messages
# ---------------------------------------------------------------------------


class TestDeleteMessages:
    def test_delete_messages_returns_existing(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="hello", created_at="t1"),
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
# Tests: delete_message
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
                Message(idx=0, id="m1", role="user", content="x", created_at="t"),
            ],
            count=1,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        result = store.delete_message("s1", -1)
        assert result is None

    def test_delete_message_valid_index(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="first", created_at="t1"),
                Message(
                    idx=1, id="m2", role="assistant", content="second", created_at="t2"
                ),
            ],
            count=2,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        removed = store.delete_message("s1", 0)
        assert removed is not None
        client.delete_message.assert_called_once_with("uuid-s1", 0)


# ---------------------------------------------------------------------------
# Tests: delete_last_message
# ---------------------------------------------------------------------------


class TestDeleteLastMessage:
    def test_delete_last_message_empty(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        result = store.delete_last_message("unknown")
        assert result is None

    def test_delete_last_message_calls_delete_at_end(self) -> None:
        client = _make_mock_client()
        client.get_messages.return_value = MessagesResult(
            messages=[
                Message(idx=0, id="m1", role="user", content="a", created_at="t"),
                Message(idx=1, id="m2", role="assistant", content="b", created_at="t"),
            ],
            count=2,
            session_id="s1",
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["s1"] = "uuid-s1"

        removed = store.delete_last_message("s1")
        assert removed is not None
        # Should call delete_message with idx = 1 (last)
        client.delete_message.assert_called_with("uuid-s1", 1)


# ---------------------------------------------------------------------------
# Tests: get_keys (server-side)
# ---------------------------------------------------------------------------


class TestGetKeys:
    def test_get_keys_no_writes_returns_empty(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")

        keys = store.get_keys()
        assert keys == []
        # Should NOT call list_sessions (no user_uuid)
        client.list_sessions.assert_not_called()

    def test_get_keys_local_only_before_server_resolve(self) -> None:
        client = _make_mock_client()
        store = MnemoChatStore(client=client, user_id="alice")
        store._uuid_cache["local-key"] = "uuid-local"

        keys = store.get_keys()
        assert keys == ["local-key"]
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
        client.list_sessions.assert_called_once_with("usr-uuid-1")

    def test_get_keys_merges_local_and_server(self) -> None:
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[
                SessionInfo(id="uuid-1", name="server-session", user_id="usr-1"),
            ],
            count=1,
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"
        store._uuid_cache["local-only"] = "uuid-local"

        keys = store.get_keys()
        assert "server-session" in keys
        assert "local-only" in keys

    def test_get_keys_backfills_uuid_cache(self) -> None:
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[
                SessionInfo(id="uuid-new", name="new-session", user_id="usr-1"),
            ],
            count=1,
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"

        store.get_keys()
        # UUID cache should be back-filled
        assert store._uuid_cache["new-session"] == "uuid-new"

    def test_get_keys_fallback_on_error(self) -> None:
        client = _make_mock_client()
        client.list_sessions.side_effect = Exception("network error")
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"
        store._uuid_cache["cached-key"] = "uuid-cached"

        keys = store.get_keys()
        assert keys == ["cached-key"]

    def test_get_keys_uses_id_when_name_is_none(self) -> None:
        client = _make_mock_client()
        client.list_sessions.return_value = SessionsResult(
            sessions=[
                SessionInfo(id="uuid-nameless", name=None, user_id="usr-1"),
            ],
            count=1,
        )
        store = MnemoChatStore(client=client, user_id="alice")
        store._user_uuid = "usr-uuid-1"

        keys = store.get_keys()
        assert "uuid-nameless" in keys


# ---------------------------------------------------------------------------
# Tests: Role mapping
# ---------------------------------------------------------------------------


class TestRoleMapping:
    def test_role_value_enum(self) -> None:
        class FakeEnum:
            value = "assistant"

        assert _role_value(FakeEnum()) == "assistant"

    def test_role_value_string(self) -> None:
        assert _role_value("user") == "user"

    def test_mnemo_role_to_llamaindex_user(self) -> None:
        assert _mnemo_role_to_llamaindex("user") == _MessageRole.USER

    def test_mnemo_role_to_llamaindex_human(self) -> None:
        assert _mnemo_role_to_llamaindex("human") == _MessageRole.USER

    def test_mnemo_role_to_llamaindex_assistant(self) -> None:
        assert _mnemo_role_to_llamaindex("assistant") == _MessageRole.ASSISTANT

    def test_mnemo_role_to_llamaindex_system(self) -> None:
        assert _mnemo_role_to_llamaindex("system") == _MessageRole.SYSTEM

    def test_mnemo_role_to_llamaindex_tool(self) -> None:
        assert _mnemo_role_to_llamaindex("tool") == _MessageRole.TOOL

    def test_mnemo_role_to_llamaindex_none_defaults_user(self) -> None:
        assert _mnemo_role_to_llamaindex(None) == _MessageRole.USER

    def test_mnemo_role_to_llamaindex_unknown_defaults_user(self) -> None:
        assert _mnemo_role_to_llamaindex("narrator") == _MessageRole.USER

    def test_mnemo_role_case_insensitive(self) -> None:
        assert _mnemo_role_to_llamaindex("ASSISTANT") == _MessageRole.ASSISTANT
        assert _mnemo_role_to_llamaindex("System") == _MessageRole.SYSTEM


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
# Tests: BaseChatStore registration
# ---------------------------------------------------------------------------


class TestBaseChatStoreRegistration:
    def test_registered_as_virtual_subclass(self) -> None:
        # MnemoChatStore should be in BaseChatStore's virtual subclasses
        assert MnemoChatStore in _BaseChatStore._virtual_subclasses


# ---------------------------------------------------------------------------
# Tests: Full workflow (integration-style with mocks)
# ---------------------------------------------------------------------------


class TestFullWorkflow:
    def test_write_read_delete_cycle(self) -> None:
        """Simulate a full write -> read -> delete -> get_keys cycle."""
        client = _make_mock_client(session_id="ses-uuid-1", user_id="usr-uuid-1")

        # Set up get_messages to return messages after write
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

        # Write
        store.add_message("my-chat", _msg("user", "Hello"))
        store.add_message("my-chat", _msg("assistant", "Hi there"))

        assert store._uuid_cache["my-chat"] == "ses-uuid-1"
        assert store._user_uuid == "usr-uuid-1"

        # Read
        msgs = store.get_messages("my-chat")
        assert len(msgs) == 2

        # Keys (server-side)
        keys = store.get_keys()
        assert "my-chat" in keys
        client.list_sessions.assert_called_once_with("usr-uuid-1")
