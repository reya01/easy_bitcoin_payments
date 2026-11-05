# Independent security review (2026-11-05)

A separate reviewer, who had not seen how the code was built, read the admin
login, the managed-barkd and setup flow, webhooks, the HTTP surface, updates
and backups, the release workflow and the container files. Code reading only;
nothing was run. Every finding below was re-checked against the code before
being fixed.

## Fixed

| # | Severity | Finding | Fix |
|---|----------|---------|-----|
| 4 | Medium | After the recovery-words window was opened (barkd restarted with `--expose-mnemonic`), three error paths returned without closing it, leaving the seed readable over barkd's local API until restart. | `close_mnemonic_window` runs on every exit after the window opens. |
| 1 | Medium | Compose files published plain-HTTP port 8080 on all interfaces, so the "HTTPS required" gate (which looks at the configured public URL) could be bypassed by opening `http://<ip>:8080/setup`. | Port now bound to `127.0.0.1` by default (`EBP_BIND_HOST=0.0.0.0` to opt out). Docs point at the HTTPS address. |
| 2 | Medium | Each login attempt ran the 600,000-round password hash while holding the database lock, so a burst of attempts stalled invoices and checkout. | Hash now runs on a blocking thread outside the lock. |
| 3 | Medium | Behind a proxy, rate limiting trusted the leftmost `X-Forwarded-For` entry (client-controlled), and the limiter table could be grown without bound. | Uses the rightmost entry (appended by our proxy); overflow bucket caps the table; added a global login cap of 120/min that needs no trust in any address. |
| 5 | Low-Med | Webhook address filter allowed IPv4-mapped IPv6 (`::ffff:127.0.0.1`), `localhost.`, CGNAT (`100.100.100.200`), `0.0.0.0/8`, NAT64 and other ranges. | Filter extended and unit-tested. |
| 6 | Low | Release workflow gave every job write and signing rights, including the job that runs third-party build code; CI had no permissions block. | Read-only by default; write/signing rights only on the jobs that need them. |
| 7 | Low | Database file and new data/backup folders were created with default (world-readable) permissions. | Created owner-only (0600 file, 0700 new folders). |

## Not fixed (accepted or later)

- **DNS rebinding** of webhook hostnames is still not caught (the filter is
  textual). Documented limitation; needs a custom resolver or egress rules.
- Release actions are pinned by version tag, not commit SHA, and the binary
  tarballs are not signed (images are). Worth doing before the first release.
- Session cookie lacks the `__Host-` prefix; failed logins write one audit row
  each (trimmed by the retention sweep); `/admin/password` has no extra rate
  limit (needs a valid session and CSRF token first).
- Managed mode keeps barkd, the encryption key and the database in one user
  and one volume, so a barkd compromise is a full compromise.
- Two crates in `Cargo.lock` are yanked (`spin 0.9.8`, `crossbeam-channel
  0.5.13`); `cargo audit` reports no vulnerabilities. Run `cargo update` in a
  PR when crates.io is reachable.

## Checked and found fine

Password hashing and constant-time compare; 256-bit session tokens stored
hashed with absolute and idle expiry; HttpOnly + SameSite=Strict cookie and
CSRF header; no `innerHTML` sinks in the pages; CSP and framing headers; no
CORS; child-process spawn (cleared environment, loopback, fixed arguments);
token and mnemonic never logged; ChaCha20-Poly1305 secrets with per-row
context; webhook signing and no redirects; update check restricted to a fixed
URL and GitHub links; non-root container with a checksum-pinned barkd.
