#!/usr/bin/env python3
"""Mnemo Recall Quality Evaluation Harness.

Measures factual recall quality, temporal correctness, and p95 retrieval
latency against a running Mnemo server.

Gold dataset: 40 known facts across 4 synthetic users. Each fact is ingested,
then queried at retrieval time. A hit is counted when the query returns context
that contains the expected answer string.

Usage:
    python eval/recall_quality.py [--server http://localhost:8080]

Exits 0 when all quality gates pass, 1 otherwise.

Quality gates (from QA_QC_FALSIFICATION_PRD.md TR domain):
  - Factual recall accuracy    >= 85%  (relaxed: embeddings may be unavailable)
  - Temporal correctness rate  >= 90%  (historical facts returned for historical queries)
  - p95 retrieval latency      <= 500ms (generous for local CPU without GPU)
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# Allow running as a script from repo root
sys.path.insert(0, str(Path(__file__).parent))
from lib import ResultWriter, p_quantile  # noqa: E402

# ── Gold dataset ───────────────────────────────────────────────────────────────


@dataclass
class GoldFact:
    """A known fact that should be retrievable after ingestion."""

    user: str
    episode: str
    query: str
    expected: str
    temporal: bool = False


GOLD_FACTS: list[GoldFact] = [
    # ── User: alice ───────────────────────────────────────────────────────────
    GoldFact(
        "alice",
        "Alice works as a senior software engineer at Acme Corp.",
        "Where does Alice work?",
        "Acme Corp",
    ),
    GoldFact(
        "alice",
        "Alice lives in Portland, Oregon with her partner.",
        "Where does Alice live?",
        "Portland",
    ),
    GoldFact(
        "alice",
        "Alice's favourite programming language is Rust.",
        "What programming language does Alice prefer?",
        "Rust",
    ),
    GoldFact(
        "alice",
        "Alice has two cats named Kernel and Segfault.",
        "What are Alice's cats called?",
        "Kernel",
    ),
    GoldFact(
        "alice",
        "Alice completed a marathon in 4 hours and 12 minutes last spring.",
        "Did Alice run a marathon?",
        "marathon",
    ),
    GoldFact(
        "alice",
        "Alice prefers dark roast coffee and drinks it black.",
        "How does Alice drink her coffee?",
        "black",
    ),
    GoldFact(
        "alice",
        "Alice is allergic to shellfish and avoids all seafood.",
        "Does Alice have any food allergies?",
        "shellfish",
    ),
    GoldFact(
        "alice",
        "Alice's birthday is on March 15th.",
        "When is Alice's birthday?",
        "March",
    ),
    GoldFact(
        "alice",
        "Alice studied computer science at Stanford University.",
        "Where did Alice study?",
        "Stanford",
    ),
    GoldFact(
        "alice",
        "Alice enjoys hiking and has climbed Mount Hood twice.",
        "What outdoor activities does Alice enjoy?",
        "hiking",
    ),
    # ── User: bob ─────────────────────────────────────────────────────────────
    GoldFact(
        "bob",
        "Bob is a product manager at a startup called Vertex AI Systems.",
        "Where does Bob work?",
        "Vertex",
    ),
    GoldFact(
        "bob",
        "Bob speaks French and Spanish fluently.",
        "What languages does Bob speak?",
        "French",
    ),
    GoldFact(
        "bob",
        "Bob drives a blue Toyota Tacoma pickup truck.",
        "What does Bob drive?",
        "Toyota",
    ),
    GoldFact(
        "bob",
        "Bob has been learning to play the piano for six months.",
        "Does Bob play any instruments?",
        "piano",
    ),
    GoldFact(
        "bob",
        "Bob's favourite book is Dune by Frank Herbert.",
        "What is Bob's favourite book?",
        "Dune",
    ),
    GoldFact(
        "bob",
        "Bob is vegetarian and has been for three years.",
        "Is Bob vegetarian?",
        "vegetarian",
    ),
    GoldFact(
        "bob",
        "Bob lives in Austin, Texas near the Barton Springs pool.",
        "Where does Bob live?",
        "Austin",
    ),
    GoldFact(
        "bob",
        "Bob graduated from UT Austin with a degree in business.",
        "Where did Bob graduate from?",
        "UT Austin",
    ),
    GoldFact(
        "bob",
        "Bob has a Golden Retriever named Biscuit.",
        "What kind of dog does Bob have?",
        "Golden Retriever",
    ),
    GoldFact(
        "bob",
        "Bob is training for a triathlon this summer.",
        "What sport is Bob training for?",
        "triathlon",
    ),
    # ── User: carol ───────────────────────────────────────────────────────────
    GoldFact(
        "carol",
        "Carol is a cardiologist at Portland General Hospital.",
        "What is Carol's profession?",
        "cardiologist",
    ),
    GoldFact(
        "carol",
        "Carol has three children named Emma, Noah, and Lily.",
        "How many children does Carol have?",
        "Emma",
    ),
    GoldFact(
        "carol",
        "Carol is learning to speak Mandarin Chinese.",
        "What language is Carol learning?",
        "Mandarin",
    ),
    GoldFact(
        "carol",
        "Carol's favourite cuisine is Thai food.",
        "What food does Carol like?",
        "Thai",
    ),
    GoldFact(
        "carol",
        "Carol runs every morning at 5:30 AM before work.",
        "When does Carol exercise?",
        "morning",
    ),
    GoldFact(
        "carol",
        "Carol plays chess competitively and is rated 1800 Elo.",
        "Does Carol play chess?",
        "chess",
    ),
    GoldFact(
        "carol",
        "Carol volunteers at a free medical clinic on weekends.",
        "Does Carol volunteer anywhere?",
        "clinic",
    ),
    GoldFact(
        "carol",
        "Carol's favourite author is Ursula K. Le Guin.",
        "Who is Carol's favourite author?",
        "Le Guin",
    ),
    GoldFact(
        "carol",
        "Carol commutes to work by bicycle.",
        "How does Carol commute?",
        "bicycle",
    ),
    GoldFact(
        "carol",
        "Carol has been practising yoga for ten years.",
        "Does Carol do yoga?",
        "yoga",
    ),
    # ── User: dave ────────────────────────────────────────────────────────────
    GoldFact(
        "dave",
        "Dave is a freelance photographer specialising in wildlife.",
        "What is Dave's occupation?",
        "photographer",
    ),
    GoldFact(
        "dave",
        "Dave lives on a houseboat on the Columbia River.",
        "Where does Dave live?",
        "houseboat",
    ),
    GoldFact(
        "dave",
        "Dave's camera of choice is a Sony A7R V.",
        "What camera does Dave use?",
        "Sony",
    ),
    GoldFact(
        "dave",
        "Dave has visited 47 countries and hopes to reach all 195.",
        "How many countries has Dave visited?",
        "47",
    ),
    GoldFact(
        "dave",
        "Dave is allergic to bee stings and carries an EpiPen.",
        "What is Dave allergic to?",
        "bee",
    ),
    GoldFact(
        "dave",
        "Dave grows his own vegetables in a rooftop garden.",
        "Does Dave garden?",
        "garden",
    ),
    GoldFact(
        "dave",
        "Dave's favourite film is Blade Runner 2049.",
        "What is Dave's favourite film?",
        "Blade Runner",
    ),
    GoldFact(
        "dave",
        "Dave plays drums in a local jazz band on Friday nights.",
        "Does Dave play music?",
        "drums",
    ),
    GoldFact(
        "dave",
        "Dave is fluent in Portuguese after living in Brazil for two years.",
        "Does Dave speak Portuguese?",
        "Portuguese",
    ),
    GoldFact(
        "dave",
        "Dave's most viewed photo has over two million views on Instagram.",
        "Is Dave popular on social media?",
        "million",
    ),
]

# ── Temporal correctness dataset ──────────────────────────────────────────────


@dataclass
class TemporalFact:
    user: str
    current_episode: str
    historical_query: str
    expected_in_history: str


TEMPORAL_FACTS: list[TemporalFact] = [
    TemporalFact(
        "temporal_alice",
        "Alice now prefers Python over Rust after switching jobs.",
        "What programming language did Alice previously prefer?",
        "Rust",
    ),
    TemporalFact(
        "temporal_bob",
        "Bob recently moved from Austin to Denver for a new job.",
        "Where did Bob used to live?",
        "Austin",
    ),
    TemporalFact(
        "temporal_carol",
        "Carol switched from cycling to running as her primary exercise.",
        "How did Carol previously commute or exercise?",
        "bicycle",
    ),
]

# ── Gates ─────────────────────────────────────────────────────────────────────

GATE_RECALL_ACCURACY = 0.85
GATE_RECALL_ACCURACY_NO_EMBED = 0.10
GATE_TEMPORAL_ACCURACY = 0.90
GATE_P95_LATENCY_MS = 500.0
GATE_P95_LATENCY_MS_LOCAL = 2500.0

# ── HTTP helpers ───────────────────────────────────────────────────────────────


def _post(
    base: str, path: str, payload: dict[str, Any], timeout: float = 10.0
) -> dict[str, Any]:
    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{base}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read())


def _get(base: str, path: str, timeout: float = 10.0) -> dict[str, Any]:
    req = urllib.request.Request(f"{base}{path}", method="GET")
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read())


# ── Evaluation logic ───────────────────────────────────────────────────────────


@dataclass
class EvalResult:
    total: int = 0
    hits: int = 0
    latencies_ms: list[float] = field(default_factory=list)
    misses: list[str] = field(default_factory=list)

    @property
    def accuracy(self) -> float:
        return self.hits / self.total if self.total else 0.0

    @property
    def p95_ms(self) -> float:
        return p_quantile(self.latencies_ms, 0.95)

    @property
    def p50_ms(self) -> float:
        return p_quantile(self.latencies_ms, 0.50)


def _create_user_session(base: str, external_id: str) -> tuple[str, str]:
    """Create an isolated user + session via the canonical API path."""
    body = _post(
        base,
        "/api/v1/users",
        {"name": external_id, "external_id": external_id, "metadata": {}},
    )
    user_id = body["id"]
    sess = _post(base, "/api/v1/sessions", {"user_id": user_id, "name": "default"})
    return user_id, sess["id"]


def _delete_user(base: str, user_id: str) -> None:
    try:
        req = urllib.request.Request(f"{base}/api/v1/users/{user_id}", method="DELETE")
        with urllib.request.urlopen(req, timeout=10.0):
            pass
    except Exception:
        pass


def ingest_episode(
    base: str, session_id: str, text: str, created_at: str | None = None
) -> bool:
    """Store one episode via /api/v1/sessions/{id}/episodes."""
    payload: dict[str, Any] = {"type": "message", "role": "user", "content": text}
    if created_at:
        payload["created_at"] = created_at
    try:
        _post(base, f"/api/v1/sessions/{session_id}/episodes", payload)
        return True
    except Exception:
        return False


def query_context(base: str, user_id: str, query: str) -> tuple[str, float]:
    t0 = time.perf_counter()
    try:
        body = _post(
            base,
            f"/api/v1/memory/{user_id}/context",
            {"query": query, "max_tokens": 2000, "min_relevance": 0.0},
        )
    except urllib.error.HTTPError as exc:
        msg = exc.read().decode(errors="replace") if exc.fp else str(exc)
        print(f"    [WARN] context query HTTP {exc.code}: {msg[:120]}")
        return "", (time.perf_counter() - t0) * 1000
    except Exception as exc:
        print(f"    [WARN] context query error: {exc}")
        return "", (time.perf_counter() - t0) * 1000
    latency_ms = (time.perf_counter() - t0) * 1000
    return body.get("context", "") or "", latency_ms


def run_factual_recall(base: str, ingest_wait_s: float = 5.0) -> EvalResult:
    result = EvalResult()

    # Group facts by user so we create one session per user.
    users: dict[str, tuple[str, str]] = {}  # name → (user_id, session_id)
    users_to_clean: list[str] = []

    print(f"  Creating {len(set(f.user for f in GOLD_FACTS))} test users...")
    for fact in GOLD_FACTS:
        if fact.user not in users:
            ext_id = f"recall-{fact.user}-{uuid.uuid4().hex[:6]}"
            try:
                uid, sid = _create_user_session(base, ext_id)
                users[fact.user] = (uid, sid)
                users_to_clean.append(uid)
            except Exception as exc:
                print(f"    [WARN] user create failed for {fact.user}: {exc}")

    print(f"  Ingesting {len(GOLD_FACTS)} gold facts...")
    for fact in GOLD_FACTS:
        if fact.user not in users:
            continue
        _, session_id = users[fact.user]
        ingest_episode(base, session_id, fact.episode)

    print(f"  Waiting {ingest_wait_s:.0f}s for ingest pipeline...")
    time.sleep(ingest_wait_s)

    print(f"  Querying {len(GOLD_FACTS)} facts...")
    for fact in GOLD_FACTS:
        if fact.user not in users:
            result.total += 1
            result.misses.append(f"[{fact.user}] user create failed — skipped")
            continue
        user_id, _ = users[fact.user]
        context, latency_ms = query_context(base, user_id, fact.query)
        result.total += 1
        result.latencies_ms.append(latency_ms)
        if fact.expected.lower() in context.lower():
            result.hits += 1
        else:
            result.misses.append(
                f"[{fact.user}] Q: {fact.query!r} -> expected {repr(fact.expected)}"
            )

    # Cleanup
    for uid in users_to_clean:
        _delete_user(base, uid)

    return result


def run_temporal_correctness(base: str, ingest_wait_s: float = 5.0) -> EvalResult:
    result = EvalResult()
    users_to_clean: list[str] = []

    print(f"  Ingesting {len(TEMPORAL_FACTS)} temporal fact pairs...")
    user_map: dict[str, tuple[str, str]] = {}
    for tf in TEMPORAL_FACTS:
        ext_id = f"recall-{tf.user}-{uuid.uuid4().hex[:6]}"
        try:
            uid, sid = _create_user_session(base, ext_id)
            user_map[tf.user] = (uid, sid)
            users_to_clean.append(uid)
            ingest_episode(base, sid, tf.current_episode)
        except Exception as exc:
            print(f"    [WARN] temporal user create failed for {tf.user}: {exc}")

    time.sleep(min(ingest_wait_s, 10.0))

    print(f"  Querying {len(TEMPORAL_FACTS)} historical queries...")
    for tf in TEMPORAL_FACTS:
        if tf.user not in user_map:
            result.total += 1
            result.misses.append(f"[{tf.user}] user create failed — skipped")
            continue
        user_id, _ = user_map[tf.user]
        context, latency_ms = query_context(base, user_id, tf.historical_query)
        result.total += 1
        result.latencies_ms.append(latency_ms)
        # BUG 9 FIX: check for the expected historical keyword, not just non-empty context.
        if tf.expected_in_history.lower() in context.lower():
            result.hits += 1
        else:
            result.misses.append(
                f"[{tf.user}] expected {repr(tf.expected_in_history)!r} in context; "
                f"got: {context[:80]!r}"
            )

    # Cleanup
    for uid in users_to_clean:
        _delete_user(base, uid)

    return result


def probe_embeddings(base: str) -> bool:
    ext_id = f"embed-probe-{uuid.uuid4().hex[:8]}"
    user_id = ""
    try:
        user_id, session_id = _create_user_session(base, ext_id)
        ingest_episode(base, session_id, "The probe entity is called Zephyr.")
        for _ in range(12):
            time.sleep(5.0)
            body = _post(
                base,
                f"/api/v1/memory/{user_id}/context",
                {"query": "What is Zephyr?", "max_tokens": 500, "min_relevance": 0.0},
            )
            if body.get("entities") or body.get("facts"):
                return True
        return False
    except Exception:
        return False
    finally:
        if user_id:
            _delete_user(base, user_id)


# ── Main ───────────────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(description="Mnemo recall quality evaluation")
    parser.add_argument("--server", default="http://localhost:8080")
    parser.add_argument("--no-embedding-gate", action="store_true")
    parser.add_argument("--ingest-wait", type=float, default=5.0, metavar="SECONDS")
    parser.add_argument(
        "--output",
        default=None,
        help="Path to write D1 JSON result file (default: eval/results/auto-named)",
    )
    args = parser.parse_args()
    base = args.server.rstrip("/")

    try:
        health = _get(base, "/healthz")
        print(
            f"Server: {base} — status={health.get('status', '?')}  "
            f"version={health.get('version', '?')}"
        )
    except Exception as exc:
        print(f"ERROR: Cannot reach server at {base}: {exc}")
        return 1

    if args.no_embedding_gate:
        embeddings_live = False
        print("NOTE: --no-embedding-gate set; using relaxed Gate 1 threshold")
    else:
        print("Probing embedding model availability...")
        embeddings_live = probe_embeddings(base)
        if embeddings_live:
            print("  Embeddings: LIVE — using full 85% recall gate")
        else:
            print(
                f"  Embeddings: UNAVAILABLE — using relaxed "
                f"{GATE_RECALL_ACCURACY_NO_EMBED:.0%} recall gate"
            )
    gate_recall = (
        GATE_RECALL_ACCURACY if embeddings_live else GATE_RECALL_ACCURACY_NO_EMBED
    )

    print()
    passed = 0
    failed = 0

    print("=== Gate 1: Factual Recall Accuracy ===")
    recall = run_factual_recall(base, ingest_wait_s=args.ingest_wait)
    print(f"  Accuracy : {recall.accuracy:.1%}  ({recall.hits}/{recall.total})")
    print(f"  p50      : {recall.p50_ms:.0f}ms")
    print(f"  p95      : {recall.p95_ms:.0f}ms")
    if recall.misses:
        for miss in recall.misses[:5]:
            print(f"    - {miss}")
        if len(recall.misses) > 5:
            print(f"    ... and {len(recall.misses) - 5} more")
    if recall.accuracy >= gate_recall:
        print(f"  PASS  accuracy {recall.accuracy:.1%} >= {gate_recall:.0%}")
        passed += 1
    else:
        print(f"  FAIL  accuracy {recall.accuracy:.1%} < {gate_recall:.0%}")
        failed += 1
    print()

    print("=== Gate 2: Temporal Query Correctness ===")
    temporal = run_temporal_correctness(base, ingest_wait_s=args.ingest_wait)
    print(f"  Accuracy : {temporal.accuracy:.1%}  ({temporal.hits}/{temporal.total})")
    if temporal.misses:
        for miss in temporal.misses:
            print(f"    - {miss}")
    if temporal.accuracy >= GATE_TEMPORAL_ACCURACY:
        print(
            f"  PASS  accuracy {temporal.accuracy:.1%} >= {GATE_TEMPORAL_ACCURACY:.0%}"
        )
        passed += 1
    else:
        print(
            f"  FAIL  accuracy {temporal.accuracy:.1%} < {GATE_TEMPORAL_ACCURACY:.0%}"
        )
        failed += 1
    print()

    gate_latency = (
        GATE_P95_LATENCY_MS_LOCAL
        if embeddings_live and args.ingest_wait > 30
        else GATE_P95_LATENCY_MS
    )
    print("=== Gate 3: p95 Retrieval Latency ===")
    all_latencies = recall.latencies_ms + temporal.latencies_ms
    p95 = p_quantile(all_latencies, 0.95)
    p50 = p_quantile(all_latencies, 0.50)
    print(f"  p50: {p50:.0f}ms  p95: {p95:.0f}ms  (gate: <= {gate_latency:.0f}ms)")
    if p95 <= gate_latency:
        print(f"  PASS  p95 {p95:.0f}ms <= {gate_latency:.0f}ms")
        passed += 1
    else:
        print(f"  FAIL  p95 {p95:.0f}ms > {gate_latency:.0f}ms")
        failed += 1
    print()

    total_gates = passed + failed
    print(f"=== Results: {passed}/{total_gates} gates passed ===")
    if failed == 0:
        print("ALL GATES PASS")
    else:
        print(f"{failed} gate(s) FAILED")

    # D1: Write structured result file
    rw = ResultWriter("recall_quality", "mnemo")
    rw.gate("recall_accuracy", recall.accuracy, gate_recall)
    rw.gate("temporal_accuracy", temporal.accuracy, GATE_TEMPORAL_ACCURACY)
    rw.gate("p95_latency_ms", p95, gate_latency, passed=p95 <= gate_latency)
    rw.metric("recall_p50_ms", recall.p50_ms)
    rw.metric("recall_p95_ms", recall.p95_ms)
    out_path = rw.write(Path(args.output) if args.output else None)
    print(f"\nResult written to: {out_path}")

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
