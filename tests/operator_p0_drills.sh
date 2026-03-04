#!/usr/bin/env bash

set -euo pipefail

bold() { printf "\033[1m%s\033[0m\n" "$1"; }
green() { printf "\033[32m%s\033[0m\n" "$1"; }

bold "=== Operator Drill A: Dead-letter recovery ==="
cargo test -p mnemo-server --test memory_api test_memory_webhook_dead_letter_and_stats_endpoint -- --test-threads=1
cargo test -p mnemo-server --test memory_api test_memory_webhook_replay_retry_and_audit_endpoints -- --test-threads=1
green "Dead-letter recovery drill tests passed"

bold "=== Operator Drill B: Why-answer-changed RCA ==="
cargo test -p mnemo-server --test memory_api test_time_travel_trace_reports_fact_shift_and_timeline -- --test-threads=1
cargo test -p mnemo-server --test memory_api test_time_travel_summary_reports_delta_counts -- --test-threads=1
cargo test -p mnemo-server --test memory_api test_trace_lookup_joins_episode_webhook_and_governance_records -- --test-threads=1
cargo test -p mnemo-server --test memory_api test_trace_lookup_supports_source_filters_and_limits -- --test-threads=1
green "Why-changed RCA drill tests passed"

bold "=== Operator Drill C: Governance misconfig detection ==="
cargo test -p mnemo-server --test memory_api test_policy_webhook_domain_allowlist_blocks_disallowed_target -- --test-threads=1
cargo test -p mnemo-server --test memory_api test_policy_preview_estimates_retention_impact -- --test-threads=1
cargo test -p mnemo-server --test memory_api test_policy_violations_endpoint_filters_by_window -- --test-threads=1
green "Governance drill tests passed"

bold "All operator P0 drill suites passed."
