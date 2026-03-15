#!/usr/bin/env python3
"""mnemo_eval CLI entry point.

Usage examples:

    # Run all packs against a local Mnemo instance
    python -m mnemo_eval --backend mnemo --base-url http://localhost:8080 --packs all

    # Run specific packs
    python -m mnemo_eval --backend mnemo --base-url http://localhost:8080 --packs temporal,longmem

    # Run against Zep
    python -m mnemo_eval --backend zep --base-url https://api.getzep.com/api/v2 \\
        --zep-api-key $ZEP_API_KEY --packs temporal,longmem

    # List available packs
    python -m mnemo_eval --list-packs

    # After pip install:
    mnemo-eval --backend mnemo --base-url http://localhost:8080 --packs all

Exit codes:
    0  — all gate checks passed (or no gates ran)
    1  — one or more gate checks failed
    2  — usage error or configuration problem
"""

from __future__ import annotations

import argparse
import importlib
import subprocess
import sys
from pathlib import Path
from typing import Any

# Add the eval/ directory to the path so lib.py and harness scripts are importable
_EVAL_DIR = Path(__file__).parent.parent
if str(_EVAL_DIR) not in sys.path:
    sys.path.insert(0, str(_EVAL_DIR))

# ── Available packs and their harness scripts ──────────────────────────────────

#: Maps pack name → harness config.
#:
#: "script"      : filename relative to eval/
#: "description" : shown by --list-packs
#: "args"        : additional fixed args always appended
#: "url_flag"    : the flag that accepts the backend base URL
#:                 (harnesses differ: --mnemo-base-url vs --server)
#: "target_flag" : True if the harness accepts --target (default False)
#: "verbose_flag": True if the harness accepts --verbose (default True)
_PACKS: dict[str, dict[str, Any]] = {
    "temporal": {
        "script": "temporal_eval.py",
        "description": "27 temporal retrieval cases (recency, point-in-time, stale fact detection)",
        "args": [],
        "url_flag": "--mnemo-base-url",
        "target_flag": True,
        "verbose_flag": True,
    },
    "scientific": {
        "script": "temporal_eval.py",
        "description": "10 scientific research cases (v2)",
        "args": ["--cases", str(_EVAL_DIR / "scientific_research_cases_v2.json")],
        "url_flag": "--mnemo-base-url",
        "target_flag": True,
        "verbose_flag": True,
    },
    "longmem": {
        "script": "longmem_eval.py",
        "description": "LongMemEval: single-hop, multi-hop, temporal, preference, absent (5 task types)",
        "args": [],
        "url_flag": "--mnemo-base-url",
        "target_flag": False,  # longmem_eval.py has no --target flag
        "verbose_flag": True,
    },
    "recall": {
        "script": "recall_quality.py",
        "description": "40 gold facts: factual recall accuracy, temporal correctness, p95 latency",
        "args": [],
        "url_flag": "--server",  # recall_quality.py uses --server not --mnemo-base-url
        "target_flag": False,
        "verbose_flag": False,  # recall_quality.py has no --verbose flag
    },
    "latency": {
        "script": "latency_bench.py",
        "description": "E2E latency benchmarks at small/medium/large scale (10/100/1000 episodes)",
        "args": [],
        "url_flag": "--mnemo-base-url",
        "target_flag": False,
        "verbose_flag": True,
    },
}

_ALL_PACKS = list(_PACKS.keys())


def _build_pack_args(pack_name: str, args: argparse.Namespace) -> list[str]:
    """Build the harness-specific CLI args for *one* pack.

    Each harness has its own flag conventions; use the per-pack metadata to
    map the CLI's --backend / --base-url to the correct flags.
    """
    pack = _PACKS[pack_name]
    result: list[str] = []

    url_flag = pack.get("url_flag", "--mnemo-base-url")
    has_target = pack.get("target_flag", False)

    if args.backend == "mnemo":
        if has_target:
            result += ["--target", "mnemo"]
        result += [url_flag, args.base_url]

    elif args.backend == "zep":
        if has_target:
            result += ["--target", "zep"]
            result += ["--zep-base-url", args.zep_base_url]
            if args.zep_api_key:
                result += ["--zep-api-key", args.zep_api_key]
        else:
            # Harness doesn't support Zep — skip it
            print(
                f"WARNING: pack '{pack_name}' does not support --backend zep; skipping.",
                file=sys.stderr,
            )

    elif args.backend == "both":
        if has_target:
            result += ["--target", "both"]
            result += ["--mnemo-base-url", args.base_url]
            result += ["--zep-base-url", args.zep_base_url]
            if args.zep_api_key:
                result += ["--zep-api-key", args.zep_api_key]
        else:
            # Fall back to Mnemo-only for harnesses that don't support --target
            result += [url_flag, args.base_url]

    elif args.backend == "custom":
        print(
            f"WARNING: custom backends are not yet supported via the CLI entry point.\n"
            f"Run harnesses directly:\n"
            f"  python eval/{pack['script']} {url_flag} http://your-system\n",
            file=sys.stderr,
        )

    return result


