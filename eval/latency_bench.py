#!/usr/bin/env python3
"""eval/latency_bench.py — E2E latency benchmark through the HTTP API.

Measures real retrieval latency end-to-end (HTTP + auth + reranking + context
assembly), not just storage layer latency. Three scale tiers: small (10
episodes), medium (100 episodes), large (1000 episodes).

Run:
    python eval/latency_bench.py [--mnemo-base-url URL] [--tiers small,medium]

Tier targets (D5 spec):
    small   10 episodes    p95 context < 100ms
    medium  100 episodes   p95 context < 300ms
    large   1000 episodes  p95 context < 1000ms

Exit code: 0 if all configured tiers pass, 1 if any fail.
"""

from __future__ import annotations

import argparse
import sys
import time
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from lib import HttpClient, MnemoBackend, ResultWriter, p_quantile  # noqa: E402

# ── Tier definitions ──────────────────────────────────────────────────────────

TIERS = {
    "small": {
        "episodes": 10,
        "n_queries": 20,
        "p95_ingest_ms": 500.0,
        "p95_context_ms": 100.0,
        "p95_entity_ms": 200.0,
    },
    "medium": {
        "episodes": 100,
        "n_queries": 20,
        "p95_ingest_ms": 500.0,
        "p95_context_ms": 300.0,
        "p95_entity_ms": 300.0,
    },
    "large": {
        "episodes": 1000,
        "n_queries": 20,
        "p95_ingest_ms": 1000.0,
        "p95_context_ms": 1000.0,
        "p95_entity_ms": 500.0,
    },
}

EPISODE_TEMPLATES = [
    "Alice works as a senior engineer at {company} and leads the platform team.",
    "The project deadline for {project} is end of Q{q} 2025.",
    "Last meeting with {name}: discussed budget, timeline, and scope.",
    "{name} prefers async communication and dislikes back-to-back meetings.",
    "Server region for {project}: {region}.",
    "API rate limit set to {n} requests per minute for {company}.",
    "The {company} contract was signed on March 1, 2025, for $220K.",
    "{name} is the engineering lead for {project}.",
    "Current sprint goal: ship {feature} by end of week.",
    "Team uses {tool} for project management and {tool2} for documentation.",
]

COMPANIES = [
    "Acme Corp",
    "Globex",
    "Initech",
    "Umbrella Corp",
    "Pinnacle Health",
    "Meridian Systems",
    "Vertex AI",
    "Soylent Industries",
    "Waystar",
    "Dunder Mifflin",
]
PROJECTS = [
    "Aurora",
    "Nova",
    "Orion",
    "Apex",
    "Zephyr",
    "Helios",
    "Atlas",
    "Nexus",
    "Titan",
    "Pegasus",
]
NAMES = [
    "Alice",
    "Bob",
    "Carol",
    "Dave",
    "Eve",
    "Frank",
    "Grace",
    "Heidi",
    "Ivan",
    "Judy",
]
REGIONS = ["us-east-1", "eu-west-1", "ap-southeast-2", "us-west-2"]
TOOLS = ["Jira", "Linear", "Asana", "Notion", "Confluence", "Shortcut"]


def _make_episode(i: int) -> str:
    import random

    rng = random.Random(i)
    tmpl = EPISODE_TEMPLATES[i % len(EPISODE_TEMPLATES)]
    return tmpl.format(
        company=rng.choice(COMPANIES),
        project=rng.choice(PROJECTS),
        name=rng.choice(NAMES),
        region=rng.choice(REGIONS),
        tool=rng.choice(TOOLS),
        tool2=rng.choice(TOOLS),
        feature=rng.choice(PROJECTS) + " feature",
        q=rng.randint(1, 4),
        n=rng.choice([100, 500, 1000, 5000]),
    )


QUERIES = [
    "What is Alice's current role?",
    "When is the next project deadline?",
    "What was discussed in the last meeting?",
    "What is the API rate limit?",
    "Which region is the server deployed in?",
    "What contract was signed recently?",
    "Who is the engineering lead?",
    "What tool does the team use for project management?",
    "What is the current sprint goal?",
    "What is the latest budget information?",
]


# ── Benchmark runner ──────────────────────────────────────────────────────────


def run_tier(
    backend: MnemoBackend,
    tier_name: str,
    config: dict,
    verbose: bool = False,
) -> dict:
    n_episodes = config["episodes"]
    n_queries = config["n_queries"]
    external_id = f"latbench-{tier_name}-{uuid.uuid4().hex[:8]}"
    user_id = ""
    session_id = ""

    ingest_latencies: list[float] = []
    context_latencies: list[float] = []
    entity_latencies: list[float] = []

    print(f"  [{tier_name}] Seeding {n_episodes} episodes...")
    try:
        user_id, session_id = backend.setup_user(external_id)

        # Ingest episodes and measure latency
        for i in range(n_episodes):
            content = _make_episode(i)
            t0 = time.perf_counter()
            ok = backend.ingest(user_id, session_id, content)
            lat = (time.perf_counter() - t0) * 1000.0
            ingest_latencies.append(lat)
            if not ok and verbose:
                print(f"  [{tier_name}] episode {i} ingest failed")

        # Brief wait for async indexing
        wait_s = min(3.0, n_episodes * 0.01)
        if verbose:
            print(f"  [{tier_name}] Waiting {wait_s:.1f}s for indexing...")
        time.sleep(wait_s)

        # Context retrieval latency
        print(f"  [{tier_name}] Running {n_queries} context queries...")
        for i in range(n_queries):
            q_text = QUERIES[i % len(QUERIES)]
            _, _, lat = backend.query(
                user_id,
                session_id,
                {"text": q_text},
                profile="baseline",
            )
            context_latencies.append(lat)

        # Entity lookup latency
        print(f"  [{tier_name}] Running {n_queries} entity lookups...")
        http = backend.http
        for _ in range(n_queries):
            t0 = time.perf_counter()
            http.req(f"/api/v1/users/{user_id}/entities", query={"limit": 20})
            lat = (time.perf_counter() - t0) * 1000.0
            entity_latencies.append(lat)

    finally:
        if user_id:
            try:
                backend.cleanup(user_id, session_id)
            except Exception:
                pass

    return {
        "tier": tier_name,
        "episodes": n_episodes,
        "n_queries": n_queries,
        "ingest_p50": p_quantile(ingest_latencies, 0.50),
        "ingest_p95": p_quantile(ingest_latencies, 0.95),
        "context_p50": p_quantile(context_latencies, 0.50),
        "context_p95": p_quantile(context_latencies, 0.95),
        "entity_p50": p_quantile(entity_latencies, 0.50),
        "entity_p95": p_quantile(entity_latencies, 0.95),
    }


