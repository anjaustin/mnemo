"""SDK falsification test suite.

Runs against a live Mnemo server (default: http://localhost:8080).
All assertions are direct and intentionally adversarial — designed to fail
if the server or SDK regresses.

Run:
    python tests/test_sdk.py
    # or
    pytest tests/test_sdk.py -v
"""

import sys
import os
import time
import uuid

# Allow running directly (python tests/test_sdk.py) from sdk/python/ dir
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from mnemo import Mnemo
from mnemo._models import (
    RememberResult,
    ContextResult,
    HealthResult,
    MessagesResult,
    DeleteResult,
    Message,
)
from mnemo._errors import MnemoError, MnemoNotFoundError

BASE_URL = os.environ.get("MNEMO_BASE_URL", "http://localhost:8080")
PASS = 0
FAIL = 0


def check(label: str, condition: bool, detail: str = "") -> None:
    global PASS, FAIL
    if condition:
        print(f"  PASS  {label}")
        PASS += 1
    else:
        print(f"  FAIL  {label}" + (f" — {detail}" if detail else ""))
        FAIL += 1


def uid() -> str:
    """Return a random UUID string (server requires UUID-format IDs)."""
    return str(uuid.uuid4())


def section(name: str) -> None:
    print(f"\n{'=' * 60}")
    print(f"  {name}")
    print(f"{'=' * 60}")


# ── Health ──────────────────────────────────────────────────────────


def test_health(client: Mnemo) -> None:
    section("Health")
    result = client.health()
    check("health() returns HealthResult", isinstance(result, HealthResult))
    check("health status is 'ok'", result.status == "ok", f"got {result.status!r}")
    check("health version is non-empty", bool(result.version))


# ── Memory ──────────────────────────────────────────────────────────


def test_memory(client: Mnemo) -> None:
    section("Memory — add() and context()")
    user = uid()

    result = client.add(user, "I love hiking in Colorado and skiing in Utah.")
    check("add() returns RememberResult", isinstance(result, RememberResult))
    check("add().ok is True", result.ok is True)
    check("add().user_id is non-empty", bool(result.user_id))
    check("add().session_id is non-empty", bool(result.session_id))
    check("add().episode_id is non-empty", bool(result.episode_id))

    # Add a second message
    client.add(user, "My favourite mountain is Pikes Peak.", session=result.session_id)

    ctx = client.context(user, "What does this user enjoy outdoors?")
    check("context() returns ContextResult", isinstance(ctx, ContextResult))
    check("context().text is non-empty", bool(ctx.text))
    check("context().token_count > 0", ctx.token_count > 0, f"got {ctx.token_count}")
    check("context().mode is set", bool(ctx.mode))

    # With role kwarg
    r2 = client.add(user, "The assistant said: great choice!", role="assistant")
    check("add(role='assistant') works", r2.ok is True)


# ── Session messages ────────────────────────────────────────────────


