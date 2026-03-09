"""LlamaIndex adapter for Mnemo.

Provides a drop-in ``BaseChatStore`` implementation backed by the
Mnemo memory API.

Install:
    pip install mnemo-client[llamaindex]

Usage:
    from mnemo import Mnemo
    from mnemo.ext.llamaindex import MnemoChatStore

    client = Mnemo("http://localhost:8080")
    store = MnemoChatStore(client=client, user_id="kendra")

    # Wire into LlamaIndex chat engine
    from llama_index.core.memory import ChatMemoryBuffer
    memory = ChatMemoryBuffer.from_defaults(
        token_limit=3000,
        chat_store=store,
        chat_store_key="my-chat-session",
    )

Notes:
    The Mnemo API separates *session names* (used when writing via ``add()``)
    from *session UUIDs* (used when reading via ``get_messages()``). This
    adapter caches the UUID returned after the first write for each ``key``
    and uses it for subsequent reads.

    When llama-index-core is installed, ``MnemoChatStore`` dynamically
    registers as a virtual subclass of ``BaseChatStore`` so that
    ``isinstance(store, BaseChatStore)`` returns True.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from mnemo.client import Mnemo
    from mnemo._models import Message

logger = logging.getLogger(__name__)


def _require_llamaindex() -> None:
    try:
        import llama_index.core  # noqa: F401
    except ImportError as e:
        raise ImportError(
            "MnemoChatStore requires llama-index-core. "
            "Install it with: pip install mnemo-client[llamaindex]"
        ) from e


def _mnemo_role_to_llamaindex(role: str | None) -> "MessageRole":
    """Map a Mnemo role string to a LlamaIndex MessageRole."""
    from llama_index.core.llms import MessageRole

    role_str = (role or "user").lower()
    mapping = {
        "user": MessageRole.USER,
        "human": MessageRole.USER,
        "assistant": MessageRole.ASSISTANT,
        "ai": MessageRole.ASSISTANT,
        "bot": MessageRole.ASSISTANT,
        "system": MessageRole.SYSTEM,
        "tool": MessageRole.TOOL,
        "function": MessageRole.TOOL,
    }
    return mapping.get(role_str, MessageRole.USER)


def _mnemo_to_llamaindex(msg: "Message") -> "ChatMessage":
    """Convert a Mnemo Message to a LlamaIndex ChatMessage."""
    from llama_index.core.llms import ChatMessage

    return ChatMessage(
        role=_mnemo_role_to_llamaindex(msg.role),
        content=msg.content,
    )


def _role_value(role: object) -> str:
    """Extract role string from a LlamaIndex MessageRole enum or plain string."""
    if hasattr(role, "value"):
        return str(role.value)
    return str(role)


class MnemoChatStore:
    """Mnemo-backed chat store for LlamaIndex.

    Implements ``llama_index.core.storage.chat_store.base.BaseChatStore``.
    When llama-index-core is installed, this class registers as a virtual
    subclass of ``BaseChatStore`` so ``isinstance()`` checks pass.

    Args:
        client: An initialised :class:`mnemo.Mnemo` sync client instance.
        user_id: Mnemo user identifier. Messages are stored under this user.
            The ``key`` arguments in all store methods are used as Mnemo
            session *names*; the server-assigned UUIDs are cached internally.

    Example:
        from mnemo import Mnemo
        from mnemo.ext.llamaindex import MnemoChatStore

        client = Mnemo("http://localhost:8080")
        store = MnemoChatStore(client=client, user_id="alice")
        store.add_message("ses-1", ChatMessage(role=MessageRole.USER, content="Hi"))
        msgs = store.get_messages("ses-1")
    """

    def __init__(self, client: "Mnemo", user_id: str) -> None:
        _require_llamaindex()
        self._client = client
        self.user_id = user_id
        # Maps session name (key) -> server UUID (for reads)
        self._uuid_cache: dict[str, str] = {}
        # Resolved server-side user UUID (set on first successful write)
        self._user_uuid: str | None = None

    def _get_uuid(self, key: str) -> str | None:
        """Return the cached server UUID for a session key, or None."""
        return self._uuid_cache.get(key)

    def _write(self, key: str, content: str, role: str) -> None:
        """Write one message to Mnemo and cache the session UUID."""
        result = self._client.add(
            self.user_id,
            content,
            session=key,
            role=role,
        )
        if key not in self._uuid_cache:
            self._uuid_cache[key] = result.session_id
        if self._user_uuid is None and result.user_id:
            self._user_uuid = result.user_id

    # ------------------------------------------------------------------
    # BaseChatStore interface (all 7 abstract methods)
    # ------------------------------------------------------------------

    def set_messages(self, key: str, messages: list) -> None:
        """Replace all messages for a session.

        Clears existing messages then adds the new ones in order.
        """
        uuid = self._get_uuid(key)
        if uuid:
            self._client.clear_messages(uuid)
        for msg in messages:
            self._write(key, str(msg.content), _role_value(msg.role))

    def get_messages(self, key: str) -> list:
        """Return all messages for a session in chronological order."""
        uuid = self._get_uuid(key)
        if not uuid:
            return []
        result = self._client.get_messages(uuid)
        return [_mnemo_to_llamaindex(m) for m in result.messages]

    def add_message(self, key: str, message, idx: int | None = None) -> None:
        """Append a message to a session.

        Note: ``idx`` is accepted for interface compliance but Mnemo always
        appends; there is no insert-at-index in the current API.
        """
        self._write(key, str(message.content), _role_value(message.role))

    def delete_messages(self, key: str) -> list | None:
        """Clear all messages for a session. Returns the cleared messages."""
        existing = self.get_messages(key)
        uuid = self._get_uuid(key)
        if uuid:
            self._client.clear_messages(uuid)
        return existing if existing else None

    def delete_message(self, key: str, idx: int) -> "ChatMessage | None":
        """Delete a message at ordinal index ``idx`` (0-based).

        Returns the removed message, or None if index is out of bounds.
        """
        existing = self.get_messages(key)
        if idx < 0 or idx >= len(existing):
            return None
        removed = existing[idx]
        uuid = self._get_uuid(key)
        if uuid:
            self._client.delete_message(uuid, idx)
        return removed

    def delete_last_message(self, key: str) -> "ChatMessage | None":
        """Delete the most recent message for a session."""
        existing = self.get_messages(key)
        if not existing:
            return None
        return self.delete_message(key, len(existing) - 1)

    def get_keys(self) -> list[str]:
        """Return all session keys (names) for this user.

        Queries the server for all sessions belonging to the user, merging
        with any locally-cached keys. Falls back to the in-memory cache if
        the server-side user UUID has not yet been resolved (no writes).
        """
        if self._user_uuid is None:
            # No writes yet; return in-memory cache only
            return list(self._uuid_cache.keys())
        try:
            result = self._client.list_sessions(self._user_uuid)
            # Build merged key set: server sessions + any local keys not yet
            # synced (edge case: names written in this instance but not yet
            # reflected in the server list due to eventual consistency)
            server_keys: dict[str, str] = {}
            for s in result.sessions:
                name = s.name or s.id
                server_keys[name] = s.id
                # Back-fill the uuid cache so reads work for sessions
                # discovered server-side
                if name not in self._uuid_cache:
                    self._uuid_cache[name] = s.id
            # Merge: server keys first, then any local-only keys
            merged = list(server_keys.keys())
            for k in self._uuid_cache:
                if k not in server_keys:
                    merged.append(k)
            return merged
        except Exception:
            logger.debug(
                "list_sessions failed, falling back to local cache", exc_info=True
            )
            return list(self._uuid_cache.keys())

    # ------------------------------------------------------------------
    # Async variants (for use with AsyncMnemo)
    # ------------------------------------------------------------------

    async def _awrite(self, key: str, content: str, role: str) -> None:
        result = await self._client.add(
            self.user_id,
            content,
            session=key,
            role=role,
        )
        if key not in self._uuid_cache:
            self._uuid_cache[key] = result.session_id
        if self._user_uuid is None and result.user_id:
            self._user_uuid = result.user_id

    async def aset_messages(self, key: str, messages: list) -> None:
        uuid = self._get_uuid(key)
        if uuid:
            await self._client.clear_messages(uuid)
        for msg in messages:
            await self._awrite(key, str(msg.content), _role_value(msg.role))

    async def aget_messages(self, key: str) -> list:
        uuid = self._get_uuid(key)
        if not uuid:
            return []
        result = await self._client.get_messages(uuid)
        return [_mnemo_to_llamaindex(m) for m in result.messages]

    async def aadd_message(self, key: str, message, idx: int | None = None) -> None:
        await self._awrite(key, str(message.content), _role_value(message.role))

    async def adelete_messages(self, key: str) -> list | None:
        existing = await self.aget_messages(key)
        uuid = self._get_uuid(key)
        if uuid:
            await self._client.clear_messages(uuid)
        return existing if existing else None

    async def adelete_message(self, key: str, idx: int) -> "ChatMessage | None":
        existing = await self.aget_messages(key)
        if idx < 0 or idx >= len(existing):
            return None
        removed = existing[idx]
        uuid = self._get_uuid(key)
        if uuid:
            await self._client.delete_message(uuid, idx)
        return removed

    async def adelete_last_message(self, key: str) -> "ChatMessage | None":
        existing = await self.aget_messages(key)
        if not existing:
            return None
        return await self.adelete_message(key, len(existing) - 1)

    async def aget_keys(self) -> list[str]:
        """Async server-side get_keys."""
        if self._user_uuid is None:
            return list(self._uuid_cache.keys())
        try:
            result = await self._client.list_sessions(self._user_uuid)
            server_keys: dict[str, str] = {}
            for s in result.sessions:
                name = s.name or s.id
                server_keys[name] = s.id
                if name not in self._uuid_cache:
                    self._uuid_cache[name] = s.id
            merged = list(server_keys.keys())
            for k in self._uuid_cache:
                if k not in server_keys:
                    merged.append(k)
            return merged
        except Exception:
            logger.debug(
                "list_sessions failed, falling back to local cache", exc_info=True
            )
            return list(self._uuid_cache.keys())


# ------------------------------------------------------------------
# Runtime BaseChatStore registration
# ------------------------------------------------------------------
# When llama-index-core is installed, register MnemoChatStore as a
# virtual subclass of BaseChatStore so isinstance() checks pass.
# This avoids a hard import dependency while satisfying framework code
# that validates adapters via isinstance().

try:
    from llama_index.core.storage.chat_store.base import BaseChatStore

    BaseChatStore.register(MnemoChatStore)
except Exception:
    # llama-index-core not installed or API changed — graceful degradation
    pass
