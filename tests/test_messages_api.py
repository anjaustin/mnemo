#!/usr/bin/env python3
"""
Falsification test for the Session Messages API.

Endpoints tested:
  GET    /api/v1/sessions/:session_id/messages
  DELETE /api/v1/sessions/:session_id/messages
  DELETE /api/v1/sessions/:session_id/messages/:idx

These endpoints are required by the LangChain MnemoChatMessageHistory
and LlamaIndex MnemoChatStore SDK adapters.
"""

import json
import os
import sys
import time
import urllib.request
import urllib.error

ENDPOINT = os.environ.get("MNEMO_ENDPOINT", "http://localhost:8080")

passed = 0
failed = 0


def check(condition: bool, message: str):
    global passed, failed
    if condition:
        print(f"  PASS: {message}")
        passed += 1
    else:
        print(f"  FAIL: {message}")
        failed += 1


def api(method: str, path: str, body=None):
    url = f"{ENDPOINT}{path}"
    data = json.dumps(body).encode() if body else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        err_body = e.read().decode() if e.fp else ""
        raise RuntimeError(f"HTTP {e.code} on {method} {path}: {err_body}")


def main():
    print(f"\nSession Messages API Falsification Test")
    print(f"Endpoint: {ENDPOINT}\n")

    # ─── Setup: create user + session ─────────────────────────
    print("Setup: create user and session")
    user = api(
        "POST",
        "/api/v1/users",
        {"name": "msg_test_user", "external_id": f"msg_test_{int(time.time())}"},
    )
    user_id = user["id"]

    session = api("POST", "/api/v1/sessions", {"user_id": user_id, "session_id": None})
    session_id = session["id"]
    print(f"  user_id={user_id}, session_id={session_id}")

    # ─── 1. Empty session returns empty messages ────────────────
    print("\n1. Empty session messages")
    msgs = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(msgs["messages"] == [], "empty session returns empty messages list")
    check(msgs["count"] == 0, "empty session count is 0")
    check(msgs["session_id"] == session_id, "session_id echoed correctly")

    # ─── 2. Add episodes to session ────────────────────────────
    print("\n2. Add episodes (human, AI, human, AI)")
    api(
        "POST",
        f"/api/v1/sessions/{session_id}/episodes",
        {
            "type": "message",
            "role": "user",
            "content": "Hello, I need help with Python.",
        },
    )
    time.sleep(0.05)
    api(
        "POST",
        f"/api/v1/sessions/{session_id}/episodes",
        {
            "type": "message",
            "role": "assistant",
            "content": "Of course! What specifically about Python?",
        },
    )
    time.sleep(0.05)
    api(
        "POST",
        f"/api/v1/sessions/{session_id}/episodes",
        {"type": "message", "role": "user", "content": "How do I use decorators?"},
    )
    time.sleep(0.05)
    api(
        "POST",
        f"/api/v1/sessions/{session_id}/episodes",
        {
            "type": "message",
            "role": "assistant",
            "content": "Decorators are functions that wrap other functions...",
        },
    )

    # ─── 3. GET messages — chronological order ─────────────────
    print("\n3. GET messages (chronological order)")
    msgs = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(msgs["count"] == 4, f"count is 4 (got {msgs['count']})")
    check(len(msgs["messages"]) == 4, f"messages list has 4 items")
    check(msgs["messages"][0]["idx"] == 0, "first message has idx=0")
    check(msgs["messages"][3]["idx"] == 3, "last message has idx=3")
    check(
        msgs["messages"][0]["role"] == "user",
        f"first message role is user (got {msgs['messages'][0]['role']})",
    )
    check(
        msgs["messages"][1]["role"] == "assistant", "second message role is assistant"
    )
    check(
        "Hello, I need help with Python." in msgs["messages"][0]["content"],
        "first message content correct",
    )
    check(
        "Decorators are functions" in msgs["messages"][3]["content"],
        "last message content correct",
    )
    # Verify chronological ordering (each created_at >= previous)
    times = [m["created_at"] for m in msgs["messages"]]
    check(times == sorted(times), "messages in chronological order")
    # Verify IDs are UUIDs
    check(len(msgs["messages"][0]["id"]) == 36, "message id is UUID")

    # ─── 4. DELETE message by index ────────────────────────────
    print("\n4. DELETE message by index (remove idx=1, the AI response)")
    del_result = api("DELETE", f"/api/v1/sessions/{session_id}/messages/1")
    check(del_result["deleted"] is True, "delete by idx returns deleted:true")

    msgs_after = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(
        msgs_after["count"] == 3, f"count is 3 after delete (got {msgs_after['count']})"
    )
    check(
        msgs_after["messages"][0]["content"] == "Hello, I need help with Python.",
        "first message unchanged",
    )
    check(
        msgs_after["messages"][1]["content"] == "How do I use decorators?",
        "second message is now the third original (AI response removed)",
    )

    # ─── 5. DELETE by index — out of bounds ────────────────────
    print("\n5. DELETE out-of-bounds index returns 400")
    try:
        api("DELETE", f"/api/v1/sessions/{session_id}/messages/99")
        check(False, "out-of-bounds delete should have returned error")
    except RuntimeError as e:
        check(
            "400" in str(e)
            or "out of range" in str(e).lower()
            or "validation" in str(e).lower(),
            "out-of-bounds delete returns 400",
        )

    # ─── 6. DELETE first message (idx=0) ──────────────────────
    print("\n6. DELETE first message (idx=0)")
    api("DELETE", f"/api/v1/sessions/{session_id}/messages/0")
    msgs_after2 = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(msgs_after2["count"] == 2, f"count is 2 (got {msgs_after2['count']})")
    check(
        "How do I use decorators?" in msgs_after2["messages"][0]["content"],
        "new first message is correct",
    )

    # ─── 7. DELETE all messages (clear) ────────────────────────
    print("\n7. DELETE all messages (clear session)")
    clear_result = api("DELETE", f"/api/v1/sessions/{session_id}/messages")
    check(
        clear_result["deleted"] == 2,
        f"cleared 2 messages (got {clear_result['deleted']})",
    )
    check(clear_result["session_id"] == session_id, "session_id echoed")

    msgs_empty = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(msgs_empty["count"] == 0, "messages empty after clear")
    check(msgs_empty["messages"] == [], "messages list empty after clear")

    # ─── 8. Session still exists after clear ──────────────────
    print("\n8. Session still exists after message clear")
    session_check = api("GET", f"/api/v1/sessions/{session_id}")
    check(session_check["id"] == session_id, "session still exists")

    # ─── 9. Can add new messages after clear ──────────────────
    print("\n9. Can add messages after clear")
    api(
        "POST",
        f"/api/v1/sessions/{session_id}/episodes",
        {"type": "message", "role": "user", "content": "New conversation start"},
    )
    msgs_new = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(msgs_new["count"] == 1, "new message added after clear")
    check(
        msgs_new["messages"][0]["content"] == "New conversation start",
        "new message content correct",
    )

    # ─── 10. Idempotent clear on empty session ─────────────────
    print("\n10. Idempotent clear on already-empty session")
    api("DELETE", f"/api/v1/sessions/{session_id}/messages")  # clear it
    clear2 = api("DELETE", f"/api/v1/sessions/{session_id}/messages")  # clear again
    check(clear2["deleted"] == 0, "clearing already-empty session returns 0")

    # ─── 11. Pagination (limit param) ──────────────────────────
    print("\n11. Pagination")
    # Add 5 messages
    for i in range(5):
        api(
            "POST",
            f"/api/v1/sessions/{session_id}/episodes",
            {"type": "message", "role": "user", "content": f"Message {i}"},
        )
        time.sleep(0.02)

    page1 = api("GET", f"/api/v1/sessions/{session_id}/messages?limit=3")
    check(
        page1["count"] <= 3,
        f"limit=3 returns at most 3 messages (got {page1['count']})",
    )

    # ─── 12. Non-message episodes (text/json) also appear ─────
    print("\n12. Text episode type in messages")
    api("DELETE", f"/api/v1/sessions/{session_id}/messages")  # clear
    api(
        "POST",
        f"/api/v1/sessions/{session_id}/episodes",
        {"type": "text", "content": "This is a text document about Python decorators."},
    )
    msgs_text = api("GET", f"/api/v1/sessions/{session_id}/messages")
    check(msgs_text["count"] == 1, "text episode appears in messages")
    check(msgs_text["messages"][0]["role"] is None, "text episode has null role")

    # ─── Cleanup ───────────────────────────────────────────────
    api("DELETE", f"/api/v1/users/{user_id}")

    # ─── Summary ───────────────────────────────────────────────
    print(f"\n{'=' * 50}")
    print(f"Results: {passed} passed, {failed} failed")
    print(f"{'=' * 50}\n")
    sys.exit(1 if failed > 0 else 0)


if __name__ == "__main__":
    main()
