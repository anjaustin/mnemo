#!/usr/bin/env python3
"""eval/compare.py — diff two D1 result JSON files and detect regressions.

Usage:
    # Compare two specific files
    python eval/compare.py baseline.json current.json

    # Fail CI if any metric regresses beyond tolerance
    python eval/compare.py baseline.json current.json --fail-on-regression

    # Find the most recent result file automatically
    python eval/compare.py --latest

Tolerances (D2 spec):
    Accuracy metrics:  2 percentage points (0.02 absolute)
    Latency metrics:   20% relative increase
    Gate pass/fail:    zero tolerance (pass → fail is always a regression)

Exit codes:
    0  — no regressions (or --fail-on-regression not set)
    1  — regressions detected (only when --fail-on-regression is set)
    2  — usage error
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

RESULTS_DIR = Path(__file__).parent / "results"

# ── Tolerances ─────────────────────────────────────────────────────────────────
ACCURACY_TOLERANCE = 0.02  # 2 percentage points
LATENCY_TOLERANCE = 0.20  # 20% relative
LATENCY_KEYWORDS = ("latency", "_ms", "p50", "p95", "p99")


def _is_latency_metric(name: str) -> bool:
    return any(kw in name for kw in LATENCY_KEYWORDS)


# ── Result file loading ────────────────────────────────────────────────────────


def load_result(path: Path) -> dict[str, Any]:
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def latest_result_file() -> Path:
    files = sorted(RESULTS_DIR.glob("*.json"), key=lambda p: p.stat().st_mtime)
    if not files:
        raise FileNotFoundError(f"No result files found in {RESULTS_DIR}")
    return files[-1]


def two_latest_result_files() -> tuple[Path, Path]:
    files = sorted(RESULTS_DIR.glob("*.json"), key=lambda p: p.stat().st_mtime)
    if len(files) < 2:
        raise FileNotFoundError(
            f"Need at least 2 result files in {RESULTS_DIR}, found {len(files)}"
        )
    return files[-2], files[-1]


# ── Comparison logic ───────────────────────────────────────────────────────────


@dataclass
class MetricDiff:
    name: str
    baseline: float
    current: float
    delta: float
    delta_pct: float
    is_latency: bool
    regressed: bool
    status: str  # "improved", "regressed", "unchanged", "new"


def compare_metrics(
    baseline: dict[str, Any],
    current: dict[str, Any],
) -> list[MetricDiff]:
    b_metrics: dict[str, float] = baseline.get("metrics", {})
    c_metrics: dict[str, float] = current.get("metrics", {})
    b_gates: dict[str, Any] = baseline.get("gates", {})
    c_gates: dict[str, Any] = current.get("gates", {})

    all_keys = sorted(set(b_metrics) | set(c_metrics))
    diffs: list[MetricDiff] = []

    for key in all_keys:
        if key not in b_metrics:
            diffs.append(
                MetricDiff(
                    name=key,
                    baseline=float("nan"),
                    current=c_metrics[key],
                    delta=float("nan"),
                    delta_pct=float("nan"),
                    is_latency=_is_latency_metric(key),
                    regressed=False,
                    status="new",
                )
            )
            continue

        b_val = b_metrics[key]
        c_val = c_metrics.get(key, float("nan"))
        if c_val != c_val:  # nan check
            continue

        delta = c_val - b_val
        delta_pct = (delta / b_val) if b_val != 0 else 0.0
        is_lat = _is_latency_metric(key)

        # Gate regression: baseline passed, current failed
        gate_regressed = False
        if key in b_gates and key in c_gates:
            gate_regressed = b_gates[key]["passed"] and not c_gates[key]["passed"]

        # Metric regression
        if gate_regressed:
            metric_regressed = True
        elif is_lat:
            # Latency: regression if current is >20% higher
            metric_regressed = delta_pct > LATENCY_TOLERANCE
        else:
            # Accuracy: regression if current is >2pp lower
            metric_regressed = delta < -ACCURACY_TOLERANCE

        if gate_regressed or metric_regressed:
            status = "REGRESSED"
        elif is_lat and delta < 0:
            status = "improved"
        elif not is_lat and delta > ACCURACY_TOLERANCE / 2:
            status = "improved"
        elif abs(delta) < 1e-9:
            status = "unchanged"
        else:
            status = "changed"

        diffs.append(
            MetricDiff(
                name=key,
                baseline=b_val,
                current=c_val,
                delta=delta,
                delta_pct=delta_pct,
                is_latency=is_lat,
                regressed=gate_regressed or metric_regressed,
                status=status,
            )
        )

    return diffs


# ── Formatting ────────────────────────────────────────────────────────────────


def _fmt_val(name: str, val: float) -> str:
    if val != val:
        return "—"
    if _is_latency_metric(name):
        return f"{val:.0f}ms"
    if 0.0 <= val <= 1.0 and (
        "accuracy" in name
        or "rate" in name
        or "precision" in name
        or "recall" in name
        or name.startswith("longmem_")
    ):
        return f"{val * 100:.1f}%"
    return f"{val:.4g}"


def _fmt_delta(d: MetricDiff) -> str:
    if d.status == "new":
        return "(new)"
    if d.delta != d.delta:
        return "—"
    sign = "+" if d.delta >= 0 else ""
    if d.is_latency:
        return f"{sign}{d.delta:.0f}ms ({sign}{d.delta_pct * 100:.1f}%)"
    if abs(d.delta) < 0.001:
        return f"{sign}{d.delta * 100:.2f}pp"
    return f"{sign}{d.delta * 100:.1f}pp"


def print_comparison(
    diffs: list[MetricDiff],
    baseline_path: Path,
    current_path: Path,
    baseline_meta: dict[str, Any],
    current_meta: dict[str, Any],
) -> None:
    print(
        f"\nBaseline : {baseline_path.name}  "
        f"({baseline_meta.get('commit', '?')}@{baseline_meta.get('branch', '?')}  "
        f"{baseline_meta.get('timestamp', '?')[:10]})"
    )
    print(
        f"Current  : {current_path.name}  "
        f"({current_meta.get('commit', '?')}@{current_meta.get('branch', '?')}  "
        f"{current_meta.get('timestamp', '?')[:10]})"
    )
    print()

    col_w = (36, 12, 12, 18, 12)
    header = (
        f"{'Metric':<{col_w[0]}}  "
        f"{'Baseline':>{col_w[1]}}  "
        f"{'Current':>{col_w[2]}}  "
        f"{'Delta':>{col_w[3]}}  "
        f"{'Status':<{col_w[4]}}"
    )
    sep = "  ".join("-" * w for w in col_w)
    print(header)
    print(sep)

    regressions = 0
    for d in diffs:
        status_label = d.status.upper() if d.regressed else d.status
        if d.regressed:
            regressions += 1
        line = (
            f"{d.name:<{col_w[0]}}  "
            f"{_fmt_val(d.name, d.baseline):>{col_w[1]}}  "
            f"{_fmt_val(d.name, d.current):>{col_w[2]}}  "
            f"{_fmt_delta(d):>{col_w[3]}}  "
            f"{status_label:<{col_w[4]}}"
        )
        print(line)

    print(sep)
    if regressions == 0:
        print("No regressions detected.")
    else:
        print(f"{regressions} regression(s) detected.")
    print()


# ── GitHub Step Summary ────────────────────────────────────────────────────────


def write_github_summary(
    diffs: list[MetricDiff],
    baseline_meta: dict[str, Any],
    current_meta: dict[str, Any],
) -> None:
    """Write a Markdown summary to $GITHUB_STEP_SUMMARY if running in CI."""
    import os

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return

    regressions = [d for d in diffs if d.regressed]
    status_emoji = "✅" if not regressions else "❌"

    lines = [
        f"## {status_emoji} Eval Regression Report\n",
        f"**Baseline:** `{baseline_meta.get('commit', '?')}`"
        f" @ `{baseline_meta.get('branch', '?')}`  ",
        f"**Current:** `{current_meta.get('commit', '?')}`"
        f" @ `{current_meta.get('branch', '?')}`\n",
        "| Metric | Baseline | Current | Delta | Status |",
        "|---|---:|---:|---:|---|",
    ]
    for d in diffs:
        status = (
            "🔴 REGRESSED"
            if d.regressed
            else ("🟢 improved" if d.status == "improved" else d.status)
        )
        lines.append(
            f"| `{d.name}` | {_fmt_val(d.name, d.baseline)} | "
            f"{_fmt_val(d.name, d.current)} | {_fmt_delta(d)} | {status} |"
        )
    if not regressions:
        lines.append("\n**No regressions detected.**")
    else:
        lines.append(f"\n**{len(regressions)} regression(s) detected.**")

    with open(summary_path, "a", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


# ── Main ───────────────────────────────────────────────────────────────────────


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Compare two D1 eval result files and detect regressions"
    )
    parser.add_argument("baseline", nargs="?", help="Baseline result JSON file")
    parser.add_argument("current", nargs="?", help="Current result JSON file")
    parser.add_argument(
        "--latest",
        action="store_true",
        help="Auto-select the two most recent files in eval/results/",
    )
    parser.add_argument(
        "--fail-on-regression",
        action="store_true",
        help="Exit 1 if any metric regresses beyond tolerance",
    )
    args = parser.parse_args()

    if args.latest:
        try:
            baseline_path, current_path = two_latest_result_files()
        except FileNotFoundError as e:
            print(f"ERROR: {e}", file=sys.stderr)
            sys.exit(2)
    elif args.baseline and args.current:
        baseline_path = Path(args.baseline)
        current_path = Path(args.current)
    else:
        parser.print_help()
        sys.exit(2)

    try:
        baseline_data = load_result(baseline_path)
        current_data = load_result(current_path)
    except (FileNotFoundError, json.JSONDecodeError) as e:
        print(f"ERROR loading result files: {e}", file=sys.stderr)
        sys.exit(2)

    diffs = compare_metrics(baseline_data, current_data)
    print_comparison(diffs, baseline_path, current_path, baseline_data, current_data)
    write_github_summary(diffs, baseline_data, current_data)

    regressions = [d for d in diffs if d.regressed]
    if args.fail_on_regression and regressions:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