def test_session_messages(client: Mnemo) -> None:
    section("Session messages — get/clear/delete")
    user = uid()

    # The 'session' param in add() is a NAME string (not UUID).
    # The server returns the UUID session_id in the response.
    session_name = f"test-ses-{uuid.uuid4().hex[:8]}"
    r1 = client.add(user, "First message", session=session_name, role="user")
    session = r1.session_id  # server-assigned UUID, used for /sessions/:id/messages
    client.add(user, "Second message", session=session_name, role="assistant")
    client.add(user, "Third message", session=session_name, role="user")

    # get_messages
    result = client.get_messages(session)
    check("get_messages() returns MessagesResult", isinstance(result, MessagesResult))
    check("get_messages().count == 3", result.count == 3, f"got {result.count}")
    check(
        "get_messages() returns list of Message",
        all(isinstance(m, Message) for m in result.messages),
    )
    check(
        "messages are in chronological order",
        result.messages[0].content == "First message"
        and result.messages[1].content == "Second message"
        and result.messages[2].content == "Third message",
        f"order: {[m.content for m in result.messages]}",
    )
    check(
        "message roles preserved",
        result.messages[0].role == "user" and result.messages[1].role == "assistant",
        f"roles: {[m.role for m in result.messages]}",
    )
    check(
        "messages have idx fields (0-based)",
        result.messages[0].idx == 0
        and result.messages[1].idx == 1
        and result.messages[2].idx == 2,
        f"idxs: {[m.idx for m in result.messages]}",
    )

    # delete_message by index — delete middle (idx=1)
    del_result = client.delete_message(session, 1)
    check("delete_message() returns DeleteResult", isinstance(del_result, DeleteResult))

    after_delete = client.get_messages(session)
    check(
        "after delete_message(1), count is 2",
        after_delete.count == 2,
        f"got {after_delete.count}",
    )
    check(
        "remaining messages are First and Third",
        after_delete.messages[0].content == "First message"
        and after_delete.messages[1].content == "Third message",
        f"got {[m.content for m in after_delete.messages]}",
    )

    # clear_messages
    clear_result = client.clear_messages(session)
    check(
        "clear_messages() returns DeleteResult", isinstance(clear_result, DeleteResult)
    )

    after_clear = client.get_messages(session)
    check(
        "after clear_messages(), count is 0",
        after_clear.count == 0,
        f"got {after_clear.count}",
    )
    check(
        "after clear_messages(), messages list is empty",
        len(after_clear.messages) == 0,
    )


# ── LangChain adapter (no langchain_core installed — structural test) ─


def test_langchain_adapter_structure() -> None:
    section("LangChain adapter — import and structure")
    try:
        from mnemo.ext.langchain import (
            MnemoChatMessageHistory,
            _mnemo_to_langchain,
            _langchain_role,
        )

        check("MnemoChatMessageHistory importable", True)
        check("_mnemo_to_langchain importable", True)
        check("_langchain_role importable", True)
        check(
            "MnemoChatMessageHistory has .messages property",
            hasattr(MnemoChatMessageHistory, "messages"),
        )
        check(
            "MnemoChatMessageHistory has .add_messages",
            hasattr(MnemoChatMessageHistory, "add_messages"),
        )
        check(
            "MnemoChatMessageHistory has .clear",
            hasattr(MnemoChatMessageHistory, "clear"),
        )
        check(
            "MnemoChatMessageHistory has .add_user_message",
            hasattr(MnemoChatMessageHistory, "add_user_message"),
        )
        check(
            "MnemoChatMessageHistory has .add_ai_message",
            hasattr(MnemoChatMessageHistory, "add_ai_message"),
        )
        # Verify constructor parameter names
        import inspect

        sig = inspect.signature(MnemoChatMessageHistory.__init__)
        check(
            "MnemoChatMessageHistory accepts session_name param",
            "session_name" in sig.parameters,
        )
        check(
            "MnemoChatMessageHistory accepts user_id param",
            "user_id" in sig.parameters,
        )
    except Exception as e:
        check("langchain adapter import succeeded", False, str(e))


# ── LlamaIndex adapter (no llama_index installed — structural test) ───


def test_llamaindex_adapter_structure() -> None:
    section("LlamaIndex adapter — import and structure")
    try:
        from mnemo.ext.llamaindex import (
            MnemoChatStore,
            _mnemo_role_to_llamaindex,
            _mnemo_to_llamaindex,
        )

        check("MnemoChatStore importable", True)
        check("_mnemo_role_to_llamaindex importable", True)
        check("_mnemo_to_llamaindex importable", True)
        check(
            "MnemoChatStore has .set_messages",
            hasattr(MnemoChatStore, "set_messages"),
        )
        check(
            "MnemoChatStore has .get_messages",
            hasattr(MnemoChatStore, "get_messages"),
        )
        check(
            "MnemoChatStore has .add_message",
            hasattr(MnemoChatStore, "add_message"),
        )
        check(
            "MnemoChatStore has .delete_messages",
            hasattr(MnemoChatStore, "delete_messages"),
        )
        check(
            "MnemoChatStore has .delete_message",
            hasattr(MnemoChatStore, "delete_message"),
        )
        check(
            "MnemoChatStore has .delete_last_message",
            hasattr(MnemoChatStore, "delete_last_message"),
        )
        check(
            "MnemoChatStore has .get_keys",
            hasattr(MnemoChatStore, "get_keys"),
        )
    except Exception as e:
        check("llamaindex adapter import succeeded", False, str(e))