def _run_pack(
    pack_name: str,
    backend_args: list[str],
    extra_args: list[str],
    output_dir: Path | None,
    verbose: bool,
) -> int:
    """Run one pack's harness script as a subprocess. Returns its exit code."""
    pack = _PACKS[pack_name]
    script = _EVAL_DIR / pack["script"]

    if not script.exists():
        print(f"ERROR: harness script not found: {script}", file=sys.stderr)
        return 2

    cmd = [sys.executable, str(script)] + backend_args + pack["args"] + extra_args

    if output_dir:
        output_dir.mkdir(parents=True, exist_ok=True)
        cmd += ["--output", str(output_dir / f"{pack_name}_result.json")]

    if verbose and pack.get("verbose_flag", True):
        cmd += ["--verbose"]

    print(f"\n{'=' * 60}")
    print(f"Pack: {pack_name}  ({pack['description']})")
    print(f"{'=' * 60}")

    result = subprocess.run(cmd, cwd=str(_EVAL_DIR))
    return result.returncode


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="mnemo-eval",
        description="Run AI memory system eval packs against any backend",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )

    # Backend selection
    parser.add_argument(
        "--backend",
        choices=["mnemo", "zep", "both", "custom"],
        default="mnemo",
        help="Memory backend to evaluate (default: mnemo)",
    )
    parser.add_argument(
        "--base-url",
        default="http://localhost:8080",
        help="Base URL for the memory system API (default: http://localhost:8080)",
    )
    parser.add_argument(
        "--zep-base-url",
        default="https://api.getzep.com/api/v2",
        help="Zep API base URL (for --backend zep or --backend both)",
    )
    parser.add_argument(
        "--zep-api-key",
        default=None,
        help="Zep API key (or set ZEP_API_KEY env var)",
    )

    # Pack selection
    parser.add_argument(
        "--packs",
        default="temporal,longmem",
        help=(
            f"Comma-separated list of packs to run, or 'all'. "
            f"Available: {', '.join(_ALL_PACKS)} (default: temporal,longmem)"
        ),
    )
    parser.add_argument(
        "--list-packs",
        action="store_true",
        help="List available packs and exit",
    )

    # Output
    parser.add_argument(
        "--output-dir",
        default=None,
        help="Directory to write result JSON files (default: eval/results/)",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Pass --verbose to each harness for per-case output",
    )
    parser.add_argument(
        "--fail-fast",
        action="store_true",
        help="Stop after the first pack with a non-zero exit code",
    )

    args = parser.parse_args()

    if args.list_packs:
        print("Available packs:\n")
        for name, info in _PACKS.items():
            print(f"  {name:<16}  {info['description']}")
        print(f"\nUse --packs all to run all {len(_PACKS)} packs.")
        sys.exit(0)

    # Resolve pack list
    if args.packs == "all":
        selected_packs = _ALL_PACKS
    else:
        raw = [p.strip() for p in args.packs.split(",") if p.strip()]
        unknown = [p for p in raw if p not in _PACKS]
        if unknown:
            print(
                f"ERROR: unknown pack(s): {', '.join(unknown)}\n"
                f"Available: {', '.join(_ALL_PACKS)}",
                file=sys.stderr,
            )
            sys.exit(2)
        selected_packs = raw

    # Fill in env-var fallback for ZEP_API_KEY
    import os

    if args.zep_api_key is None:
        args.zep_api_key = os.environ.get("ZEP_API_KEY")

    output_dir = Path(args.output_dir) if args.output_dir else None

    print(f"mnemo-eval v{_version()}")
    print(f"Backend : {args.backend}")
    print(f"Base URL: {args.base_url}")
    print(f"Packs   : {', '.join(selected_packs)}")

    exit_codes: list[int] = []
    for pack_name in selected_packs:
        pack_backend_args = _build_pack_args(pack_name, args)
        code = _run_pack(
            pack_name=pack_name,
            backend_args=pack_backend_args,
            extra_args=[],
            output_dir=output_dir,
            verbose=args.verbose,
        )
        exit_codes.append(code)
        if args.fail_fast and code != 0:
            print(f"\nFail-fast: stopping after failed pack '{pack_name}'")
            break

    failed = [name for name, code in zip(selected_packs, exit_codes) if code != 0]

    print(f"\n{'=' * 60}")
    print(
        f"Results: {len(selected_packs) - len(failed)}/{len(selected_packs)} packs passed"
    )
    if failed:
        print(f"Failed : {', '.join(failed)}")
    print(f"{'=' * 60}\n")

    sys.exit(1 if failed else 0)


def _version() -> str:
    try:
        from mnemo_eval import __version__

        return __version__
    except Exception:
        return "0.1.0"


if __name__ == "__main__":
    main()
