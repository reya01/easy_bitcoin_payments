#!/bin/sh
# barkd OpenAPI drift check.
#
# Verifies that every REST path the ebp client uses (the PATH_* constants in
# src/wallet/barkd.rs) still exists in the pinned barkd spec
# (docs/barkd-openapi.json). Run from the repo root; CI runs it on every
# push and PR. Uses only sh + python3's json module — no new dependencies,
# no network.
#
# Exit codes: 0 = no drift, 1 = drift (or an unreadable spec),
# 2 = setup problem (missing file, no PATH_* constants).

set -eu

BARKD_RS="${1:-src/wallet/barkd.rs}"
OPENAPI_JSON="${2:-docs/barkd-openapi.json}"

if [ ! -f "$BARKD_RS" ]; then
    echo "error: $BARKD_RS not found (run from the repo root)" >&2
    exit 2
fi

if [ ! -f "$OPENAPI_JSON" ]; then
    echo "error: $OPENAPI_JSON not found (run from the repo root)" >&2
    exit 2
fi

python3 - "$BARKD_RS" "$OPENAPI_JSON" <<'PY'
import json
import re
import sys

src, spec_file = sys.argv[1], sys.argv[2]

# PATH_* constant definitions in src/wallet/barkd.rs, e.g.
#   const PATH_WALLET_HISTORY: &str = "/wallet/history";
with open(src, encoding="utf-8") as f:
    source = f.read()
used = re.findall(r"const\s+PATH_[A-Z0-9_]+[^=\n]*=\s*\"([^\"]+)\"", source)

if not used:
    sys.exit(f"error: no PATH_* constants found in {src}")

try:
    with open(spec_file, encoding="utf-8") as f:
        spec = json.load(f)
except (OSError, ValueError) as err:
    sys.exit(f"error: could not read or parse {spec_file}: {err}")

spec_paths = set(spec.get("paths", {}))

missing = set()
for path in used:
    # The spec may key paths with or without the /api/v1 prefix; accept
    # either shape so the check tests the path, not the base-URL convention.
    candidates = {path, "/api/v1" + path}
    if path.startswith("/api/v1/"):
        candidates.add(path[len("/api/v1"):])
    if not candidates & spec_paths:
        missing.add(path)

for path in sorted(missing):
    print(f"DRIFT: {path} (used by {src}) is missing from {spec_file}")

if missing:
    sys.exit(
        f"error: {len(missing)} barkd path(s) missing from the pinned spec; "
        "update docs/barkd-openapi.json or the client"
    )

print(f"ok: all {len(set(used))} PATH_* constants from {src} exist in {spec_file}")
PY