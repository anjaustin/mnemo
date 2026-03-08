#!/usr/bin/env python3
"""Browser-driven dashboard smoke for incident drilldowns.

Requires a running Mnemo server, Playwright for Python, and Firefox.
It seeds a failing webhook, a governance violation, and traceable request IDs,
then drives the embedded dashboard end-to-end in a real browser.
"""

from __future__ import annotations

import os
import json
import time
import tempfile
from dataclasses import dataclass
from pathlib import Path

import requests
from playwright.sync_api import sync_playwright, expect


BASE = os.environ.get("MNEMO_URL", "http://127.0.0.1:8080")
REQUEST_ID_HEADER = "x-mnemo-request-id"
FIREFOX_PATH = os.environ.get("PLAYWRIGHT_FIREFOX_PATH", "/usr/bin/firefox")
API_TIMEOUT = float(os.environ.get("MNEMO_BROWSER_SMOKE_TIMEOUT", "60"))


@dataclass
class Seeded:
    user: str
    user_id: str
    session_id: str
    governance_user_path: str
    webhook_id: str
    policy_request_id: str
    register_request_id: str
    replay_request_id: str
    violation_request_id: str


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def read_download(download, download_dir: str) -> dict:
    target = Path(download_dir) / download.suggested_filename
    download.save_as(target)
    return json.loads(target.read_text())


def api(method: str, path: str, payload=None, request_id: str | None = None):
    headers = {}
    if request_id:
        headers[REQUEST_ID_HEADER] = request_id
    response = requests.request(
        method, f"{BASE}{path}", json=payload, headers=headers, timeout=API_TIMEOUT
    )
    return response


def seed() -> Seeded:
    suffix = str(int(time.time()))
    user = f"browser-smoke-{suffix}"
    replay_request_id = f"browser-replay-{suffix}"
    policy_request_id = f"browser-policy-{suffix}"
    violation_request_id = f"browser-violation-{suffix}"
    register_request_id = f"browser-register-{suffix}"

    print(f"seed: create user {user}")
    response = api(
        "POST",
        "/api/v1/users",
        {
            "name": user,
            "external_id": user,
            "metadata": {"suite": "dashboard-browser-smoke"},
        },
    )
    require(
        response.status_code == 201,
        f"user create failed: {response.status_code} {response.text}",
    )
    user_id = response.json()["id"]

    print("seed: register failing webhook")
    response = api(
        "POST",
        "/api/v1/memory/webhooks",
        {
            "user": user,
            "target_url": "http://127.0.0.1:9/browser-smoke-hook",
            "events": ["head_advanced"],
        },
        request_id=register_request_id,
    )
    require(
        response.status_code == 201,
        f"webhook register failed: {response.status_code} {response.text}",
    )
    webhook_id = response.json()["webhook"]["id"]

    print("seed: tighten governance allowlist")
    response = api(
        "PUT",
        f"/api/v1/policies/{user}",
        {
            "webhook_domain_allowlist": ["hooks.acme.example"],
            "default_memory_contract": "default",
            "default_retrieval_policy": "balanced",
        },
        request_id=policy_request_id,
    )
    require(
        response.ok, f"policy update failed: {response.status_code} {response.text}"
    )

    print("seed: force governance violation")
    response = api(
        "POST",
        "/api/v1/memory/webhooks",
        {
            "user": user,
            "target_url": "https://evil.example/browser-smoke",
            "events": ["head_advanced"],
        },
        request_id=violation_request_id,
    )
    require(
        response.status_code == 400,
        "expected governance violation when registering blocked webhook",
    )

    print("seed: record replay audit row")
    response = api(
        "GET",
        f"/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=5",
        request_id=replay_request_id,
    )
    require(
        response.ok, f"replay request failed: {response.status_code} {response.text}"
    )

    return Seeded(
        user=user,
        user_id=user_id,
        session_id="00000000-0000-0000-0000-00000000feed",
        governance_user_path=f"/_/governance/{user_id}",
        webhook_id=webhook_id,
        policy_request_id=policy_request_id,
        register_request_id=register_request_id,
        replay_request_id=replay_request_id,
        violation_request_id=violation_request_id,
    )


