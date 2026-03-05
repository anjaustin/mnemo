#!/usr/bin/env python3
"""
Falsification test for Mnemo's Raw Vector API (the surface that the
AnythingLLM provider talks to).

Prerequisites:
  - Mnemo server running at MNEMO_ENDPOINT (default http://localhost:8080)
  - Qdrant running (Mnemo's backend)

Run:
  python3 integrations/anythingllm/test_api.py
"""

import json
import os
import random
import sys
import time
import urllib.request
import urllib.error

ENDPOINT = os.environ.get("MNEMO_ENDPOINT", "http://localhost:8080")
NAMESPACE = f"__mnemo_test_{int(time.time() * 1000)}"
DIM = 384

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


def random_vector(dim: int):
    return [random.uniform(-1, 1) for _ in range(dim)]


def main():
    print(f"\nMnemo Raw Vector API Falsification Test")
    print(f"Endpoint: {ENDPOINT}")
    print(f"Namespace: {NAMESPACE}\n")

    # ─── 1. Health ──────────────────────────────────────────────
    print("1. Health check")
    health = api("GET", "/health")
    check(health["status"] == "ok", "health status is ok")
    check("version" in health, f"version present: {health.get('version')}")

    # ─── 2. Namespace does not exist initially ──────────────────
    print("\n2. Namespace lifecycle (pre-create)")
    exists = api("GET", f"/api/v1/vectors/{NAMESPACE}/exists")
    check(exists["exists"] is False, "namespace does not exist initially")

    count = api("GET", f"/api/v1/vectors/{NAMESPACE}/count")
    check(count["count"] == 0, "count is 0 for non-existent namespace")

    # ─── 3. Upsert vectors ─────────────────────────────────────
    print("\n3. Upsert vectors")
    vectors = []
    for i in range(10):
        vectors.append(
            {
                "id": f"vec-{i}",
                "vector": random_vector(DIM),
                "metadata": {
                    "text": f"This is test document {i}",
                    "docId": f"doc-{i % 3}",
                    "source": "test",
                },
            }
        )

    result = api("POST", f"/api/v1/vectors/{NAMESPACE}", {"vectors": vectors})
    check(result["ok"] is True, "upsert returns ok:true")
    check(result["upserted"] == 10, f"upserted count is 10 (got {result['upserted']})")
    check(result["namespace"] == NAMESPACE, "namespace echoed back correctly")

    # ─── 4. Namespace exists after upsert ───────────────────────
    print("\n4. Namespace exists after upsert")
    exists = api("GET", f"/api/v1/vectors/{NAMESPACE}/exists")
    check(exists["exists"] is True, "namespace exists after upsert")

    # ─── 5. Count ───────────────────────────────────────────────
    print("\n5. Vector count")
    count = api("GET", f"/api/v1/vectors/{NAMESPACE}/count")
    check(count["count"] == 10, f"count is 10 (got {count['count']})")

    # ─── 6. Similarity search ──────────────────────────────────
    print("\n6. Similarity search")
    # Query with the exact vector of vec-0 (should get near-perfect match)
    query_vector = vectors[0]["vector"]
    search = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/query",
        {
            "vector": query_vector,
            "top_k": 5,
            "min_score": 0.0,
        },
    )
    check(
        len(search["results"]) > 0, f"search returned {len(search['results'])} results"
    )
    check(
        search["results"][0]["score"] > 0.9,
        f"top result score is >0.9 (got {search['results'][0]['score']:.3f})",
    )
    # Verify the top result is vec-0 (exact match should be highest)
    check(
        search["results"][0]["id"] == "vec-0",
        f"top result is vec-0 (got {search['results'][0]['id']})",
    )

    # Verify payload came back
    payload = search["results"][0].get("payload", {})
    check("text" in payload, "payload contains text field")
    check(payload.get("text") == "This is test document 0", "payload text matches")

    # ─── 7. Search with min_score filter ────────────────────────
    print("\n7. Search with min_score filter")
    high_threshold = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/query",
        {
            "vector": random_vector(DIM),
            "top_k": 10,
            "min_score": 0.99,  # Very high threshold — random vector unlikely to match
        },
    )
    check(isinstance(high_threshold["results"], list), "high threshold returns list")
    # Random vector against random vectors at 0.99 threshold should return few/no results
    check(
        len(high_threshold["results"]) <= 2,
        f"high threshold filters effectively ({len(high_threshold['results'])} results)",
    )

    # ─── 8. Search defaults (no min_score, no top_k) ────────────
    print("\n8. Search with defaults")
    defaults = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/query",
        {
            "vector": query_vector,
        },
    )
    check(len(defaults["results"]) > 0, "search with defaults returns results")
    check(len(defaults["results"]) <= 10, "default top_k is 10")

    # ─── 9. Delete specific vectors ─────────────────────────────
    print("\n9. Delete specific vectors")
    del_result = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/delete",
        {
            "ids": ["vec-0", "vec-1", "vec-2"],
        },
    )
    check(del_result["ok"] is True, "delete returns ok:true")
    check(
        del_result["deleted"] == 3, f"deleted count is 3 (got {del_result['deleted']})"
    )

    count_after = api("GET", f"/api/v1/vectors/{NAMESPACE}/count")
    check(
        count_after["count"] == 7,
        f"count after delete is 7 (got {count_after['count']})",
    )

    # ─── 10. Search after deletion ──────────────────────────────
    print("\n10. Search after deletion (deleted vectors shouldn't appear)")
    search_after = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/query",
        {
            "vector": query_vector,
            "top_k": 10,
            "min_score": 0.0,
        },
    )
    result_ids = [r["id"] for r in search_after["results"]]
    check("vec-0" not in result_ids, "vec-0 not in results after deletion")
    check("vec-1" not in result_ids, "vec-1 not in results after deletion")
    check("vec-2" not in result_ids, "vec-2 not in results after deletion")

    # ─── 11. Idempotent upsert (overwrite) ──────────────────────
    print("\n11. Idempotent upsert (overwrite existing vector)")
    new_vector = random_vector(DIM)
    api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}",
        {
            "vectors": [
                {
                    "id": "vec-3",
                    "vector": new_vector,
                    "metadata": {"text": "UPDATED document 3", "source": "test"},
                }
            ],
        },
    )
    count_after_overwrite = api("GET", f"/api/v1/vectors/{NAMESPACE}/count")
    check(
        count_after_overwrite["count"] == 7,
        f"count unchanged after overwrite (still 7, got {count_after_overwrite['count']})",
    )

    # Verify the overwritten vector returns the new metadata
    search_overwrite = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/query",
        {
            "vector": new_vector,
            "top_k": 1,
            "min_score": 0.0,
        },
    )
    check(
        search_overwrite["results"][0]["id"] == "vec-3", "overwritten vector is found"
    )
    check(
        search_overwrite["results"][0]["payload"].get("text") == "UPDATED document 3",
        "overwritten vector has new metadata",
    )

    # ─── 12. Delete non-existent vectors (idempotent) ───────────
    print("\n12. Delete non-existent vectors (idempotent)")
    del_nonexist = api(
        "POST",
        f"/api/v1/vectors/{NAMESPACE}/delete",
        {
            "ids": ["does-not-exist-1", "does-not-exist-2"],
        },
    )
    check(del_nonexist["ok"] is True, "delete non-existent returns ok:true")
    count_unchanged = api("GET", f"/api/v1/vectors/{NAMESPACE}/count")
    check(
        count_unchanged["count"] == 7,
        f"count unchanged after deleting non-existent (got {count_unchanged['count']})",
    )

    # ─── 13. Query non-existent namespace ───────────────────────
    print("\n13. Query non-existent namespace")
    ghost = api(
        "POST",
        "/api/v1/vectors/__nonexistent_ns_test/query",
        {
            "vector": random_vector(DIM),
            "top_k": 5,
        },
    )
    check(ghost["results"] == [], "query on non-existent namespace returns empty list")

    # ─── 14. Delete from non-existent namespace ─────────────────
    print("\n14. Delete from non-existent namespace")
    del_ghost = api(
        "POST",
        "/api/v1/vectors/__nonexistent_ns_test/delete",
        {
            "ids": ["x"],
        },
    )
    check(del_ghost["ok"] is True, "delete from non-existent namespace returns ok")

    # ─── 15. Validation: empty vectors array ────────────────────
    print("\n15. Validation errors")
    try:
        api("POST", f"/api/v1/vectors/{NAMESPACE}", {"vectors": []})
        check(False, "empty vectors should be rejected (no error thrown)")
    except RuntimeError as e:
        check(
            "400" in str(e) or "empty" in str(e).lower(),
            "empty vectors array returns 400",
        )

    try:
        api("POST", f"/api/v1/vectors/{NAMESPACE}/delete", {"ids": []})
        check(False, "empty ids should be rejected")
    except RuntimeError as e:
        check(
            "400" in str(e) or "empty" in str(e).lower(), "empty ids array returns 400"
        )

    # ─── 16. Batch upsert (>500 vectors) ────────────────────────
    print("\n16. Batch upsert (600 vectors)")
    batch_ns = f"{NAMESPACE}_batch"
    big_batch = [
        {"id": f"b-{i}", "vector": random_vector(DIM), "metadata": {"idx": i}}
        for i in range(600)
    ]
    batch_result = api("POST", f"/api/v1/vectors/{batch_ns}", {"vectors": big_batch})
    check(
        batch_result["upserted"] == 600,
        f"batch upserted 600 (got {batch_result['upserted']})",
    )

    batch_count = api("GET", f"/api/v1/vectors/{batch_ns}/count")
    check(
        batch_count["count"] == 600, f"batch count is 600 (got {batch_count['count']})"
    )

    # ─── 17. Delete namespace ───────────────────────────────────
    print("\n17. Delete namespace")
    del_ns = api("DELETE", f"/api/v1/vectors/{NAMESPACE}")
    check(del_ns["ok"] is True, "delete namespace returns ok:true")
    check(del_ns["deleted"] is True, "delete namespace returns deleted:true")

    exists_after = api("GET", f"/api/v1/vectors/{NAMESPACE}/exists")
    check(exists_after["exists"] is False, "namespace gone after delete")

    # Cleanup batch namespace
    api("DELETE", f"/api/v1/vectors/{batch_ns}")

    # ─── 18. Delete non-existent namespace (idempotent) ─────────
    print("\n18. Delete non-existent namespace (idempotent)")
    del_ghost_ns = api("DELETE", f"/api/v1/vectors/{NAMESPACE}")
    check(del_ghost_ns["ok"] is True, "delete non-existent namespace returns ok")

    # ─── Summary ────────────────────────────────────────────────
    print(f"\n{'=' * 50}")
    print(f"Results: {passed} passed, {failed} failed")
    print(f"{'=' * 50}\n")

    sys.exit(1 if failed > 0 else 0)


if __name__ == "__main__":
    main()
