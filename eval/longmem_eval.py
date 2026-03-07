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
from typing import Any

# ─── Re-use HttpClient and Backend ABC from temporal_eval ────────────────────
import os

sys.path.insert(0, os.path.dirname(__file__))
from temporal_eval import Backend, HttpClient, MnemoBackend  # noqa: E402


# ─── Result types ─────────────────────────────────────────────────────────────


@dataclass
class TaskResult:
    task_type: str
    total: int
    passed: int
    errors: int
    latencies_ms: list[int] = field(default_factory=list)

    @property
    def accuracy(self) -> float:
        return self.passed / self.total if self.total else 0.0

    @property
    def p95_ms(self) -> int:
        if not self.latencies_ms:
            return 0
        ordered = sorted(self.latencies_ms)
        idx = min(len(ordered) - 1, int(0.95 * (len(ordered) - 1)))
        return int(ordered[idx])


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
            {
                "content": "My best friend is Sofia.",
                "created_at": "2025-01-01T10:00:00Z",
            },
            {
                "content": "Sofia moved to Austin last year.",
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
    {
        "name": "pref_coffee_order",
        "task_type": "preference",
        "memories": [
            {
                "content": "I always order a flat white at coffee shops.",
                "created_at": "2024-01-01T00:00:00Z",
            },
            {
                "content": "I switched to oat milk lattes after going dairy-free.",
                "created_at": "2025-02-01T00:00:00Z",
            },
        ],
        "query": {
            "text": "What coffee drink do I prefer now?",
            "mode": "head",
            "time_intent": "current",
        },
        "expect": {
            "contains": ["oat milk latte"],
            "not_contains": ["flat white"],
        },
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
        "expect": {"contains": ["Bloomberg"], "not_contains": ["Guardian"]},
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
        "expect": {"contains": ["swimming"], "not_contains": ["running"]},
    },
    # ── 5. Absent information ──────────────────────────────────────────────
    {
        "name": "absent_salary",
        "task_type": "absent",
        "memories": [
            {
                "content": "I work as a software engineer at Meridian Systems.",
                "created_at": "2025-01-15T09:00:00Z",
            }
        ],
        "query": {"text": "What is my annual salary?"},
        "expect": {"absent": True},
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
        "expect": {"absent": True},
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
        "expect": {"absent": True},
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


def _check_contains(text: str, tokens: list[str]) -> bool:
    lower = text.lower()
    return all(t.lower() in lower for t in tokens)


def _check_not_contains(text: str, tokens: list[str]) -> bool:
    lower = text.lower()
    return not any(t.lower() in lower for t in tokens)


def run_longmem_eval(
    backend: MnemoBackend,
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
            user_id, session_id = backend.create_user_session(external_id)

            # Ingest memories
            all_ok = True
            for mem in case.get("memories", []):
                ok = backend.remember(
                    user_id, session_id, mem["content"], mem["created_at"]
                )
                if not ok:
                    all_ok = False

            if not all_ok:
                result.errors += 1
                if verbose:
                    print(f"[{case['name']}] INGEST FAILED")
                continue

            # Retrieve
            query = case["query"]
            t0 = time.time()
            status, ctx_text, latency_ms = backend.retrieve(
                user_id, session_id, query, tt
            )
            result.latencies_ms.append(latency_ms)

            if status != 200:
                result.errors += 1
                if verbose:
                    print(f"[{case['name']}] HTTP {status} latency={latency_ms}ms")
                continue

            expect = case.get("expect", {})
            is_absent = expect.get("absent", False)

            if is_absent:
                # Absent: pass if context is empty or contains no relevant tokens
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
            f"{r.p95_ms:>11}ms  {status}  ({r.passed}/{r.total}, errors={r.errors})"
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

    if args.gate_only:
        sys.exit(0 if all_pass else 1)
    else:
        sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()
