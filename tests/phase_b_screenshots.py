#!/usr/bin/env python3
"""Phase B Dashboard — Playwright screenshot validation.

Seeds test data, navigates every dashboard page, captures screenshots.
Server must be running on localhost:8080 with MNEMO_AUTH_ENABLED=false.
"""

import json, time, sys, os, requests

BASE = "http://localhost:8080"
SHOTS = os.path.join(os.path.dirname(__file__), "..", "screenshots")
os.makedirs(SHOTS, exist_ok=True)


def seed():
    """Seed a webhook and some memory so the dashboard has data to show."""
    # Write memories first (webhook creation requires user to exist)
    for i in range(3):
        mr = requests.post(
            f"{BASE}/api/v1/memory",
            json={
                "user": "test-user",
                "text": f"Test memory entry {i + 1}: The capital of France is Paris.",
                "session": "demo-session",
            },
        )
        print(f"  Memory {i + 1}: {mr.status_code}")

    # Create a webhook
    r = requests.post(
        f"{BASE}/api/v1/memory/webhooks",
        json={
            "user": "test-user",
            "target_url": "https://example.com/hooks/test",
            "events": ["fact_added", "fact_superseded", "conflict_detected"],
            "signing_secret": "whsec_test_123",
        },
    )
    wh_id = (
        r.json().get("webhook", {}).get("id") if r.status_code in (200, 201) else None
    )
    print(f"  Webhook: {r.status_code} id={wh_id}")

    # Create a policy
    r = requests.put(
        f"{BASE}/api/v1/policies/test-user",
        json={
            "retention_days_message": 90,
            "retention_days_text": 180,
            "retention_days_json": 365,
            "webhook_domain_allowlist": ["example.com"],
            "default_memory_contract": "default",
            "default_retrieval_policy": "balanced",
        },
    )
    print(f"  Policy: {r.status_code}")
    return wh_id


def take_screenshots():
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        ctx = browser.new_context(viewport={"width": 1400, "height": 900})
        results = {}

        def screenshot_page(name, url, setup_fn=None, wait_ms=2000):
            """Open a fresh page, navigate, optionally do setup, screenshot, close."""
            page = ctx.new_page()
            try:
                page.goto(url, timeout=10000)
                page.wait_for_timeout(wait_ms)
                if setup_fn:
                    setup_fn(page)
                path = f"{SHOTS}/{name}.png"
                page.screenshot(path=path, full_page=True)
                sz = os.path.getsize(path)
                results[name] = sz
                print(f"  {name}.png ({sz:,} bytes)")
            except Exception as e:
                print(f"  {name}: FAILED — {e}")
                results[name] = 0
            finally:
                page.close()

        # 1. Home page
        print("  [1/7] Home...")
        screenshot_page("01_home", f"{BASE}/_/", wait_ms=3000)

        # 2. Webhooks grid
        print("  [2/7] Webhooks grid...")

        def setup_webhooks(page):
            page.click('[data-page="webhooks"]')
            page.wait_for_timeout(2000)

        screenshot_page("02_webhooks_grid", f"{BASE}/_/", setup_fn=setup_webhooks)

        # 3. Webhook detail
        print("  [3/7] Webhook detail...")

        def setup_wh_detail(page):
            page.click('[data-page="webhooks"]')
            page.wait_for_timeout(2000)
            rows = page.query_selector_all(".clickable-row[data-wh-id]")
            if rows:
                rows[0].click()
                page.wait_for_timeout(2000)

        screenshot_page("03_webhook_detail", f"{BASE}/_/", setup_fn=setup_wh_detail)

        # 4. RCA form
        print("  [4/7] RCA form...")

        def setup_rca_form(page):
            page.click('[data-page="rca"]')
            page.wait_for_timeout(500)
            page.fill("#rca-user", "test-user")
            page.fill("#rca-query", "What is the capital of France?")
            now = time.strftime("%Y-%m-%dT%H:%M", time.localtime())
            one_hour_ago = time.strftime(
                "%Y-%m-%dT%H:%M", time.localtime(time.time() - 3600)
            )
            page.fill("#rca-from", one_hour_ago)
            page.fill("#rca-to", now)

        screenshot_page("04_rca_form", f"{BASE}/_/", setup_fn=setup_rca_form)

        # 5. RCA results
        print("  [5/7] RCA results...")

        def setup_rca_results(page):
            page.click('[data-page="rca"]')
            page.wait_for_timeout(500)
            page.fill("#rca-user", "test-user")
            page.fill("#rca-query", "What is the capital of France?")
            now = time.strftime("%Y-%m-%dT%H:%M", time.localtime())
            one_hour_ago = time.strftime(
                "%Y-%m-%dT%H:%M", time.localtime(time.time() - 3600)
            )
            page.fill("#rca-from", one_hour_ago)
            page.fill("#rca-to", now)
            page.click('#rca-form button[type="submit"]')
            page.wait_for_timeout(5000)

        screenshot_page(
            "05_rca_results", f"{BASE}/_/", setup_fn=setup_rca_results, wait_ms=1000
        )

        # 6. Governance
        print("  [6/7] Governance...")

        def setup_gov(page):
            page.click('[data-page="governance"]')
            page.wait_for_timeout(500)
            page.fill("#gov-user", "test-user")
            page.click("#gov-load-btn")
            page.wait_for_timeout(2000)

        screenshot_page("06_governance", f"{BASE}/_/", setup_fn=setup_gov)

        # 7. Traces form (skip lookup — endpoint hangs with 43+ users in DB)
        print("  [7/7] Traces form...")

        def setup_traces(page):
            page.click('[data-page="traces"]')
            page.wait_for_timeout(500)
            page.fill("#trace-request-id", "00000000-0000-0000-0000-000000000000")

        screenshot_page("07_traces_form", f"{BASE}/_/", setup_fn=setup_traces)

        # 8. Explorer
        print("  [8/8] Explorer...")

        def setup_explorer(page):
            page.click('[data-page="explorer"]')
            page.wait_for_timeout(500)
            page.fill("#explorer-user", "test-user")
            page.click("#explorer-load-btn")
            page.wait_for_timeout(3000)

        screenshot_page("08_explorer", f"{BASE}/_/", setup_fn=setup_explorer)

        browser.close()
        return results


def main():
    try:
        r = requests.get(f"{BASE}/healthz", timeout=5)
        print(f"Server health: {r.json()}")
    except Exception as e:
        print(f"Server not reachable: {e}")
        sys.exit(1)

    print("\nSeeding test data...")
    seed()
    print("\nWaiting 3s for ingestion...")
    time.sleep(3)

    print("\nTaking screenshots...")
    results = take_screenshots()

    print(
        f"\nDone! {len([v for v in results.values() if v > 0])}/{len(results)} screenshots captured."
    )


if __name__ == "__main__":
    main()