def print_results(results: list[dict], configs: dict) -> list[bool]:
    cols = (8, 12, 12, 12, 12, 12, 12, 10)
    header = (
        f"{'Tier':<{cols[0]}}  "
        f"{'Ingest p50':>{cols[1]}}  {'Ingest p95':>{cols[2]}}  "
        f"{'Ctx p50':>{cols[3]}}  {'Ctx p95':>{cols[4]}}  "
        f"{'Ent p50':>{cols[5]}}  {'Ent p95':>{cols[6]}}  "
        f"{'Status':<{cols[7]}}"
    )
    sep = "  ".join("-" * c for c in cols)
    print(f"\n{'─' * 90}")
    print("E2E Latency Benchmark")
    print(f"{'─' * 90}")
    print(header)
    print(sep)

    passes = []
    for r in results:
        cfg = configs[r["tier"]]
        ctx_pass = r["context_p95"] <= cfg["p95_context_ms"]
        ent_pass = r["entity_p95"] <= cfg["p95_entity_ms"]
        tier_pass = ctx_pass and ent_pass
        passes.append(tier_pass)
        status = "PASS" if tier_pass else "FAIL"
        print(
            f"{r['tier']:<{cols[0]}}  "
            f"{r['ingest_p50']:>{cols[1]}.0f}ms  "
            f"{r['ingest_p95']:>{cols[2]}.0f}ms  "
            f"{r['context_p50']:>{cols[3]}.0f}ms  "
            f"{r['context_p95']:>{cols[4]}.0f}ms  "
            f"{r['entity_p50']:>{cols[5]}.0f}ms  "
            f"{r['entity_p95']:>{cols[6]}.0f}ms  "
            f"{status:<{cols[7]}}"
        )
    print(sep)
    print(
        f"Gates: context_p95 < [100ms/300ms/1000ms] per tier, entity_p95 < [200ms/300ms/500ms]\n"
    )
    return passes


# ── Main ──────────────────────────────────────────────────────────────────────


def main() -> None:
    parser = argparse.ArgumentParser(description="E2E latency benchmark for Mnemo")
    parser.add_argument("--mnemo-base-url", default="http://localhost:8080")
    parser.add_argument(
        "--tiers",
        default="small,medium",
        help="Comma-separated list of tiers to run: small, medium, large",
    )
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument(
        "--output",
        default=None,
        help="Path to write D1 JSON result file",
    )
    args = parser.parse_args()

    tier_names = [t.strip() for t in args.tiers.split(",") if t.strip() in TIERS]
    if not tier_names:
        print(
            "ERROR: no valid tiers specified. Use: small, medium, large",
            file=sys.stderr,
        )
        sys.exit(2)

    backend = MnemoBackend(args.mnemo_base_url)
    http = HttpClient(args.mnemo_base_url)
    status, body = http.req("/health")
    if status != 200:
        print(f"ERROR: server not healthy — {status} {body}", file=sys.stderr)
        sys.exit(1)
    print(f"Server healthy: version={body.get('version', 'unknown')}")
    print(f"Running tiers: {', '.join(tier_names)}\n")

    results = []
    for tier_name in tier_names:
        print(f"Tier: {tier_name} ({TIERS[tier_name]['episodes']} episodes)")
        r = run_tier(backend, tier_name, TIERS[tier_name], verbose=args.verbose)
        results.append(r)

    passes = print_results(results, TIERS)

    # D1: Write result file
    rw = ResultWriter("latency_bench", "mnemo")
    for r in results:
        t = r["tier"]
        rw.gate(
            f"{t}_context_p95_ms",
            r["context_p95"],
            TIERS[t]["p95_context_ms"],
            passed=r["context_p95"] <= TIERS[t]["p95_context_ms"],
        )
        rw.gate(
            f"{t}_entity_p95_ms",
            r["entity_p95"],
            TIERS[t]["p95_entity_ms"],
            passed=r["entity_p95"] <= TIERS[t]["p95_entity_ms"],
        )
        rw.metric(f"{t}_ingest_p50_ms", r["ingest_p50"])
        rw.metric(f"{t}_ingest_p95_ms", r["ingest_p95"])
        rw.metric(f"{t}_context_p50_ms", r["context_p50"])
        rw.metric(f"{t}_entity_p50_ms", r["entity_p50"])

    out_path = rw.write(Path(args.output) if args.output else None)
    print(f"Result written to: {out_path}")

    sys.exit(0 if all(passes) else 1)


if __name__ == "__main__":
    main()
