#!/usr/bin/env python3
"""WooCommerce end-to-end: real EBP binary + fake barkd + a real WordPress with
WooCommerce (docker) running our plugin.

Flow: a customer checks out through the WooCommerce Store API (the same server
path the block checkout uses), is sent to the EBP checkout page, "pays" through
the fake wallet, and the order must flip to paid by itself via the signed
webhook. Also checks bad/forged webhooks, a partial payment, and that the block
checkout script is registered.

Usage: run_woo.py <path-to-ebp-binary>      (needs docker and python3)
"""
import hashlib
import hmac
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tests", "e2e"))
EBP_BIN = os.path.abspath(sys.argv[1])
FAKE_PORT, EBP_PORT, WP_PORT = 18891, 18880, 18885
TOKEN = "woo-token"
NET = "woonet"
tmp = tempfile.mkdtemp(prefix="ebp-woo-")
procs, results = [], []


def sh(*argv, check=True, capture=True):
    r = subprocess.run(argv, capture_output=capture, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"{' '.join(argv)[:200]} -> {r.returncode}\n{r.stdout[-800:]}\n{r.stderr[-800:]}")
    return r.stdout.strip()


def http(method, url, body=None, headers=None, raw=None):
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    h = {"content-type": "application/json", **(headers or {})}
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            b = r.read()
            return r.status, dict(r.headers), b
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def jhttp(*a, **k):
    code, hdr, b = http(*a, **k)
    try:
        return code, hdr, json.loads(b)
    except Exception:
        return code, hdr, b.decode(errors="replace")


def wait_for(fn, what, timeout=60, every=1):
    end, last = time.time() + timeout, None
    while time.time() < end:
        try:
            last = fn()
            if last:
                return last
        except Exception as e:  # noqa: BLE001
            last = e
        time.sleep(every)
    raise AssertionError(f"timed out waiting for: {what} (last: {last})")


def step(name):
    def deco(fn):
        t0 = time.time()
        try:
            fn()
            results.append((name, True))
            print(f"PASS  {name}  ({time.time() - t0:.1f}s)", flush=True)
        except Exception as e:  # noqa: BLE001
            results.append((name, False))
            print(f"FAIL  {name}: {repr(e)[:900]}", flush=True)
        return fn

    return deco


def wpcli(*args):
    return sh("docker", "run", "--rm", "--network", NET, "--volumes-from", "woo-wp", "--user", "33:33",
              "-e", "WORDPRESS_DB_HOST=woo-db", "-e", "WORDPRESS_DB_USER=wp", "-e", "WORDPRESS_DB_PASSWORD=wp",
              "-e", "WORDPRESS_DB_NAME=wp", "wordpress:cli", "wp", *args)


def fake(**kw):
    return jhttp("POST", f"http://127.0.0.1:{FAKE_PORT}/__ctl", kw)[2]


def cleanup():
    for p in procs:
        p.terminate()
    sh("docker", "rm", "-f", "woo-wp", "woo-db", check=False)
    sh("docker", "network", "rm", NET, check=False)


