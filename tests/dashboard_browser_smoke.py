#!/usr/bin/env python3
"""Browser-driven dashboard smoke for incident drilldowns.

Requires a running Mnemo server, Playwright for Python, and Firefox.
It seeds a failing webhook, a governance violation, and traceable request IDs,
then drives the embedded dashboard end-to-end in a real browser.
"""

from __future__ import annotations

import os
import time
import tempfile
from dataclasses import dataclass

import requests
from playwright.sync_api import sync_playwright, expect


BASE = os.environ.get("MNEMO_URL", "http://127.0.0.1:8080")
REQUEST_ID_HEADER = "x-mnemo-request-id"
FIREFOX_PATH = os.environ.get("PLAYWRIGHT_FIREFOX_PATH", "/usr/bin/firefox")
API_TIMEOUT = float(os.environ.get("MNEMO_BROWSER_SMOKE_TIMEOUT", "60"))


@dataclass
class Seeded:
    user: str
    governance_user_path: str
    webhook_id: str
    policy_request_id: str
    register_request_id: str
    replay_request_id: str
    violation_request_id: str


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


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
            require(
                download_info.value.suggested_filename.startswith(
                    "mnemo-webhook-evidence-"
                ),
                "webhook evidence export did not start",
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
            require(
                download_info.value.suggested_filename.startswith(
                    "mnemo-trace-evidence-"
                ),
                "trace evidence export did not start",
            )

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
            require(
                download_info.value.suggested_filename.startswith(
                    "mnemo-governance-evidence-"
                ),
                "governance evidence export did not start",
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
