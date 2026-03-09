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

    ``MnemoChatStore`` is a plain class (not a Pydantic ``BaseChatStore``
    subclass) because ``BaseChatStore`` uses Pydantic's ``ModelMetaclass``
    which prevents ``isinstance()`` via ``ABC.register()``. LlamaIndex's
    ``ChatMemoryBuffer`` accepts any object with the right methods (duck
    typing), so direct subclassing is unnecessary.
"""

from __future__ import annotations

import asyncio
import logging
from typing import TYPE_CHECKING

from mnemo._errors import MnemoError

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
        return str(role.value).lower()
    return str(role).lower()


def _safe_content(content: object) -> str:
    """Safely extract string content, handling None and list types."""
    if content is None:
        return ""
    if isinstance(content, list):
        # Multimodal content: extract text parts
        parts = [p.get("text", "") if isinstance(p, dict) else str(p) for p in content]
        return " ".join(p for p in parts if p)
    return str(content)


class MnemoChatStore:
    """Mnemo-backed chat store for LlamaIndex.

    Implements the 7 abstract methods of
    ``llama_index.core.storage.chat_store.base.BaseChatStore`` via duck
    typing. LlamaIndex's ``ChatMemoryBuffer`` accepts any object with the
    correct method signatures.

    Note: ``BaseChatStore`` is a Pydantic ``BaseModel`` subclass, so
    ``ABC.register()`` does NOT enable ``isinstance()`` checks. This is a
    known Pydantic limitation. The adapter works via duck typing instead.

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
        # Lock for async dict mutation safety
        self._async_lock = asyncio.Lock()

    def _get_uuid(self, key: str) -> str | None:
        """Return the cached server UUID for a session key, or None."""
        return self._uuid_cache.get(key)

    def _ensure_uuid(self, key: str) -> str | None:
        """Resolve the session UUID by writing a no-op if needed."""
        uuid = self._get_uuid(key)
        if uuid is not None:
            return uuid
        # Try to discover UUID from server-side session list
        if self._user_uuid is not None:
            try:
                result = self._client.list_sessions(self._user_uuid)
                for s in result.sessions:
                    name = s.name or s.id
                    if name not in self._uuid_cache:
                        self._uuid_cache[name] = s.id
                uuid = self._uuid_cache.get(key)
            except Exception:
                pass
        return uuid

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
        uuid = self._ensure_uuid(key)
        if uuid:
            self._client.clear_messages(uuid)
        for msg in messages:
            self._write(key, _safe_content(msg.content), _role_value(msg.role))

    def get_messages(self, key: str) -> list:
        """Return all messages for a session in chronological order."""
        uuid = self._ensure_uuid(key)
        if not uuid:
            return []
        result = self._client.get_messages(uuid)
        return [_mnemo_to_llamaindex(m) for m in result.messages]

    def add_message(self, key: str, message) -> None:
        """Append a message to a session."""
        self._write(key, _safe_content(message.content), _role_value(message.role))

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
        Uses the server-side message ``idx`` field for the delete call to
        handle non-contiguous indices after prior deletions.
        """
        uuid = self._get_uuid(key)
        if not uuid:
            return None
        result = self._client.get_messages(uuid)
        if idx < 0 or idx >= len(result.messages):
            return None
        server_msg = result.messages[idx]
        removed = _mnemo_to_llamaindex(server_msg)
        # Use the server's idx field, not the list position
        self._client.delete_message(uuid, server_msg.idx)
        return removed

    def delete_last_message(self, key: str) -> "ChatMessage | None":
        """Delete the most recent message for a session.

        Fetches messages once and uses the last one's server-side idx to
        avoid the double-fetch race in delete_message.
        """
        uuid = self._get_uuid(key)
        if not uuid:
            return None
        result = self._client.get_messages(uuid)
        if not result.messages:
            return None
        last_msg = result.messages[-1]
        removed = _mnemo_to_llamaindex(last_msg)
        self._client.delete_message(uuid, last_msg.idx)
        return removed

    def get_keys(self) -> list[str]:
        """Return all session keys (names) for this user.

        Queries the server for all sessions belonging to the user, merging
        with any locally-cached keys. Falls back to the in-memory cache if
        the server-side user UUID has not yet been resolved (no writes).
        """
        if self._user_uuid is None:
            return list(self._uuid_cache.keys())
        try:
            result = self._client.list_sessions(self._user_uuid)
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
        except MnemoError:
            logger.warning(
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
        async with self._async_lock:
            if key not in self._uuid_cache:
                self._uuid_cache[key] = result.session_id
            if self._user_uuid is None and result.user_id:
                self._user_uuid = result.user_id

    async def _aensure_uuid(self, key: str) -> str | None:
        """Async variant of _ensure_uuid."""
        uuid = self._uuid_cache.get(key)
        if uuid is not None:
            return uuid
        if self._user_uuid is not None:
            try:
                result = await self._client.list_sessions(self._user_uuid)
                async with self._async_lock:
                    for s in result.sessions:
                        name = s.name or s.id
                        if name not in self._uuid_cache:
                            self._uuid_cache[name] = s.id
                uuid = self._uuid_cache.get(key)
            except Exception:
                pass
        return uuid

    async def aset_messages(self, key: str, messages: list) -> None:
        uuid = await self._aensure_uuid(key)
        if uuid:
            await self._client.clear_messages(uuid)
        for msg in messages:
            await self._awrite(key, _safe_content(msg.content), _role_value(msg.role))

    async def aget_messages(self, key: str) -> list:
        uuid = await self._aensure_uuid(key)
        if not uuid:
            return []
        result = await self._client.get_messages(uuid)
        return [_mnemo_to_llamaindex(m) for m in result.messages]

    async def aadd_message(self, key: str, message) -> None:
        await self._awrite(
            key, _safe_content(message.content), _role_value(message.role)
        )

    # LlamaIndex uses async_add_message as the canonical async name
    async_add_message = aadd_message

    async def adelete_messages(self, key: str) -> list | None:
        existing = await self.aget_messages(key)
        uuid = self._uuid_cache.get(key)
        if uuid:
            await self._client.clear_messages(uuid)
        return existing if existing else None

    async def adelete_message(self, key: str, idx: int) -> "ChatMessage | None":
        uuid = self._uuid_cache.get(key)
        if not uuid:
            return None
        result = await self._client.get_messages(uuid)
        if idx < 0 or idx >= len(result.messages):
            return None
        server_msg = result.messages[idx]
        removed = _mnemo_to_llamaindex(server_msg)
        await self._client.delete_message(uuid, server_msg.idx)
        return removed

    async def adelete_last_message(self, key: str) -> "ChatMessage | None":
        uuid = self._uuid_cache.get(key)
        if not uuid:
            return None
        result = await self._client.get_messages(uuid)
        if not result.messages:
            return None
        last_msg = result.messages[-1]
        removed = _mnemo_to_llamaindex(last_msg)
        await self._client.delete_message(uuid, last_msg.idx)
        return removed

    async def aget_keys(self) -> list[str]:
        """Async server-side get_keys."""
        if self._user_uuid is None:
            return list(self._uuid_cache.keys())
        try:
            result = await self._client.list_sessions(self._user_uuid)
            async with self._async_lock:
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
        except MnemoError:
            logger.warning(
                "list_sessions failed, falling back to local cache", exc_info=True
            )
            return list(self._uuid_cache.keys())