# ── LangChain adapter — functional (requires langchain_core) ─────────


def test_langchain_adapter_functional(client: Mnemo) -> None:
    section("LangChain adapter — functional (requires langchain-core)")
    try:
        from langchain_core.messages import HumanMessage, AIMessage, SystemMessage
        from mnemo.ext.langchain import (
            MnemoChatMessageHistory,
            _mnemo_to_langchain,
            _langchain_role,
        )
    except ImportError:
        print("  SKIP  langchain_core not installed — skipping functional tests")
        return

    user = uid()
    session_name = f"lc-{uuid.uuid4().hex[:8]}"
    history = MnemoChatMessageHistory(
        session_name=session_name, user_id=user, client=client
    )

    # add_messages
    history.add_messages(
        [
            HumanMessage(content="Hello from LangChain"),
            AIMessage(content="Hello back from AI"),
            SystemMessage(content="System directive"),
        ]
    )

    msgs = history.messages
    check("messages property returns list", isinstance(msgs, list))
    check("messages count is 3", len(msgs) == 3, f"got {len(msgs)}")
    check("first message is HumanMessage", type(msgs[0]).__name__ == "HumanMessage")
    check("second message is AIMessage", type(msgs[1]).__name__ == "AIMessage")

    # add_user_message convenience
    history.add_user_message("Quick user message")
    msgs2 = history.messages
    check("add_user_message appends correctly", len(msgs2) == 4, f"got {len(msgs2)}")

    # clear
    history.clear()
    msgs3 = history.messages
    check("after clear(), messages is empty", len(msgs3) == 0, f"got {len(msgs3)}")

    # Role mapping unit tests
    check(
        "_langchain_role(HumanMessage) == 'user'",
        _langchain_role(HumanMessage(content="x")) == "user",
    )
    check(
        "_langchain_role(AIMessage) == 'assistant'",
        _langchain_role(AIMessage(content="x")) == "assistant",
    )
    check(
        "_langchain_role(SystemMessage) == 'system'",
        _langchain_role(SystemMessage(content="x")) == "system",
    )


# ── LlamaIndex adapter — functional (requires llama_index) ────────────


