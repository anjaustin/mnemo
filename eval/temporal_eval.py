#!/usr/bin/env python3
"""Temporal retrieval evaluation harness.

Tests that Mnemo (and optionally Zep) correctly:
  - Returns the current fact for head-mode queries
  - Returns the historical fact for as_of queries
  - Does not surface stale (superseded) facts as primary results

Run:
    python eval/temporal_eval.py [--cases eval/temporal_cases.json] [--verbose]
    python eval/temporal_eval.py --cases eval/cases/enterprise_crm.json

Gates (enforced in CI via quality-gates.yml):
    temporal_accuracy   >= 95%
    stale_fact_rate     <= 5%
    p95_latency_ms      <= 300ms

Exit code: 0 if all gates pass, 1 if any fail.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# Allow running as a script from repo root
sys.path.insert(0, str(Path(__file__).parent))
from lib import (  # noqa: E402
    HttpClient,
    MemoryBackend,
    MnemoBackend,
    ResultWriter,
    ZepBackend,
    p_quantile,
)

# Re-export for longmem_eval.py backwards compatibility
Backend = MemoryBackend

ACCURACY_GATE = 0.95
STALE_GATE = 0.05
P95_GATE_MS = 300.0


@dataclass
class EvalResult:
    system: str
    profile: str
    total: int
    passed: int
    stale_failures: int
    errors: int
    latencies_ms: list[float]

    @property
    def accuracy(self) -> float:
        return (self.passed / self.total) if self.total else 0.0

    @property
    def stale_rate(self) -> float:
        return (self.stale_failures / self.total) if self.total else 0.0

    @property
    def p50_ms(self) -> float:
        return p_quantile(self.latencies_ms, 0.50)

    @property
    def p95_ms(self) -> float:
        return p_quantile(self.latencies_ms, 0.95)


def extract_top_context_line(context_text: str) -> str:
    """Return the first bullet line from the context text.

    Mnemo formats context as '- [fact]' bullet lines.  If no bullet line is
    found (e.g. plain prose or a different format), return the full text so
    contains/not_contains checks still work — but log a brief prefix so callers
    can see the fallback in verbose output.
    """
    for line in context_text.splitlines():
        stripped = line.strip()
        if stripped.startswith("- [") or stripped.startswith("- "):
            return stripped
    # Fallback: return the first non-empty line (not the whole blob).
    # This prevents a stale token anywhere in a long context from
    # inflating the stale-rate when the top result is actually correct.
    for line in context_text.splitlines():
        if line.strip():
            return line.strip()
    return context_text


def run_profile(
    backend: MemoryBackend,
    cases: list[dict[str, Any]],
    profile: str,
    verbose: bool = False,
) -> EvalResult:
    total = len(cases)
    passed = 0
    stale_failures = 0
    errors = 0
    latencies_ms: list[float] = []

    for case in cases:
        external_id = f"eval-{backend.name}-{profile}-{uuid.uuid4().hex[:8]}"
        user_id = ""
        session_id = ""

        try:
            user_id, session_id = backend.setup_user(external_id)

            memories_ok = True
            for memory in case.get("memories", []):
                ok = backend.ingest(
                    user_id, session_id, memory["content"], memory.get("created_at")
                )
                memories_ok = memories_ok and ok

            if not memories_ok:
                errors += 1
                if verbose:
                    print(
                        f"[case:{case.get('name', '?')}] profile={profile} status=remember_failed"
                    )
                continue

            status, context_text, latency_ms = backend.query(
                user_id, session_id, case["query"], profile
            )
            latencies_ms.append(latency_ms)

            if status != 200:
                errors += 1
                if verbose:
                    print(
                        f"[case:{case.get('name', '?')}] profile={profile} "
                        f"status=http_{status} latency_ms={latency_ms:.0f}"
                    )
                continue

            top_line = extract_top_context_line(context_text)
            expect = case.get("expect", {})
            contains = all(token in top_line for token in expect.get("contains", []))
            stale = any(token in top_line for token in expect.get("not_contains", []))

            if stale:
                stale_failures += 1
            if contains and not stale:
                passed += 1
                if verbose:
                    print(
                        f"[case:{case.get('name', '?')}] profile={profile} "
                        f"status=pass latency_ms={latency_ms:.0f}"
                    )
            elif verbose:
                print(
                    f"[case:{case.get('name', '?')}] profile={profile} "
                    f"status=fail latency_ms={latency_ms:.0f} "
                    f"contains={contains} stale={stale} top={top_line!r}"
                )

        except Exception as exc:
            errors += 1
            if verbose:
                print(
                    f"[case:{case.get('name', '?')}] profile={profile} EXCEPTION: {exc}"
                )
        finally:
            if user_id and session_id:
                try:
                    backend.cleanup(user_id, session_id)
                except Exception:
                    pass

    return EvalResult(
        system=backend.name,
        profile=profile,
        total=total,
        passed=passed,
        stale_failures=stale_failures,
        errors=errors,
        latencies_ms=latencies_ms,
    )


def print_markdown(results: list[EvalResult]) -> None:
    print("| System | Profile | Accuracy | Stale Rate | Errors | p50 (ms) | p95 (ms) |")
    print("|---|---|---:|---:|---:|---:|---:|")
    for r in results:
        print(
            f"| {r.system} | {r.profile} | {r.accuracy * 100:.1f}% | "
            f"{r.stale_rate * 100:.1f}% | {r.errors} | "
            f"{r.p50_ms:.0f} | {r.p95_ms:.0f} |"
        )


def load_key(path: str) -> str:
    with open(path, "r", encoding="utf-8") as f:
        return f.read().strip()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run Mnemo/Zep temporal evaluation harness"
    )
    parser.add_argument(
        "--cases",
        default=str(Path(__file__).with_name("temporal_cases.json")),
        help="Path to JSON case pack",
    )
    parser.add_argument("--target", choices=["mnemo", "zep", "both"], default="mnemo")
    parser.add_argument("--mnemo-base-url", default="http://localhost:8080")
    parser.add_argument("--zep-base-url", default="https://api.getzep.com/api/v2")
    parser.add_argument("--zep-api-key", default=None)
    parser.add_argument("--zep-api-key-file", default="zep_api.key")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument(
        "--output",
        default=None,
        help="Path to write D1 JSON result file (default: eval/results/auto-named)",
    )
    args = parser.parse_args()

    with open(args.cases, "r", encoding="utf-8") as f:
        cases = json.load(f)

    results: list[EvalResult] = []

    if args.target in ("mnemo", "both"):
        mnemo = MnemoBackend(args.mnemo_base_url)
        results.append(run_profile(mnemo, cases, "temporal", verbose=args.verbose))
        results.append(run_profile(mnemo, cases, "baseline", verbose=args.verbose))

    if args.target in ("zep", "both"):
        key = args.zep_api_key or os.environ.get("ZEP_API_KEY")
        if not key and os.path.exists(args.zep_api_key_file):
            key = load_key(args.zep_api_key_file)
        if not key:
            print("ERROR: ZEP_API_KEY not set", file=sys.stderr)
            sys.exit(1)
        zep = ZepBackend(args.zep_base_url, key)
        results.append(run_profile(zep, cases, "baseline", verbose=args.verbose))

    print_markdown(results)

    # D1: Write structured result file
    rw = ResultWriter("temporal_eval", results[0].system if results else "unknown")
    all_pass = True
    for r in results:
        prefix = f"{r.system}_{r.profile}"
        rw.metric(f"{prefix}_accuracy", r.accuracy)
        rw.metric(f"{prefix}_stale_rate", r.stale_rate)
        rw.metric(f"{prefix}_p50_ms", r.p50_ms)
        rw.metric(f"{prefix}_p95_ms", r.p95_ms)
        rw.metric(f"{prefix}_errors", float(r.errors))

        if r.profile == "temporal":
            rw.gate(f"{prefix}_accuracy", r.accuracy, ACCURACY_GATE)
            rw.gate(
                f"{prefix}_stale_rate",
                r.stale_rate,
                STALE_GATE,
                passed=r.stale_rate <= STALE_GATE,
            )
            rw.gate(
                f"{prefix}_p95_ms",
                r.p95_ms,
                P95_GATE_MS,
                passed=r.p95_ms <= P95_GATE_MS,
            )

    for g in rw._result.gates.values():
        if not g["passed"]:
            all_pass = False

    out_path = rw.write(Path(args.output) if args.output else None)
    print(f"\nResult written to: {out_path}")

    sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()
