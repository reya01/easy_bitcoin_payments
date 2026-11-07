#!/usr/bin/env bash
# Tests scripts/vps-setup.sh without touching the system: a fake start command
# prints the same box start.sh prints, and the files are written to a temp dir.
set -euo pipefail
cd "$(dirname "$0")/.."
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/ebp"

cat > "$T/fake-start.sh" <<'FAKE'
#!/usr/bin/env bash
echo "==> https is ON"
echo
echo "=================================================="
echo "  The server is running."
echo
echo "  1. Open this page in your browser:"
echo "       https://203-0-113-7.sslip.io/setup"
echo
echo "  2. When it asks for your SETUP CODE, type:"
echo
echo "       ABCD-1234"
echo "=================================================="
FAKE
chmod +x "$T/fake-start.sh"

run() {
  EBP_SKIP_SYSTEM=1 EBP_DIR="$T/ebp" EBP_START_CMD="$T/fake-start.sh" \
    EBP_INFO_FILE="$T/info.txt" EBP_ISSUE_FILE="$T/issue" EBP_MOTD_FILE="$T/motd" EBP_LOG_FILE="$T/log.txt" \
    ./scripts/vps-setup.sh >/dev/null
}
fail() { echo "FAIL  $1"; exit 1; }

run
grep -q "https://203-0-113-7.sslip.io/setup" "$T/info.txt" || fail "info file has the setup address"
grep -q "ABCD-1234" "$T/info.txt" || fail "info file has the setup code"
grep -q "ABCD-1234" "$T/issue" || fail "console login screen shows the setup code"
grep -q "ABCD-1234" "$T/motd" || fail "message after login shows the setup code"
grep -q "=====" "$T/info.txt" && fail "the box rules are not copied"
echo "PASS  setup address and code are copied to the info file and the login screen"

run
[ "$(grep -c "ABCD-1234" "$T/issue")" = 1 ] || fail "running twice must not duplicate the login screen text"
echo "PASS  safe to run twice"

cat > "$T/fake-start.sh" <<'FAKE'
#!/usr/bin/env bash
echo "docker exploded"; exit 1
FAKE
run
grep -q "ebp-info" "$T/info.txt" || fail "a failed start still leaves a helpful message"
echo "PASS  a failed start leaves a helpful message"

# Which start command is chosen: first run asks for https, later runs keep the saved address.
mkdir -p "$T/ebp2/scripts"
cat > "$T/ebp2/scripts/start.sh" <<'FAKE'
#!/usr/bin/env bash
echo "$*" > "$(dirname "$0")/../args.txt"
echo "=================================================="
echo "  ok"
echo "=================================================="
FAKE
chmod +x "$T/ebp2/scripts/start.sh"
run2() {
  EBP_SKIP_SYSTEM=1 EBP_DIR="$T/ebp2" EBP_INFO_FILE="$T/info2.txt" EBP_ISSUE_FILE="$T/issue2" \
    EBP_MOTD_FILE="$T/motd2" EBP_LOG_FILE="$T/log2.txt" ./scripts/vps-setup.sh >/dev/null
}
run2
[ "$(cat "$T/ebp2/args.txt")" = "--https" ] || fail "first run asks for https"
echo "PASS  first run asks for https"
echo "EBP_DOMAIN=pay.myshop.com" > "$T/ebp2/.env"
run2
[ -z "$(cat "$T/ebp2/args.txt")" ] || fail "a saved domain must not be replaced by --https"
echo "PASS  a saved domain is kept on later runs"