def test_llamaindex_adapter_functional(client: Mnemo) -> None:
    section("LlamaIndex adapter — functional (requires llama-index-core)")
    try:
        from llama_index.core.llms import ChatMessage, MessageRole
        from mnemo.ext.llamaindex import MnemoChatStore, _mnemo_role_to_llamaindex
    except ImportError:
        print("  SKIP  llama_index not installed — skipping functional tests")
        return

    user = uid()
    store = MnemoChatStore(client=client, user_id=user)
    session = f"li-{uuid.uuid4().hex[:8]}"  # session name (key)

    # set_messages
    msgs_in = [
        ChatMessage(role=MessageRole.USER, content="LlamaIndex user message"),
        ChatMessage(role=MessageRole.ASSISTANT, content="LlamaIndex AI response"),
        ChatMessage(role=MessageRole.USER, content="Follow-up question"),
    ]
    store.set_messages(session, msgs_in)

    # get_messages
    msgs_out = store.get_messages(session)
    check(
        "get_messages() returns 3 messages", len(msgs_out) == 3, f"got {len(msgs_out)}"
    )
    check(
        "message content preserved",
        msgs_out[0].content == "LlamaIndex user message",
        f"got {msgs_out[0].content!r}",
    )

    # add_message
    store.add_message(
        session, ChatMessage(role=MessageRole.USER, content="Extra message")
    )
    msgs_after_add = store.get_messages(session)
    check(
        "add_message() appends", len(msgs_after_add) == 4, f"got {len(msgs_after_add)}"
    )

    # delete_message at index 1
    removed = store.delete_message(session, 1)
    check("delete_message returns ChatMessage", removed is not None)
    check(
        "delete_message returns correct message",
        removed.content == "LlamaIndex AI response",
        f"got {removed.content!r}",
    )
    msgs_after_del = store.get_messages(session)
    check(
        "after delete_message(1), count is 3",
        len(msgs_after_del) == 3,
        f"got {len(msgs_after_del)}",
    )

    # delete_last_message
    last = store.delete_last_message(session)
    check("delete_last_message returns ChatMessage", last is not None)
    msgs_after_last = store.get_messages(session)
    check(
        "after delete_last_message, count is 2",
        len(msgs_after_last) == 2,
        f"got {len(msgs_after_last)}",
    )

    # delete_messages (clear)
    cleared = store.delete_messages(session)
    check("delete_messages returns list", isinstance(cleared, list))
    msgs_after_clear = store.get_messages(session)
    check(
        "after delete_messages, session is empty",
        len(msgs_after_clear) == 0,
        f"got {len(msgs_after_clear)}",
    )

    # delete_message out of bounds returns None
    result_oob = store.delete_message(session, 99)
    check("delete_message out-of-bounds returns None", result_oob is None)

    # Role mapping
    check(
        "_mnemo_role_to_llamaindex('user') == MessageRole.USER",
        _mnemo_role_to_llamaindex("user") == MessageRole.USER,
    )
    check(
        "_mnemo_role_to_llamaindex('assistant') == MessageRole.ASSISTANT",
        _mnemo_role_to_llamaindex("assistant") == MessageRole.ASSISTANT,
    )
    check(
        "_mnemo_role_to_llamaindex('system') == MessageRole.SYSTEM",
        _mnemo_role_to_llamaindex("system") == MessageRole.SYSTEM,
    )


# ── Error handling ───────────────────────────────────────────────────


def test_error_handling(client: Mnemo) -> None:
    section("Error handling")
    # get_messages on a UUID session with no episodes should return empty, not error
    result = client.get_messages(str(uuid.uuid4()))
    check(
        "get_messages() on empty session returns MessagesResult",
        isinstance(result, MessagesResult),
    )
    check(
        "get_messages() on empty session count is 0",
        result.count == 0,
        f"got {result.count}",
    )

    # delete_message out of bounds on empty session raises MnemoValidationError
    from mnemo._errors import MnemoValidationError

    raised = False
    try:
        client.delete_message(str(uuid.uuid4()), 0)
    except MnemoValidationError:
        raised = True
    check(
        "delete_message() on empty session raises MnemoValidationError",
        raised,
    )


# ── __init__.py exports ──────────────────────────────────────────────


def test_package_exports() -> None:
    section("Package exports — mnemo.__init__")
    import mnemo

    expected = [
        "Mnemo",
        "AsyncMnemo",
        "MnemoError",
        "MnemoConnectionError",
        "MnemoTimeoutError",
        "MnemoHttpError",
        "MnemoRateLimitError",
        "MnemoNotFoundError",
        "MnemoValidationError",
        "RememberResult",
        "ContextResult",
        "ChangesSinceResult",
        "ContextResult",
        "MessagesResult",
        "DeleteResult",
        "HealthResult",
        "Message",
    ]
    for name in expected:
        check(f"mnemo.{name} exported", hasattr(mnemo, name))


# ── Main ────────────────────────────────────────────────────────────


def main() -> int:
    print(f"\nMnemo SDK falsification test suite")
    print(f"Server: {BASE_URL}")
    print(f"Time:   {time.strftime('%Y-%m-%d %H:%M:%S')}")

    client = Mnemo(BASE_URL)

    test_health(client)
    test_memory(client)
    test_session_messages(client)
    test_langchain_adapter_structure()
    test_llamaindex_adapter_structure()
    test_langchain_adapter_functional(client)
    test_llamaindex_adapter_functional(client)
    test_error_handling(client)
    test_package_exports()

    print(f"\n{'=' * 60}")
    print(f"  RESULTS: {PASS} passed, {FAIL} failed")
    print(f"{'=' * 60}\n")

    return 0 if FAIL == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
