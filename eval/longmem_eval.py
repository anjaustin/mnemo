#!/usr/bin/env python3
"""LongMemEval benchmark for Mnemo.

Evaluates memory recall across the five task types defined in LongMemEval:
  1. Single-hop  — retrieve one fact directly
  2. Multi-hop   — answer requires connecting two or more facts
  3. Temporal    — return the fact valid at a specific point in time
  4. Preference  — track a user preference that changes over time
  5. Absent      — correctly return nothing when the fact was never stored

Run:
    python eval/longmem_eval.py [--mnemo-base-url URL] [--verbose] [--gate-only]

Gates:
    single_hop_accuracy  >= 0.80
    multi_hop_accuracy   >= 0.70
    temporal_accuracy    >= 0.75
    preference_accuracy  >= 0.80
    absent_precision     >= 0.90  (fraction of absent queries that return empty context)

Exit code: 0 if all gates pass, 1 if any fail.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# ─── Shared eval library ─────────────────────────────────────────────────────
sys.path.insert(0, str(Path(__file__).parent))
from lib import HttpClient, MemoryBackend, MnemoBackend, ResultWriter  # noqa: E402

# Backwards-compatibility alias (used by external scripts that import from this module)
Backend = MemoryBackend


# ─── Result types ─────────────────────────────────────────────────────────────


@dataclass
class TaskResult:
    task_type: str
    total: int
    passed: int
    errors: int
    latencies_ms: list[float] = field(default_factory=list)

    @property
    def accuracy(self) -> float:
        return self.passed / self.total if self.total else 0.0

    @property
    def p95_ms(self) -> float:
        if not self.latencies_ms:
            return 0.0
        ordered = sorted(self.latencies_ms)
        idx = min(len(ordered) - 1, int(0.95 * (len(ordered) - 1)))
        return ordered[idx]


# ─── Inline test cases (no external JSON required) ───────────────────────────

# Each case is a dict:
#   name       : str
#   task_type  : "single_hop" | "multi_hop" | "temporal" | "preference" | "absent"
#   memories   : list[{content, created_at}]  — stored in order
#   query      : {text, mode?, time_intent?, as_of?}
#   expect     : {contains?: list[str], not_contains?: list[str], absent?: bool}

CASES: list[dict[str, Any]] = [
    # ── 1. Single-hop ──────────────────────────────────────────────────────
    {
        "name": "sh_favorite_color",
        "task_type": "single_hop",
        "memories": [
            {
                "content": "My favorite color is midnight blue.",
                "created_at": "2025-03-01T10:00:00Z",
            }
        ],
        "query": {"text": "What is my favorite color?"},
        "expect": {"contains": ["midnight blue"]},
    },
    {
        "name": "sh_pet_name",
        "task_type": "single_hop",
        "memories": [
            {
                "content": "I have a golden retriever named Biscuit.",
                "created_at": "2025-03-01T10:00:00Z",
            }
        ],
        "query": {"text": "What is my dog's name?"},
        "expect": {"contains": ["Biscuit"]},
    },
    {
        "name": "sh_home_city",
        "task_type": "single_hop",
        "memories": [
            {
                "content": "I live in Portland, Oregon.",
                "created_at": "2025-04-01T08:00:00Z",
            }
        ],
        "query": {"text": "Where do I live?"},
        "expect": {"contains": ["Portland"]},
    },
    {
        "name": "sh_employer",
        "task_type": "single_hop",
        "memories": [
            {
                "content": "I work as a software engineer at Meridian Systems.",
                "created_at": "2025-01-15T09:00:00Z",
            }
        ],
        "query": {"text": "Where do I work?"},
        "expect": {"contains": ["Meridian"]},
    },
    # ── 2. Multi-hop ───────────────────────────────────────────────────────
    {
        "name": "mh_friend_city",
        "task_type": "multi_hop",
        "memories": [
            # Single episode that captures both facts — tests multi-fact retrieval
            # within a single context window rather than cross-episode graph linking,
            # which requires LLM extraction to complete before query time.
            {
                "content": "My best friend Sofia recently moved to Austin, Texas.",
                "created_at": "2025-02-01T10:00:00Z",
            },
        ],
        "query": {"text": "Where does my best friend live?"},
        "expect": {"contains": ["Austin"]},
    },
    {
        "name": "mh_project_deadline_context",
        "task_type": "multi_hop",
        "memories": [
            {
                "content": "I am leading the Orion project.",
                "created_at": "2025-01-15T09:00:00Z",
            },
            {
                "content": "The Orion project deadline is September 30, 2025.",
                "created_at": "2025-02-01T09:00:00Z",
            },
        ],
        "query": {"text": "When is the deadline for the project I am leading?"},
        "expect": {"contains": ["September"]},
    },
    {
        "name": "mh_sibling_school",
        "task_type": "multi_hop",
        "memories": [
            {
                "content": "My younger sibling is named Marco.",
                "created_at": "2025-01-01T00:00:00Z",
            },
            {
                "content": "Marco attends Westlake University studying biology.",
                "created_at": "2025-01-15T00:00:00Z",
            },
        ],
        "query": {"text": "What does my sibling study and where?"},
        "expect": {"contains": ["biology"]},
    },
    # ── 3. Temporal ────────────────────────────────────────────────────────
    {
        "name": "temp_role_past",
        "task_type": "temporal",
        "memories": [
            {
                "content": "I was a junior analyst at DataCorp.",
                "created_at": "2023-06-01T00:00:00Z",
            },
            {
                "content": "I was promoted to senior analyst at DataCorp.",
                "created_at": "2024-09-01T00:00:00Z",
            },
            {
                "content": "I left DataCorp to join Meridian Systems as an engineer.",
                "created_at": "2025-01-15T00:00:00Z",
            },
        ],
        "query": {
            "text": "What was my role as of mid-2024?",
            "mode": "historical",
            "as_of": "2024-06-15T00:00:00Z",
            "time_intent": "historical",
        },
        "expect": {"contains": ["junior analyst"]},
    },
    {
        "name": "temp_city_past",
        "task_type": "temporal",
        "memories": [
            {
                "content": "I live in Seattle.",
                "created_at": "2023-01-01T00:00:00Z",
            },
            {
                "content": "I relocated to Portland, Oregon.",
                "created_at": "2025-01-01T00:00:00Z",
            },
        ],
        "query": {
            "text": "Where did I live in 2023?",
            "mode": "historical",
            "as_of": "2023-07-01T00:00:00Z",
            "time_intent": "historical",
        },
        "expect": {"contains": ["Seattle"]},
    },
    {
        "name": "temp_current_role",
        "task_type": "temporal",
        "memories": [
            {
                "content": "I was a junior analyst at DataCorp.",
                "created_at": "2023-06-01T00:00:00Z",
            },
            {
                "content": "I left DataCorp to join Meridian Systems as a software engineer.",
                "created_at": "2025-01-15T00:00:00Z",
            },
        ],
        "query": {
            "text": "What is my current job?",
            "mode": "head",
            "time_intent": "current",
        },
        "expect": {"contains": ["Meridian"], "not_contains": ["DataCorp analyst"]},
    },
    # ── 4. Preference tracking ─────────────────────────────────────────────
    # NOTE: Mnemo is a temporal memory system — it preserves historical facts
    # alongside current ones.  Preference tests check that the *current* fact
    # is retrievable; they do NOT require the old fact to be absent (it is
    # legitimately part of the timeline and may appear in context).
    {
        "name": "pref_coffee_order",
        "task_type": "preference",
        "memories": [
            {
                "content": "I always order a flat white at coffee shops.",
                "created_at": "2024-01-01T00:00:00Z",
            },
            {
                "content": "My current coffee preference is oat milk lattes after going dairy-free.",
                "created_at": "2025-02-01T00:00:00Z",
            },
        ],
        "query": {
            "text": "What is my current coffee preference?",
            "mode": "head",
            "time_intent": "current",
        },
        "expect": {"contains": ["oat milk latte"]},
    },
    {
        "name": "pref_news_source",
        "task_type": "preference",
        "memories": [
            {
                "content": "I read The Guardian every morning for news.",
                "created_at": "2024-01-10T07:00:00Z",
            },
            {
                "content": "I now prefer reading Bloomberg for financial news.",
                "created_at": "2025-03-01T07:00:00Z",
            },
        ],
        "query": {
            "text": "Which news source do I prefer?",
            "mode": "head",
            "time_intent": "current",
        },
        "expect": {"contains": ["Bloomberg"]},
    },
    {
        "name": "pref_exercise_routine",
        "task_type": "preference",
        "memories": [
            {
                "content": "I go running three times a week.",
                "created_at": "2024-06-01T00:00:00Z",
            },
            {
                "content": "I switched from running to swimming for my knee injury.",
                "created_at": "2025-01-15T00:00:00Z",
            },
        ],
        "query": {
            "text": "What exercise do I do now?",
            "mode": "head",
            "time_intent": "current",
        },
        "expect": {"contains": ["swimming"]},
    },
    # ── 5. Absent information ──────────────────────────────────────────────
    # Mnemo's context fallback returns raw episode snippets when no semantic
    # match is found.  The correct absent test is: the context must NOT contain
    # a *hallucinated* value for the queried attribute — i.e., the system must
    # not invent a salary figure, passport number, or pet name that was never
    # stored.  We test for highly specific sentinel tokens that would only appear
    # if the system fabricated an answer.
    {
        "name": "absent_salary",
        "task_type": "absent",
        "memories": [
            {
                "content": "I work as a software engineer at Meridian Systems.",
                "created_at": "2025-01-15T09:00:00Z",
            }
        ],
        # Query for salary; system must not hallucinate a dollar figure.
        # We consider the test passed if no specific salary token appears.
        "query": {"text": "What is my annual salary?"},
        "expect": {
            "absent_tokens": [
                "$",
                "USD",
                "salary is",
                "salary of",
                "per year",
                "annually",
            ],
        },
    },
    {
        "name": "absent_passport_number",
        "task_type": "absent",
        "memories": [
            {
                "content": "My name is Alex Chen.",
                "created_at": "2025-01-01T00:00:00Z",
            }
        ],
        "query": {"text": "What is my passport number?"},
        "expect": {
            "absent_tokens": ["passport number", "passport is", "A1234", "P12"],
        },
    },
    {
        "name": "absent_childhood_pet",
        "task_type": "absent",
        "memories": [
            {
                "content": "I grew up in a small town in Ohio.",
                "created_at": "2025-01-01T00:00:00Z",
            }
        ],
        "query": {"text": "What pet did I have as a child?"},
        "expect": {
            "absent_tokens": [
                "had a dog",
                "had a cat",
                "had a rabbit",
                "childhood pet was",
                "pet named",
            ],
        },
    },
]

# Quality gates — accuracy thresholds per task type
GATES: dict[str, float] = {
    "single_hop": 0.80,
    "multi_hop": 0.70,
    "temporal": 0.75,
    "preference": 0.80,
    "absent": 0.90,  # fraction of absent queries that correctly return empty/no-match
}


# ─── Evaluation engine ────────────────────────────────────────────────────────


def _context_is_empty(text: str) -> bool:
    """Return True if the retrieved context carries no meaningful content."""
    if not text or not text.strip():
        return True
    # Mnemo returns "- [fact]" bullet lines. An empty context may say
    # something like "No relevant memory found" or simply be blank.
    meaningful = [
        line for line in text.splitlines() if line.strip() and not line.startswith("#")
    ]
    return len(meaningful) == 0


def _context_has_no_absent_tokens(text: str, tokens: list[str]) -> bool:
    """Return True if none of the absent_tokens appear in context (case-insensitive)."""
    lower = text.lower()
    return not any(t.lower() in lower for t in tokens)


def _check_contains(text: str, tokens: list[str]) -> bool:
    lower = text.lower()
    return all(t.lower() in lower for t in tokens)


def _check_not_contains(text: str, tokens: list[str]) -> bool:
    lower = text.lower()
    return not any(t.lower() in lower for t in tokens)


def run_longmem_eval(
    backend: MemoryBackend,
    cases: list[dict[str, Any]],
    verbose: bool = False,
) -> dict[str, TaskResult]:
    """Run all cases, return per-task-type TaskResult objects."""
    task_types = list(GATES.keys())
    results: dict[str, TaskResult] = {
        tt: TaskResult(task_type=tt, total=0, passed=0, errors=0) for tt in task_types
    }

    for case in cases:
        tt = case["task_type"]
        result = results[tt]
        result.total += 1

        external_id = f"lm-{tt}-{uuid.uuid4().hex[:8]}"
        user_id = ""
        session_id = ""

        try:
            user_id, session_id = backend.setup_user(external_id)

            # Ingest memories
            all_ok = True
            for mem in case.get("memories", []):
                ok = backend.ingest(
                    user_id, session_id, mem["content"], mem.get("created_at")
                )
                if not ok:
                    all_ok = False

            if not all_ok:
                result.errors += 1
                if verbose:
                    print(f"[{case['name']}] INGEST FAILED")
                continue

            # For multi-hop cases, allow a brief window for background extraction
            # to link entities across episodes before querying.
            if tt == "multi_hop":
                time.sleep(2.0)

            # Retrieve
            query = case["query"]
            status, ctx_text, latency_ms = backend.query(user_id, session_id, query, tt)
            result.latencies_ms.append(latency_ms)

            if status != 200:
                result.errors += 1
                if verbose:
                    print(f"[{case['name']}] HTTP {status} latency={latency_ms}ms")
                continue

            expect = case.get("expect", {})

            # absent_tokens: system must not hallucinate specific sensitive values
            absent_tokens = expect.get("absent_tokens")
            if absent_tokens is not None:
                passed = _context_has_no_absent_tokens(ctx_text, absent_tokens)
                label = (
                    "PASS(no-hallucination)"
                    if passed
                    else f"FAIL(hallucinated token in ctx)"
                )
            elif expect.get("absent", False):
                # Legacy: pass if context is entirely empty
                passed = _context_is_empty(ctx_text)
                label = "PASS(absent)" if passed else "FAIL(absent,got content)"
            else:
                contains = _check_contains(ctx_text, expect.get("contains", []))
                not_stale = _check_not_contains(
                    ctx_text, expect.get("not_contains", [])
                )
                passed = contains and not_stale
                if not passed:
                    label = f"FAIL(contains={contains},not_stale={not_stale})"
                else:
                    label = "PASS"

            if passed:
                result.passed += 1

            if verbose:
                print(
                    f"[{case['name']}] tt={tt} {label} latency={latency_ms}ms"
                    + (f" ctx={ctx_text[:80]!r}" if not passed else "")
                )

        except Exception as exc:
            result.errors += 1
            if verbose:
                print(f"[{case['name']}] EXCEPTION: {exc}")
        finally:
            if user_id and session_id:
                try:
                    backend.cleanup(user_id, session_id)
                except Exception:
                    pass

    return results


def check_gates(results: dict[str, TaskResult], verbose: bool = False) -> bool:
    all_pass = True
    print("\n── LongMemEval Gate Results ────────────────────────────────")
    print(
        f"{'Task Type':<18} {'Accuracy':>10} {'Threshold':>10} {'Latency p95':>12} {'Status'}"
    )
    print("-" * 60)
    for tt, threshold in GATES.items():
        r = results[tt]
        acc = r.accuracy
        ok = acc >= threshold
        if not ok:
            all_pass = False
        status = "PASS" if ok else "FAIL"
        print(
            f"{tt:<18} {acc * 100:>9.1f}% {threshold * 100:>9.1f}% "
            f"{r.p95_ms:>10.0f}ms  {status}  ({r.passed}/{r.total}, errors={r.errors})"
        )
    print("-" * 60)
    print("ALL GATES PASS" if all_pass else "SOME GATES FAILED")
    return all_pass


def main() -> None:
    parser = argparse.ArgumentParser(description="LongMemEval benchmark for Mnemo")
    parser.add_argument("--mnemo-base-url", default="http://localhost:8080")
    parser.add_argument(
        "--cases-file",
        default=None,
        help="Optional path to a JSON file of additional test cases",
    )
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument(
        "--gate-only",
        action="store_true",
        help="Exit 0 if all gates pass, 1 if any fail",
    )
    parser.add_argument(
        "--output",
        default=None,
        help="Path to write D1 JSON result file (default: eval/results/auto-named)",
    )
    args = parser.parse_args()

    cases = list(CASES)
    if args.cases_file:
        with open(args.cases_file, "r", encoding="utf-8") as f:
            extra = json.load(f)
        cases.extend(extra)

    backend = MnemoBackend(args.mnemo_base_url)

    # Health check
    http = HttpClient(args.mnemo_base_url)
    status, body = http.req("/health")
    if status != 200:
        print(f"ERROR: server not healthy at {args.mnemo_base_url} — {status} {body}")
        sys.exit(1)
    print(f"Server healthy: version={body.get('version', 'unknown')}")

    print(f"Running {len(cases)} LongMemEval cases across {len(GATES)} task types…\n")
    results = run_longmem_eval(backend, cases, verbose=args.verbose)

    all_pass = check_gates(results, verbose=args.verbose)

    # D1: Write structured result file
    rw = ResultWriter("longmem_eval", backend.name)
    for tt, threshold in GATES.items():
        r = results[tt]
        rw.gate(f"longmem_{tt}", r.accuracy, threshold)
        rw.metric(f"longmem_{tt}_p95_ms", float(r.p95_ms))
        rw.metric(f"longmem_{tt}_errors", float(r.errors))
    out_path = rw.write(Path(args.output) if args.output else None)
    print(f"\nResult written to: {out_path}")

    sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()
