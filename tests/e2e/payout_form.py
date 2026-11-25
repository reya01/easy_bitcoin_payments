#!/usr/bin/env python3
"""Settings, "Where your money goes", with the server mocked, at phone width.

It must: show the current address with a type pill, in groups of four that copy
without spaces; offer Ark and On-chain as two cards with a clear selected one
(Ark first, recommended); change the label and example with the card; select
the matching card by itself when an address of the other kind is pasted, and
say so; send exactly the typed address when saved; and keep the safety-delay
wording. No script errors, no Content-Security-Policy errors, no sideways scroll.
Run: python3 tests/e2e/payout_form.py [screenshot-dir]
"""
import json, os, sys
from playwright.sync_api import sync_playwright
from page_assets import HEADERS, serve_asset, watch_csp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "dashboard.html")).read()
ORIGIN = "http://ebp.test"
SHOTS = sys.argv[1] if len(sys.argv) > 1 else ""
# Made-up addresses of the right shape (never real ones).
ARK = "ark1qqexampleexampleexampleexampleexampleexampleexampleexample00"
BC1 = "bc1qexampleexampleexampleexampleexampl0"


def main():
    problems, errs, posts = [], [], []
    with sync_playwright() as pw:
        b = pw.chromium.launch()
        p = b.new_context(viewport={"width": 390, "height": 1400}).new_page()
        p.on("pageerror", lambda e: errs.append(str(e)))
        watch_csp(p, errs)

        def handler(route):
            req = route.request
            path = req.url[len(ORIGIN):].split("?")[0]
            if path in ("/admin", "/admin/"):
                return route.fulfill(status=200, content_type="text/html", body=HTML, headers=HEADERS)
            if serve_asset(route, path):
                return
            j = lambda o: route.fulfill(status=200, content_type="application/json", body=json.dumps(o))
            if path == "/admin/session":
                return j({"authenticated": True, "csrf": "tok", "password_set": True, "twofa": False})
            if path == "/api/v1/payout":
                return j({"address": ARK, "kind": "ark", "live": True, "sweep_min_sats": 1500, "sweep_min_floor_sats": 1500})
            if path == "/admin/payout":
                posts.append(json.loads(req.post_data or "{}"))
                return j({"applied_now": False, "change_delay_hours": 24})
            if path == "/api/v1/admin/status":
                return j({"checks": [], "counters": {}, "wallet": {}, "mode": "live", "usd_per_btc": 67000})
            if path.startswith("/api/v1/"):
                return j([])
            return j({})

        p.route(ORIGIN + "/**", handler)
        p.goto(ORIGIN + "/admin"); p.wait_for_timeout(600)
        p.click('#tabs button[data-v="set"]'); p.wait_for_timeout(400)
        p.click("#fold-payout summary"); p.wait_for_timeout(300)

        if p.inner_text("#po-kind-pill").strip() != "Ark": problems.append("the current address needs an 'Ark' pill")
        chunks = p.eval_on_selector_all("#po-now .chunk", "cs => cs.map(c => c.textContent)")
        if "".join(chunks) != ARK or any(len(c) > 4 for c in chunks): problems.append(f"the address must show in groups of four: {chunks[:3]}")
        if p.inner_text("#po-now").replace("\n", "") != ARK: problems.append("the shown address must read (and copy) without spaces")
        ids = p.eval_on_selector_all("#po-kinds .po-kind", "bs => bs.map(b => b.id)")
        if ids != ["po-kind-ark", "po-kind-onchain"]: problems.append(f"two cards, Ark first: {ids}")
        if p.get_attribute("#po-kind-ark", "aria-checked") != "true": problems.append("Ark starts selected")
        if "Recommended" not in p.inner_text("#po-kind-ark"): problems.append("Ark is marked Recommended")
        if "privacy" not in p.inner_text("#po-kind-onchain"): problems.append("the on-chain card keeps the privacy trade-off")
        if "ark1" not in p.inner_text("#po-hint"): problems.append("the hint names how an Ark address starts")

        p.click("#po-kind-onchain"); p.wait_for_timeout(100)
        if p.get_attribute("#po-kind-onchain", "aria-checked") != "true": problems.append("tapping On-chain selects it")
        if "bc1" not in p.inner_text("#po-hint") or p.inner_text("#po-addr-label") != "Your Bitcoin address": problems.append("the label and hint follow the card")

        # Paste an Ark address while On-chain is selected: Ark selects itself, and it says so.
        p.fill("#po-addr", ARK); p.wait_for_timeout(100)
        if p.get_attribute("#po-kind-ark", "aria-checked") != "true": problems.append("pasting an Ark address selects Ark")
        if "Ark is now selected" not in p.inner_text("#po-switched"): problems.append("the switch is explained")
        p.fill("#po-addr", BC1); p.wait_for_timeout(100)
        if p.get_attribute("#po-kind-onchain", "aria-checked") != "true": problems.append("pasting an on-chain address selects On-chain")

        if "takes effect after a delay" not in p.inner_text("#po-form"): problems.append("the safety delay is explained before saving")
        p.fill("#po-pw", "a good long password"); p.click("#po-go"); p.wait_for_timeout(300)
        if not posts or posts[-1].get("address") != BC1: problems.append(f"the typed address is sent as is: {posts}")
        if "takes effect in about 24 hours" not in p.inner_text("#po-msg"): problems.append("the saved message keeps the delay wording")

        if SHOTS:
            os.makedirs(SHOTS, exist_ok=True)
            p.fill("#po-addr", ""); p.click("#po-kind-ark"); p.wait_for_timeout(100)
            p.locator("#fold-payout").screenshot(path=os.path.join(SHOTS, "payout.png"))
        if p.evaluate("document.documentElement.scrollWidth - window.innerWidth") > 0: problems.append("the page scrolls sideways")
        b.close()
    if errs: problems.append(f"script or policy errors: {errs}")
    print(("FAIL  " if problems else "PASS  ") + "payout form: current address, two cards, auto-select, save" + ("  -> " + "; ".join(problems) if problems else ""))
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
