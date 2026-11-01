#!/usr/bin/env bash
# Barkd drift check (Phase 1, plan task 2 — upgraded).
#
# Compares every PATH_* constant in src/ against the pinned OpenAPI spec
# (docs/barkd-openapi.json):
#
#   1. PATH PRESENCE — a path that is not in the pinned spec is a WARNING:
#      those are the `VERIFY` markers (endpoints used before the spec
#      pinned them, or the spec lagging the daemon). They are listed,
#      not fatal.
#   2. METHOD MATCH — the HTTP method named in the constant's doc comment
#      (the codebase convention: the doc block carries the verb, e.g.
#      `/// Fresh Ark receive address. POST.`) must be one of the OpenAPI
#      methods for that path. A mismatch FAILS: the daemon changed under
#      us. A constant with no verb in its doc comment is a warning — the
#      check cannot run for it.
#
# The spec may or may not include the /api/v1 prefix, so both the path as
# written and with the prefix are tried.
#
# Requires jq (or python3 as a fallback) to read the spec.
#
# Usage: scripts/check_barkd_drift.sh [spec.json]
set -euo pipefail

cd "$(dirname "$0")/.."

OPENAPI="${1:-docs/barkd-openapi.json}"
if [ ! -f "${OPENAPI}" ]; then
  echo "error: OpenAPI spec not found: ${OPENAPI}" >&2
  exit 1
fi

# Sanity: the spec must parse and have a paths object.
if command -v jq >/dev/null 2>&1; then
  jq -e '(.paths | type) == "object"' "${OPENAPI}" >/dev/null
elif command -v python3 >/dev/null 2>&1; then
  python3 - "${OPENAPI}" <<'PY'
import json, sys
with open(sys.argv[1]) as handle:
    spec = json.load(handle)
if not isinstance(spec.get("paths"), dict):
    raise SystemExit("no paths object")
PY
else
  echo "error: need jq or python3 to read ${OPENAPI}" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# openapi_methods <path> — print the spec's HTTP methods for a path (tries
# the path as written and with the /api/v1 prefix). Empty output = the
# path is not in the spec.
# ---------------------------------------------------------------------------
openapi_methods() {
  local path="$1" candidate methods
  for candidate in "${path}" "/api/v1${path}"; do
    methods=""
    if command -v jq >/dev/null 2>&1; then
      methods="$(jq -r --arg p "${candidate}" \
        '[.paths[$p] // {} | keys[] | ascii_downcase |
          select(. == "get" or . == "post" or . == "put" or . == "delete" or . == "patch" or . == "head" or . == "options")]
         | sort | .[] | ascii_upcase' "${OPENAPI}" 2>/dev/null)" || methods=""
    elif command -v python3 >/dev/null 2>&1; then
      methods="$(python3 - "${OPENAPI}" "${candidate}" <<'PY'
import json, sys
try:
    with open(sys.argv[1]) as handle:
        spec = json.load(handle)
    entry = spec.get("paths", {}).get(sys.argv[2]) or {}
    verbs = {"get", "post", "put", "delete", "patch", "head", "options"}
    for method in sorted(k for k in entry if k.lower() in verbs):
        print(method.upper())
except Exception:
    pass
PY
)" || methods=""
    else
      echo "error: need jq or python3 to read ${OPENAPI}" >&2
      exit 1
    fi
    if [ -n "${methods}" ]; then
      echo "${methods}"
      return 0
    fi
  done
  return 0
}

# ---------------------------------------------------------------------------
# Walk src/ and emit one line per PATH_* constant:
#   <file>:<line> <CONST_NAME> <METHOD-or-?> <path>
# The method comes from the constant's doc comment (see the header).
# ---------------------------------------------------------------------------
scan_sources() {
  { grep -rl --include='*.rs' -E '(pub +)?const PATH_[A-Z0-9_]+: *&str' src/ 2>/dev/null || true; } \
    | sort -u \
    | while read -r file; do
        awk -v file="${file}" '
          function last_method(text, re, found, rest) {
            found = ""
            re = "(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)\\."
            rest = text
            while (match(rest, re)) {
              found = substr(rest, RSTART, RLENGTH - 1)
              rest = substr(rest, RSTART + RLENGTH)
            }
            return found
          }
          /^[[:space:]]*\/\/\// { doc = doc " " $0; next }
          /^[[:space:]]*(pub[[:space:]]+)?const[[:space:]]+PATH_[A-Z0-9_]+[[:space:]]*:[[:space:]]*&str/ {
            line = $0
            sub(/^[[:space:]]*(pub[[:space:]]+)?const[[:space:]]+/, "", line)
            name = line
            sub(/[[:space:]].*$/, "", name)
            value = line
            sub(/^[^"]*"/, "", value)
            sub(/".*$/, "", value)
            method = last_method(doc)
            if (method == "") method = "?"
            print file ":" FNR " " name " " method " " value
            doc = ""
            next
          }
          /[^[:space:]]/ { doc = "" }
        ' "${file}"
      done
}

failures=0
warnings=0

while read -r location name method path; do
  methods="$(openapi_methods "${path}")"
  if [ -z "${methods}" ]; then
    echo "WARN  ${location}: ${name} = ${path} is not in the pinned spec (VERIFY marker — confirm against a live barkd)"
    warnings=$((warnings + 1))
    continue
  fi
  if [ "${method}" = "?" ]; then
    echo "WARN  ${location}: ${name} has no HTTP verb in its doc comment — add one (e.g. \`/// ... POST.\`) so the drift check can verify it"
    warnings=$((warnings + 1))
    continue
  fi
  if ! printf '%s\n' "${methods}" | grep -qx "${method}"; then
    echo "FAIL  ${location}: ${name} uses ${method} but the pinned spec offers: $(printf '%s ' ${methods})"
    failures=$((failures + 1))
  else
    echo "ok    ${location}: ${name} ${method} ${path}"
  fi
done < <(scan_sources)

echo
echo "barkd drift: ${failures} method mismatch(es), ${warnings} warning(s)"
if [ "${failures}" -gt 0 ]; then
  exit 1
fi