def run_browser(seed: Seeded) -> None:
    with tempfile.TemporaryDirectory(prefix="mnemo-browser-smoke-") as download_dir:
        with sync_playwright() as p:
            try:
                browser = p.chromium.launch(headless=True)
            except Exception:
                browser = p.firefox.launch(headless=True, executable_path=FIREFOX_PATH)
            context = browser.new_context(
                viewport={"width": 1600, "height": 1100}, accept_downloads=True
            )
            page = context.new_page()

            print("browser: open home")
            page.goto(f"{BASE}/_/", wait_until="domcontentloaded")
            page.wait_for_selector("#incident-panel")
            expect(page.locator("#card-incidents")).to_be_visible()

            print("browser: open dead-letter filtered webhooks")
            page.goto(
                f"{BASE}/_/webhooks?filter=dead-letter", wait_until="domcontentloaded"
            )
            page.wait_for_selector("#page-webhooks")
            expect(page.locator("#webhooks-grid")).to_contain_text(
                "No matching webhooks"
            )
            expect(page.locator("#webhooks-grid")).to_contain_text(
                "No webhooks currently match the dead letter incident filter"
            )

            print("browser: open unfiltered webhook detail")
            page.goto(f"{BASE}/_/webhooks", wait_until="domcontentloaded")
            row = page.locator(".clickable-row[data-wh-id]").first
            expect(row).to_be_visible()
            row.click()
            expect(page.locator("#wh-detail-content")).to_contain_text(
                "Export Evidence"
            )
            expect(page.locator("#wh-detail-content")).to_contain_text("req")

            print("browser: export webhook evidence bundle")
            with page.expect_download() as download_info:
                page.get_by_role("button", name="Export Evidence").first.click()
            webhook_bundle = read_download(download_info.value, download_dir)
            require(
                download_info.value.suggested_filename.startswith(
                    "mnemo-webhook-evidence-"
                ),
                "webhook evidence export did not start",
            )
            require(
                webhook_bundle.get("kind") == "webhook_evidence_bundle",
                "webhook evidence export returned unexpected payload",
            )
            require(
                webhook_bundle.get("source_path", "").startswith("/_/webhooks"),
                "webhook evidence export should preserve dashboard source path",
            )

            print("browser: follow webhook audit trace link")
            page.locator("#wh-detail-content a[data-trace-link]").first.click()
            page.wait_for_url(
                lambda url: "/_/traces/" in url and "focus=webhooks" in url
            )
            expect(page.locator("#trace-results")).to_be_visible()
            require(
                page.locator("#trace-chk-episodes").is_checked() is False,
                "webhook-focused trace should disable episodes",
            )
            require(
                page.locator("#trace-chk-webhooks").is_checked() is True,
                "webhook-focused trace should enable webhooks",
            )
            require(
                page.locator("#trace-chk-governance").is_checked() is False,
                "webhook-focused trace should disable governance",
            )
            require(
                page.locator("#trace-results").text_content() is not None,
                "trace results should render content",
            )
            expect(page.locator("#trace-results")).to_contain_text("Trace:")

            print("browser: export trace evidence bundle")
            with page.expect_download() as download_info:
                page.locator(
                    '#trace-results button:has-text("Export Evidence")'
                ).click()
            trace_bundle = read_download(download_info.value, download_dir)
            require(
                download_info.value.suggested_filename.startswith(
                    "mnemo-trace-evidence-"
                ),
                "trace evidence export did not start",
            )
            require(
                trace_bundle.get("kind") == "trace_evidence_bundle",
                "trace evidence export returned unexpected payload",
            )
            require(
                trace_bundle.get("payload", {}).get("focus") == "webhooks",
                "trace evidence export should preserve webhook focus",
            )

            print("browser: render synthetic episode graph node")
            page.evaluate(
                """
                seed => {
                  history.pushState({}, '', `/_/traces/${seed.replay_request_id}`);
                  renderTraceResults({
                    request_id: seed.replay_request_id,
                    matched_episodes: [{
                      user_id: seed.user_id,
                      session_id: seed.session_id,
                      episode_id: 'synthetic-episode-id',
                      created_at: '2026-03-08T12:00:00Z',
                      preview: 'synthetic episode preview'
                    }],
                    matched_webhook_events: [],
                    matched_webhook_audit: [],
                    matched_governance_audit: [],
                    summary: {
                      episode_matches: 1,
                      webhook_event_matches: 0,
                      webhook_audit_matches: 0,
                      governance_audit_matches: 0
                    }
                  });
                }
                """,
                {
                    "replay_request_id": seed.replay_request_id,
                    "user_id": seed.user_id,
                    "session_id": seed.session_id,
                },
            )

            print("browser: click episode graph node drilldown")
            page.eval_on_selector(
                '#trace-evidence-graph .trace-graph-node.interactive[data-kind="episode"]',
                "el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))",
            )
            page.wait_for_url(
                lambda url: "/_/rca?" in url
                and "episode_id=synthetic-episode-id" in url
            )
            expect(page.locator("#rca-focus-banner")).to_contain_text(
                "Episode RCA Focus"
            )
            require(
                page.locator("#rca-user").input_value() == seed.user_id,
                "episode drilldown should prefill RCA user",
            )
            require(
                page.locator("#rca-session").input_value() == seed.session_id,
                "episode drilldown should prefill RCA session",
            )
            require(
                page.locator("#rca-query").input_value() == "synthetic episode preview",
                "episode drilldown should prefill RCA query",
            )

            print("browser: render synthetic webhook graph node")
            page.evaluate(
                """
                seed => {
                  history.pushState({}, '', `/_/traces/${seed.replay_request_id}?focus=webhooks`);
                  renderTraceResults({
                    request_id: seed.replay_request_id,
                    matched_episodes: [],
                    matched_webhook_events: [{
                      id: 'synthetic-webhook-event',
                      webhook_id: seed.webhook_id,
                      event_type: 'head_advanced',
                      delivered: false,
                      dead_letter: false,
                      created_at: new Date().toISOString()
                    }],
                    matched_webhook_audit: [],
                    matched_governance_audit: [],
                    summary: {
                      episode_matches: 0,
                      webhook_event_matches: 1,
                      webhook_audit_matches: 0,
                      governance_audit_matches: 0
                    }
                  });
                }
                """,
                {
                    "replay_request_id": seed.replay_request_id,
                    "webhook_id": seed.webhook_id,
                },
            )

            print("browser: click webhook graph node drilldown")
            page.eval_on_selector(
                '#trace-evidence-graph .trace-graph-node.interactive[data-kind="webhook_event"]',
                "el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))",
            )
            page.wait_for_url(lambda url: f"/_/webhooks/{seed.webhook_id}" in url)
            expect(page.locator("#wh-detail-content")).to_contain_text("Audit Log")

            print("browser: open governance drilldown")
            page.goto(
                f"{BASE}{seed.governance_user_path}", wait_until="domcontentloaded"
            )
            expect(page.locator("#gov-policy-panel")).to_contain_text(seed.user)
            expect(
                page.locator("#gov-violations-panel a[data-trace-link]")
            ).to_be_visible()
            expect(
                page.locator("#gov-audit-panel a[data-trace-link]").first
            ).to_be_visible()

            print("browser: export governance evidence bundle")
            with page.expect_download() as download_info:
                page.get_by_role("button", name="Export Evidence").first.click()
            governance_bundle = read_download(download_info.value, download_dir)
            require(
                download_info.value.suggested_filename.startswith(
                    "mnemo-governance-evidence-"
                ),
                "governance evidence export did not start",
            )
            require(
                governance_bundle.get("kind") == "governance_evidence_bundle",
                "governance evidence export returned unexpected payload",
            )
            require(
                governance_bundle.get("payload", {})
                .get("policy", {})
                .get("user_identifier")
                == seed.user,
                "governance evidence export should include the current user policy",
            )

            print("browser: follow governance audit trace link")
            page.locator(
                f'#gov-audit-panel a[data-trace-link="{seed.policy_request_id}"]'
            ).click()
            page.wait_for_url(
                lambda url: "/_/traces/" in url and "focus=governance" in url
            )
            require(
                page.locator("#trace-chk-episodes").is_checked() is False,
                "governance-focused trace should disable episodes",
            )
            require(
                page.locator("#trace-chk-webhooks").is_checked() is False,
                "governance-focused trace should disable webhooks",
            )
            require(
                page.locator("#trace-chk-governance").is_checked() is True,
                "governance-focused trace should enable governance",
            )
            expect(page.locator("#trace-results")).to_contain_text("Governance Audit")
            expect(page.locator("#trace-results")).to_contain_text(
                "Evidence Constellation"
            )
            graph_nodes = int(
                page.locator("#trace-evidence-graph").get_attribute("data-node-count")
                or "0"
            )
            require(
                graph_nodes >= 2,
                "governance-focused trace should render a non-trivial evidence graph",
            )

            print("browser: render synthetic governance graph node")
            page.evaluate(
                """
                seed => {
                  history.pushState({}, '', `/_/traces/${seed.policy_request_id}?focus=governance`);
                  renderTraceResults({
                    request_id: seed.policy_request_id,
                    matched_episodes: [],
                    matched_webhook_events: [],
                    matched_webhook_audit: [],
                    matched_governance_audit: [{
                      id: 'synthetic-governance-audit',
                      user_id: seed.user_id,
                      action: 'policy_updated',
                      at: new Date().toISOString(),
                      details: { synthetic: true }
                    }],
                    summary: {
                      episode_matches: 0,
                      webhook_event_matches: 0,
                      webhook_audit_matches: 0,
                      governance_audit_matches: 1
                    }
                  });
                }
                """,
                {
                    "policy_request_id": seed.policy_request_id,
                    "user_id": seed.user_id,
                },
            )

            print("browser: click governance graph node drilldown")
            page.eval_on_selector(
                '#trace-evidence-graph .trace-graph-node.interactive[data-kind="governance"]',
                "el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))",
            )
            page.wait_for_url(lambda url: seed.governance_user_path in url)
            expect(page.locator("#gov-policy-panel")).to_contain_text(seed.user)

            browser.close()


def main() -> int:
    try:
        response = requests.get(f"{BASE}/health", timeout=5)
        require(
            response.ok, f"health check failed: {response.status_code} {response.text}"
        )
        seeded = seed()
        run_browser(seeded)
        print("dashboard browser smoke: PASS")
        return 0
    except Exception as exc:
        print(f"dashboard browser smoke: FAIL — {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
