"""Shared by the mocked-browser tests (checkout, admin, setup).

The pages load their script and stylesheet as files (no inline code), and the
server sends a strict Content-Security-Policy. The mocked tests serve those
files from the repository and send the server's real policy, read straight from
src/routes/mod.rs so the two can never drift apart. Anything the policy blocks
shows up as a console message in the browser; watch_csp turns that into a test
failure.
"""
import os
import re

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

_src = open(os.path.join(ROOT, "src", "routes", "mod.rs"), encoding="utf-8").read()
_m = re.search(r'"content-security-policy",\s*"((?:[^"\\]|\\.)*)"', _src, re.S)
if not _m:
    raise SystemExit("could not find the Content-Security-Policy in src/routes/mod.rs")
# A Rust string continues past a backslash-newline, dropping the leading spaces.
CSP = re.sub(r"\\\n\s*", "", _m.group(1))
if "unsafe-inline" in CSP or "script-src 'self';" not in CSP:
    raise SystemExit("unexpected Content-Security-Policy: " + CSP)

HEADERS = {"Content-Security-Policy": CSP}
_TYPES = {".js": "application/javascript", ".css": "text/css"}


def serve_asset(route, path):
    """Serve /assets/<name>.js or .css from the repository. True if handled."""
    path = path.split("?")[0]
    if not path.startswith("/assets/") or path.count("/") != 2:
        return False
    name = path[len("/assets/"):]
    ext = os.path.splitext(name)[1]
    fn = os.path.join(ROOT, "assets", name)
    if ext not in _TYPES or not os.path.isfile(fn):
        return False
    route.fulfill(status=200, content_type=_TYPES[ext], body=open(fn, encoding="utf-8").read())
    return True


def watch_csp(page, errors):
    """Record anything the Content-Security-Policy blocks as an error."""
    def on_console(msg):
        if "Content Security Policy" in msg.text:
            errors.append("blocked by the Content-Security-Policy: " + msg.text[:200])
    page.on("console", on_console)
