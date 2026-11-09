#!/usr/bin/env python3
"""Checkout page states, with the server mocked (no ebp binary needed).

Loads assets/checkout.html in a phone-sized browser and feeds it every answer the
server can give (pending, paid, overpaid, underpaid, expired, cancelled, on-chain
waiting, not found, connection errors, hostile text) and checks what the customer
sees: the right message, the pay details hidden once a payment is finished, no
script errors, no sideways scrolling.  Run: python3 tests/e2e/checkout_states.py
"""
import json, os, sys, time
from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HTML = open(os.path.join(ROOT, "assets", "checkout.html")).read()
QR = open(os.path.join(ROOT, "assets", "qr.js")).read()
ORIGIN = "http://ebp.test"
X = "<img src=x onerror=window.__xss=1>"


def invoice(**kw):
    d = {"return_url": "https://shop.example/order/7", "store_name": "Test Shop", "status": "pending",
         "amount_sats": 1500, "fiat_currency": "USD", "fiat_amount": "1.00",
         "ark_address": "tark1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq",
         "expires_at_unix": int(time.time()) + 600, "onchain_enabled": False, "onchain_address": None,
         "bip321": "bitcoin:?ark=tark1qqqq&amount=0.000015", "lightning": None, "test_mode": True}
    d.update(kw)
    return d


def run_case(pw, name, json_resp, statuses, checks, width=360):
    """json_resp: dict, 404, or 'abort'. statuses: list of dicts / codes / 'abort', used in order, last repeats."""
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": width, "height": 780})
    p = ctx.new_page()
    errs = []
    p.on("pageerror", lambda e: errs.append(str(e)))
    seen = {"n": 0}

    def handler(route):
        path = route.request.url[len(ORIGIN):]
        if path == "/assets/qr.js":
            return route.fulfill(status=200, content_type="application/javascript", body=QR)
        if path.endswith("/json"):
            if json_resp == "abort":
                return route.abort()
            if json_resp == 404:
                return route.fulfill(status=404, content_type="application/json", body='{"error":{"message":"not found"}}')
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(json_resp))
        if path.endswith("/status"):
            i = min(seen["n"], len(statuses) - 1)
            seen["n"] += 1
            r = statuses[i]
            if r == "abort":
                return route.abort()
            if isinstance(r, int):
                return route.fulfill(status=r, content_type="text/plain", body="oops")
            return route.fulfill(status=200, content_type="application/json", body=json.dumps(r))
        return route.fulfill(status=200, content_type="text/html", body=HTML)

    p.route(ORIGIN + "/**", handler)
    p.goto(ORIGIN + "/i/inv_test")
    p.wait_for_timeout(1500)
    api = {
        "status": lambda: p.inner_text("#status"),
        "card": lambda: p.is_visible("#pay-card"),
        "back": lambda: p.inner_text("#back-link") if p.is_visible("#back") else "",
        "overflow": lambda: p.evaluate("document.documentElement.scrollWidth - window.innerWidth"),
        "page": p,
    }
    problems = []
    try:
        checks(api, problems)
    except Exception as e:  # noqa: BLE001
        problems.append(f"check crashed: {e}")
    if errs:
        problems.append(f"script errors: {errs}")
    if api["overflow"]() > 0:
        problems.append(f"page scrolls sideways by {api['overflow']()}px")
    if p.evaluate("window.__xss||0"):
        problems.append("hostile text ran as script")
    b.close()
    print(("FAIL  " if problems else "PASS  ") + name + ("  -> " + "; ".join(problems) if problems else ""))
    return not problems


def has(api, problems, what, text, in_=None):
    got = api["status"]() if in_ is None else in_
    if text.lower() not in got.lower():
        problems.append(f"{what}: wanted '{text}', saw '{got[:90]}'")


