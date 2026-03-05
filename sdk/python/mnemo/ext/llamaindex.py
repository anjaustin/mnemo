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
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from mnemo.client import Mnemo
    from mnemo._models import Message


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
    This class lazy-imports LlamaIndex so the rest of the Mnemo SDK stays
    zero-dependency.

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
        # Maps session name (key) → server UUID (for reads)
        self._uuid_cache: dict[str, str] = {}

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
        """Return all known session keys (names) for this store instance.

        Returns the keys that have been written to in this store instance.
        Note: keys are only tracked in-memory; they are not persisted to the
        server across restarts.
        """
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
        return list(self._uuid_cache.keys())
