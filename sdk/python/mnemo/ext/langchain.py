"""LangChain adapter for Mnemo.

Provides a drop-in ``BaseChatMessageHistory`` implementation backed by the
Mnemo memory API.

Install:
    pip install mnemo-client[langchain]

Usage:
    from mnemo import Mnemo
    from mnemo.ext.langchain import MnemoChatMessageHistory

    client = Mnemo("http://localhost:8080")
    history = MnemoChatMessageHistory(
        session_name="my-chat-session",
        user_id="kendra",
        client=client,
    )
    # Wire into LangChain
    from langchain_core.runnables.history import RunnableWithMessageHistory
    chain_with_history = RunnableWithMessageHistory(
        chain,
        lambda session_id: MnemoChatMessageHistory(session_id, "kendra", client),
        input_messages_key="input",
        history_messages_key="history",
    )

Notes:
    The Mnemo API separates *session names* (used when writing via ``add()``)
    from *session UUIDs* (used when reading via ``get_messages()``). This
    adapter tracks the UUID automatically after the first write and caches it
    for subsequent reads.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Sequence

if TYPE_CHECKING:
    from mnemo.client import Mnemo
    from mnemo._models import Message


def _require_langchain() -> None:
    try:
        import langchain_core  # noqa: F401
    except ImportError as e:
        raise ImportError(
            "MnemoChatMessageHistory requires langchain-core. "
            "Install it with: pip install mnemo-client[langchain]"
        ) from e


def _mnemo_to_langchain(msg: "Message") -> "BaseMessage":
    """Convert a Mnemo Message to a LangChain BaseMessage."""
    from langchain_core.messages import (
        AIMessage,
        HumanMessage,
        SystemMessage,
        ChatMessage,
    )

    role = (msg.role or "user").lower()
    content = msg.content

    if role in ("user", "human"):
        return HumanMessage(content=content)
    elif role in ("assistant", "ai", "bot"):
        return AIMessage(content=content)
    elif role == "system":
        return SystemMessage(content=content)
    else:
        # Preserve unknown roles via generic ChatMessage
        return ChatMessage(role=role, content=content)


def _langchain_role(msg: "BaseMessage") -> str:
    """Map a LangChain message type to a Mnemo role string."""
    from langchain_core.messages import (
        AIMessage,
        HumanMessage,
        SystemMessage,
        FunctionMessage,
        ToolMessage,
        ChatMessage,
    )

    if isinstance(msg, HumanMessage):
        return "user"
    elif isinstance(msg, AIMessage):
        return "assistant"
    elif isinstance(msg, SystemMessage):
        return "system"
    elif isinstance(msg, (FunctionMessage, ToolMessage)):
        return "tool"
    elif isinstance(msg, ChatMessage):
        return msg.role
    else:
        return "user"


class MnemoChatMessageHistory:
    """Mnemo-backed chat message history for LangChain.

    Implements ``langchain_core.chat_history.BaseChatMessageHistory``.
    This class lazy-imports LangChain so the rest of the Mnemo SDK stays
    zero-dependency.

    Args:
        session_name: A human-readable name for the session (e.g.
            ``"user-alice-chat-1"``). Used as the session label when writing
            new messages via ``add()``. The server returns a UUID for the
            session which is cached and used for subsequent reads via
            ``get_messages()``.
        user_id: Mnemo user identifier. Messages are stored under this user.
        client: An initialised :class:`mnemo.Mnemo` sync client instance.
        session_uuid: Optionally supply a known server session UUID directly
            (e.g. from a prior run). When provided, read operations start
            immediately without waiting for a write.

    Example:
        from mnemo import Mnemo
        from mnemo.ext.langchain import MnemoChatMessageHistory

        client = Mnemo("http://localhost:8080")
        history = MnemoChatMessageHistory("ses-1", "alice", client)
        history.add_user_message("Hello!")
        print(history.messages)  # [HumanMessage(content='Hello!')]
    """

    def __init__(
        self,
        session_name: str,
        user_id: str,
        client: "Mnemo",
        *,
        session_uuid: str | None = None,
    ) -> None:
        _require_langchain()
        self.session_name = session_name
        self.user_id = user_id
        self._client = client
        # The UUID is resolved lazily on first write (or immediately if supplied)
        self._session_uuid: str | None = session_uuid

    # ------------------------------------------------------------------
    # BaseChatMessageHistory interface
    # ------------------------------------------------------------------

    @property
    def messages(self) -> list:
        """Fetch all messages for this session from Mnemo (chronological)."""
        if not self._session_uuid:
            return []
        result = self._client.get_messages(self._session_uuid)
        return [_mnemo_to_langchain(m) for m in result.messages]

    def add_messages(self, messages: Sequence) -> None:
        """Persist a sequence of LangChain messages into Mnemo."""
        for msg in messages:
            role = _langchain_role(msg)
            # Extract text content; handle list-of-dicts (multimodal) gracefully
            content = msg.content
            if isinstance(content, list):
                # Flatten: take text parts only
                parts = [
                    p.get("text", "") if isinstance(p, dict) else str(p)
                    for p in content
                ]
                content = " ".join(p for p in parts if p)
            result = self._client.add(
                self.user_id,
                str(content),
                session=self.session_name,
                role=role,
            )
            # Cache the UUID from the first successful write
            if self._session_uuid is None:
                self._session_uuid = result.session_id

    def add_user_message(self, message: str) -> None:
        """Convenience: add a user message."""
        from langchain_core.messages import HumanMessage

        self.add_messages([HumanMessage(content=message)])

    def add_ai_message(self, message: str) -> None:
        """Convenience: add an AI message."""
        from langchain_core.messages import AIMessage

        self.add_messages([AIMessage(content=message)])

    def clear(self) -> None:
        """Clear all messages for this session from Mnemo."""
        if self._session_uuid:
            self._client.clear_messages(self._session_uuid)

    # ------------------------------------------------------------------
    # Async variants (for use with AsyncMnemo + async LangChain)
    # ------------------------------------------------------------------

    async def aget_messages(self) -> list:
        """Async fetch of messages (requires AsyncMnemo client)."""
        if not self._session_uuid:
            return []
        result = await self._client.get_messages(self._session_uuid)
        return [_mnemo_to_langchain(m) for m in result.messages]

    async def aadd_messages(self, messages: Sequence) -> None:
        """Async persist of messages (requires AsyncMnemo client)."""
        for msg in messages:
            role = _langchain_role(msg)
            content = msg.content
            if isinstance(content, list):
                parts = [
                    p.get("text", "") if isinstance(p, dict) else str(p)
                    for p in content
                ]
                content = " ".join(p for p in parts if p)
            result = await self._client.add(
                self.user_id,
                str(content),
                session=self.session_name,
                role=role,
            )
            if self._session_uuid is None:
                self._session_uuid = result.session_id

    async def aclear(self) -> None:
        """Async clear (requires AsyncMnemo client)."""
        if self._session_uuid:
            await self._client.clear_messages(self._session_uuid)