def main():
    ok = True
    with sync_playwright() as pw:
        def c_pending(a, pr):
            has(a, pr, "waiting message", "Waiting for your payment")
            if not a["card"](): pr.append("pay details must be visible while pending")

        def c_paid(a, pr):
            has(a, pr, "paid message", "Paid. Thank you")
            if a["card"](): pr.append("pay details must go away once paid")
            has(a, pr, "return link", "Done", a["back"]())

        def c_over(a, pr):
            has(a, pr, "overpaid message", "little extra")
            if a["card"](): pr.append("pay details must go away once paid")

        def c_under(a, pr):
            has(a, pr, "underpaid message", "part of your payment")
            if not a["card"](): pr.append("pay details stay while part-paid")

        def c_expired(a, pr):
            has(a, pr, "expired message", "expired")
            if a["card"](): pr.append("pay details must go away once expired")
            has(a, pr, "back link", "try again", a["back"]())

        def c_cancelled(a, pr):
            has(a, pr, "cancelled message", "cancelled")
            if a["card"](): pr.append("pay details must go away once cancelled")
            has(a, pr, "back link", "try again", a["back"]())

        def c_chain(a, pr):
            has(a, pr, "waiting for confirmations", "0 of 1 confirmations")

        def c_nf(a, pr):
            has(a, pr, "not found message", "could not find")

        def c_net(a, pr):
            has(a, pr, "connection message", "check your connection")

        def c_recover(a, pr):
            a["page"].wait_for_timeout(4500)
            has(a, pr, "recovers after errors", "Paid. Thank you")

        def c_evil(a, pr):
            if a["back"]() and "javascript" in a["page"].get_attribute("#back-link", "href").lower():
                pr.append("a javascript: link was followed")
            has(a, pr, "shows the store name as text", X, a["page"].inner_text("#store-name"))

        BOLT = "lntbs1500n1pfakeinvoicefakeinvoicefakeinvoice"

        def copied(a):
            """Click Copy and return what would have gone to the clipboard."""
            pg = a["page"]
            pg.evaluate("window.__copied = null; Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: function (t) { window.__copied = t; return Promise.resolve(); } } })")
            pg.click("#copy-btn")
            pg.wait_for_timeout(100)
            return pg.evaluate("window.__copied")

        def shown(a):
            return a["page"].text_content("#address").strip()

        def c_ln_first(a, pr):
            pg = a["page"]
            if shown(a) != BOLT: pr.append(f"Lightning invoice should be shown first, saw {shown(a)[:50]}")
            if copied(a) != BOLT: pr.append("Copy should copy the Lightning invoice")
            has(a, pr, "says which way is shown", "Paying with Lightning", pg.inner_text("#method"))
            if not pg.is_visible("#ln-fee"): pr.append("the Lightning fee note should be visible without opening anything")
            has(a, pr, "fee note", "3 sats", pg.inner_text("#ln-fee"))
            if pg.get_attribute("#tab-ln", "aria-selected") != "true": pr.append("Lightning tab not selected")
            pg.click("#other summary")
            pg.click("#tab-bip321")
            if not shown(a).startswith("bitcoin:"): pr.append("the combined code should show after choosing it")
            if copied(a) != shown(a): pr.append("Copy should follow the combined code")
            if pg.is_visible("#ln-fee"): pr.append("the Lightning fee note should hide on other methods")
            pg.click("#tab-ark")
            if not shown(a).startswith("tark1"): pr.append("Ark address should show after choosing Ark")
            if copied(a) != shown(a): pr.append("Copy should follow Ark")
            pg.click("#tab-chain")
            if shown(a) != "tb1qfakefakefakefakefake": pr.append(f"on-chain tab should show the plain address only, saw {shown(a)[:40]}")
            if copied(a) != "tb1qfakefakefakefakefake": pr.append("Copy should copy just the on-chain address")
            if "bitcoin:" in shown(a): pr.append("the on-chain choice must not include the combined code")
            pg.click("#tab-ln")
            if copied(a) != BOLT: pr.append("Copy should follow back to Lightning")

        def c_no_ln(a, pr):
            if not shown(a).startswith("bitcoin:"): pr.append(f"without Lightning the combined code shows first, saw {shown(a)[:40]}")
            if a["page"].is_visible("#tab-ln"): pr.append("no Lightning tab without an invoice")
            if a["page"].is_visible("#ln-fee"): pr.append("no fee note without Lightning")

        def c_usd_sats(a, pr):
            pg = a["page"]
            has(a, pr, "amount shows dollars", "1,000 sats ($0.40)", pg.inner_text("#amount"))
            has(a, pr, "sats line shows dollars", "1,000 sats ($0.40)", pg.inner_text("#sats"))
            has(a, pr, "fee note shows dollars", "3 sats (<$0.01)", pg.inner_text("#ln-fee"))

        def c_usd_shop_in_dollars(a, pr):
            pg = a["page"]
            has(a, pr, "dollar shop heading", "USD 1.00", pg.inner_text("#amount"))
            if "$" in pg.inner_text("#sats"): pr.append("a shop that prices in dollars needs no second dollar figure")

        def c_usd_euro_shop(a, pr):
            has(a, pr, "euro shop sats line", "about 1,500 sats ($1.32)", a["page"].inner_text("#sats"))

        def c_usd_unknown(a, pr):
            pg = a["page"]
            if "$" in pg.inner_text("#amount") or "(" in pg.inner_text("#sats"): pr.append("no dollars when the price is unknown")

        cases = [
            ("pending: waiting message, pay details shown", invoice(), [{"status": "pending", "onchain_confirmations": None}], c_pending),
            ("paid: thank-you, pay details hidden, 'Done' link", invoice(), [{"status": "paid"}], c_paid),
            ("overpaid: thank-you with a note about the extra", invoice(), [{"status": "overpaid"}], c_over),
            ("underpaid: asks the customer to contact the store", invoice(), [{"status": "underpaid"}], c_under),
            ("expired: message, pay details hidden, link back to try again", invoice(), [{"status": "expired"}], c_expired),
            ("cancelled: message, pay details hidden, link back", invoice(), [{"status": "cancelled"}], c_cancelled),
            ("on-chain waiting: shows the confirmation count", invoice(), [{"status": "pending", "onchain_confirmations": 0, "onchain_confirmations_required": 1}], c_chain),
            ("unknown payment: plain 'could not find' message", 404, [{"status": "pending"}], c_nf),
            ("server unreachable: plain connection message", "abort", [{"status": "pending"}], c_net),
            ("status errors then recovery: the page keeps trying and shows paid", invoice(), [500, "abort", {"status": "paid"}], c_recover),
            ("hostile text and a javascript: link are inert", invoice(store_name=X, return_url="javascript:window.__xss=1", fiat_amount=X, amount_sats=10**18), [{"status": "pending"}], c_evil),
        ]
        cases += [
            ("Lightning is shown first; Copy follows every way to pay; on-chain has no combined code",
             invoice(lightning={"bolt11": BOLT, "fee_estimate_sat": 3}, onchain_enabled=True, onchain_address="tb1qfakefakefakefakefake"),
             [{"status": "pending"}], c_ln_first),
            ("no Lightning invoice: the combined code is the default", invoice(), [{"status": "pending"}], c_no_ln),
        ]
        cases += [
            ("sats-only payment shows dollars next to sats (and in the fee note)",
             invoice(fiat_currency=None, fiat_amount=None, amount_sats=1000, usd_per_btc=40000.0, lightning={"bolt11": BOLT, "fee_estimate_sat": 3}),
             [{"status": "pending"}], c_usd_sats),
            ("a shop pricing in dollars does not repeat dollars", invoice(usd_per_btc=40000.0), [{"status": "pending"}], c_usd_shop_in_dollars),
            ("a euro shop still gets the dollar figure", invoice(fiat_currency="EUR", fiat_amount="1.20", amount_sats=1500, usd_per_btc=88000.0), [{"status": "pending"}], c_usd_euro_shop),
            ("no price known: no dollars, nothing broken", invoice(fiat_currency=None, fiat_amount=None, usd_per_btc=None), [{"status": "pending"}], c_usd_unknown),
        ]
        for name, j, st, chk in cases:
            ok &= run_case(pw, name, j, st, chk)
        ok &= run_case(pw, "narrow phone (320 px) fits", invoice(store_name="A very long shop name " * 6), [{"status": "pending"}], c_pending, width=320)
    sys.exit(0 if ok else 1)


main()