def main():
    data = os.path.join(tmp, "data")
    os.makedirs(data)
    sh("docker", "network", "create", NET)
    sh("docker", "run", "-d", "--name", "woo-db", "--network", NET, "-e", "MARIADB_ROOT_PASSWORD=root",
       "-e", "MARIADB_DATABASE=wp", "-e", "MARIADB_USER=wp", "-e", "MARIADB_PASSWORD=wp", "mariadb:11")
    sh("docker", "run", "-d", "--name", "woo-wp", "--network", NET, "--add-host=host.docker.internal:host-gateway",
       "-p", f"127.0.0.1:{WP_PORT}:80",
       "-e", "WORDPRESS_DB_HOST=woo-db", "-e", "WORDPRESS_DB_USER=wp", "-e", "WORDPRESS_DB_PASSWORD=wp",
       "-e", "WORDPRESS_DB_NAME=wp",
       "-v", f"{ROOT}/integrations/woocommerce/ebp-bitcoin:/var/www/html/wp-content/plugins/ebp-bitcoin:ro",
       "wordpress:latest")

    for name, argv in (
        ("fake", [sys.executable, os.path.join(ROOT, "tests", "e2e", "fake_barkd.py"), str(FAKE_PORT), TOKEN]),
    ):
        f = open(os.path.join(tmp, f"{name}.log"), "w")
        procs.append(subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, cwd=tmp))
    wait_for(lambda: http("GET", f"http://127.0.0.1:{FAKE_PORT}/ping")[0] == 200, "fake barkd")

    # The wallet the wizard would have created.
    http("POST", f"http://127.0.0.1:{FAKE_PORT}/api/v1/wallet/create", {"mnemonic": None})
    env = {
        "PATH": os.environ["PATH"],
        "EBP_BIND_ADDR": f"0.0.0.0:{EBP_PORT}",
        "EBP_PUBLIC_BASE_URL": f"http://127.0.0.1:{EBP_PORT}",
        "EBP_DATABASE_PATH": os.path.join(data, "ebp.sqlite3"),
        "EBP_BARKD_URL": f"http://127.0.0.1:{FAKE_PORT}/api/v1",
        "EBP_BARKD_TOKEN": TOKEN,
        "EBP_WATCHER_INTERVAL_SECS": "1",
        "EBP_SETTLEMENT_INTERVAL_SECS": "3600",
        "EBP_WEBHOOK_ALLOW_PRIVATE": "1",
        "EBP_UPDATE_CHECK": "false",
    }
    f = open(os.path.join(tmp, "ebp.log"), "w")
    procs.append(subprocess.Popen([EBP_BIN, "serve"], stdout=f, stderr=subprocess.STDOUT, env=env, cwd=tmp))
    wait_for(lambda: http("GET", f"http://127.0.0.1:{EBP_PORT}/api/v1/health")[0] in (200, 503), "EBP")
    # Mark setup complete (the wizard itself is covered by the browser e2e).
    db = sqlite3.connect(os.path.join(data, "ebp.sqlite3"))
    db.execute("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('setup_complete','true',strftime('%s','now'))")
    db.commit()
    db.close()
    out = subprocess.run([EBP_BIN, "store", "create", "--name", "Woo Shop", "--webhook-url",
                          f"http://127.0.0.1:{WP_PORT}/?wc-api=ebp_bitcoin"],
                         capture_output=True, text=True, env=env, cwd=tmp)
    assert out.returncode == 0, out.stderr
    api_key = re.search(r"(ebp_[A-Za-z0-9_\-]+)", out.stdout).group(1)
    secret = re.search(r"signing secret.*\n\s+(\S+)", out.stdout).group(1)
    base = f"http://127.0.0.1:{EBP_PORT}"
    wp = f"http://127.0.0.1:{WP_PORT}"

    # ---------------------------------------------------------- install WP
    wait_for(lambda: http("GET", f"{wp}/")[0] in (200, 302, 500), "wordpress container", timeout=120)
    wait_for(lambda: "Connected" in subprocess.run(
        ["docker", "exec", "woo-db", "mariadb-admin", "-uwp", "-pwp", "ping"], capture_output=True, text=True).stdout
        or "alive" in subprocess.run(["docker", "exec", "woo-db", "mariadb-admin", "-uwp", "-pwp", "ping"],
                                     capture_output=True, text=True).stdout, "mariadb", timeout=120)
    time.sleep(5)
    wpcli("core", "install", f"--url={wp}", "--title=Woo", "--admin_user=admin", "--admin_password=pw-admin-1",
          "--admin_email=a@example.com", "--skip-email")
    wpcli("plugin", "install", "woocommerce", "--activate")
    wpcli("plugin", "activate", "ebp-bitcoin")
    wpcli("option", "update", "woocommerce_currency", "USD")
    wpcli("option", "update", "woocommerce_ebp_bitcoin_settings", json.dumps({
        "enabled": "yes", "title": "Bitcoin", "description": "Pay with Bitcoin",
        "server_url": f"http://host.docker.internal:{EBP_PORT}", "api_key": api_key, "webhook_secret": secret,
    }), "--format=json")
    pid = wpcli("post", "create", "--post_type=product", "--post_title=Test Mug", "--post_status=publish", "--porcelain")
    wpcli("post", "meta", "update", pid, "_regular_price", "10.00")
    wpcli("post", "meta", "update", pid, "_price", "10.00")
    wpcli("post", "meta", "update", pid, "_virtual", "yes")
    wpcli("eval", f"wp_set_object_terms({pid}, 'simple', 'product_type');")

    state = {}

    @step("WooCommerce lists Bitcoin as an available payment method")
    def _():
        gws = wpcli("eval", "echo implode(',', array_keys(WC()->payment_gateways()->get_available_payment_gateways()));")
        assert "ebp_bitcoin" in gws, gws

    @step("block checkout script is registered for the Bitcoin method")
    def _():
        wpcli("eval", "echo 1;")  # warm up
        # The default Checkout page uses the block; render it and look for our script.
        page_id = wpcli("option", "get", "woocommerce_checkout_page_id")
        code, _, body = http("GET", f"{wp}/?page_id={page_id}")
        assert code == 200, code
        assert b"ebp-bitcoin-blocks" in body, "block script handle missing from the checkout page"

    @step("checkout via the Store API redirects to the EBP checkout page")
    def _():
        code, hdr, cart = jhttp("GET", f"{wp}/?rest_route=/wc/store/v1/cart")
        assert code == 200, (code, cart)
        nonce = hdr.get("Nonce") or hdr.get("nonce")
        token = hdr.get("Cart-Token") or hdr.get("cart-token")
        h = {"Nonce": nonce, "Cart-Token": token}
        code, _, r = jhttp("POST", f"{wp}/?rest_route=/wc/store/v1/cart/add-item", {"id": int(pid), "quantity": 1}, h)
        assert code in (200, 201), (code, str(r)[:300])
        addr = {"first_name": "Ada", "last_name": "Lovelace", "address_1": "1 Main St", "city": "Austin",
                "state": "TX", "postcode": "78701", "country": "US", "email": "ada@example.com", "phone": ""}
        code, _, r = jhttp("POST", f"{wp}/?rest_route=/wc/store/v1/checkout", {
            "billing_address": addr, "shipping_address": addr, "payment_method": "ebp_bitcoin", "payment_data": []}, h)
        assert code == 200, (code, str(r)[:600])
        assert r["payment_result"]["payment_status"] == "success", r
        url = r["payment_result"]["redirect_url"]
        assert url.startswith(f"{base}/i/"), url
        state["order"] = r["order_id"]
        state["invoice"] = url.rsplit("/", 1)[1]
        status = wpcli("eval", f"echo wc_get_order({state['order']})->get_status();")
        assert status == "on-hold", status

    def order_status():
        return wpcli("eval", f"echo wc_get_order({state['order']})->get_status();")

    @step("the EBP checkout page links back to the shop's order page")
    def _():
        code, _, d = jhttp("GET", f"{base}/i/{state['invoice']}/json")
        assert code == 200 and d.get("return_url", "").startswith(f"{wp}/"), d
        assert f"order-received/{state['order']}" in d["return_url"] or f"order_id={state['order']}" in d["return_url"] or "key=wc_order" in d["return_url"], d["return_url"]

    inv_auth = {"authorization": f"Bearer {api_key}"}

    @step("the invoice is for the order total and carries the order reference")
    def _():
        code, _, inv = jhttp("GET", f"{base}/api/v1/invoices/{state['invoice']}", headers=inv_auth)
        assert code == 200 and inv["order_id"] == f"wc-{state['order']}", inv
        assert inv["fiat_currency"] == "USD" and inv["fiat_amount"] in ("10.00", "10"), inv
        state["inv"] = inv

    @step("a forged webhook is refused and changes nothing")
    def _():
        body = json.dumps({"event": "invoice.paid", "invoice_id": state["invoice"]}).encode()
        code, _, _ = http("POST", f"{wp}/?wc-api=ebp_bitcoin", raw=body,
                          headers={"x-ebp-signature": f"t={int(time.time())},v1={'0' * 64}"})
        assert code == 401, code
        code, _, _ = http("POST", f"{wp}/?wc-api=ebp_bitcoin", raw=body)
        assert code == 401, code
        assert order_status() == "on-hold"

    @step("a correctly signed notice for a payment that never happened does not mark the order paid")
    def _():
        body = json.dumps({"event": "invoice.paid", "invoice_id": state["invoice"]}).encode()
        t = int(time.time())
        sig = hmac.new(secret.encode(), f"{t}.".encode() + body, hashlib.sha256).hexdigest()
        code, _, _ = http("POST", f"{wp}/?wc-api=ebp_bitcoin", raw=body, headers={"x-ebp-signature": f"t={t},v1={sig}"})
        assert code == 200, code
        assert order_status() == "on-hold", "the shop must re-check with the server before trusting the notice"

    @step("a partial payment is noted but the order stays on hold")
    def _():
        fake(action="pay_ark", address=state["inv"]["ark_address"], sats=1000)
        wait_for(lambda: "Partial Bitcoin payment" in wpcli(
            "eval", f"foreach (wc_get_order({state['order']})->get_customer_order_notes() as $n) {{}} "
                    f"foreach (wc_get_order_notes(['order_id'=>{state['order']}]) as $n) echo $n->content.'|';"), "partial note")
        assert order_status() == "on-hold"

    @step("paying the rest marks the order paid by itself")
    def _():
        total = state["inv"]["amount_sats"]
        fake(action="pay_ark", address=state["inv"]["ark_address"], sats=total - 1000)
        wait_for(lambda: order_status() in ("processing", "completed"), "order to be marked paid", timeout=90)

    @step("a repeated notice does not pay the order twice")
    def _():
        body = json.dumps({"event": "invoice.paid", "invoice_id": state["invoice"]}).encode()
        t = int(time.time())
        sig = hmac.new(secret.encode(), f"{t}.".encode() + body, hashlib.sha256).hexdigest()
        before = wpcli("eval", f"echo count(wc_get_order_notes(['order_id'=>{state['order']}]));")
        code, _, _ = http("POST", f"{wp}/?wc-api=ebp_bitcoin", raw=body, headers={"x-ebp-signature": f"t={t},v1={sig}"})
        assert code == 200
        after = wpcli("eval", f"echo count(wc_get_order_notes(['order_id'=>{state['order']}]));")
        assert before == after, (before, after)


try:
    main()
except Exception as e:  # noqa: BLE001
    print(f"FAIL  setup: {e!r}"[:1500], flush=True)
    results.append(("setup", False))
finally:
    for n in ("ebp", "fake"):
        p = os.path.join(tmp, f"{n}.log")
        if os.path.exists(p) and any(not ok for _, ok in results):
            print(f"--- {n}.log (tail)\n" + open(p).read()[-2500:])
    if any(not ok for _, ok in results):
        print("--- wordpress log\n" + sh("docker", "logs", "--tail", "40", "woo-wp", check=False, capture=True))
    cleanup()
    shutil.rmtree(tmp, ignore_errors=True)

failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)}/{len(results)} steps passed")
sys.exit(1 if failed else 0)
