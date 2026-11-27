# Security policy

Easy Bitcoin Payments is **experimental software** and is not yet for real
funds. Please still report problems; a clear report now is worth a lot.

## How to report a vulnerability

Use GitHub's private reporting: on the repository page open the **Security**
tab and choose **Report a vulnerability**. This reaches the maintainers
privately. Please do **not** open a public issue or post details publicly until
a fix is released.

Include what you did, what you expected, what happened, and the version
(`ebp` prints it with `--help`, and it is on the admin page).

Never include recovery words, API keys, webhook secrets, admin passwords or
backup files in a report. If you need to show that you could read one, say so
without pasting it.

## What matters most to us

- Anything that lets someone other than the merchant move or redirect funds.
- Marking an order paid without a real payment, or the reverse.
- Reading or changing another person's data, or bypassing the admin sign-in.
- Weaknesses in install, update, backup or release signing.

Known limits we already document are in `docs/KNOWN_ISSUES.md` and
`docs/dev/archive/SECURITY_REVIEW_2026-11-05.md`.
What the code already guards, and what makes a useful report, is in
`docs/dev/REVIEWING.md`.

## Found a non-security bug?

Something broken that isn't a security problem (setup trouble, a confusing
screen, docs that are wrong)? Please open a regular
[bug report](https://github.com/reya01/easy_bitcoin_payments/issues/new?title=Bug%3A%20%3Cone-line%20summary%3E&body=%23%23%20What%20happened%0A%0A%23%23%20What%20you%20expected%0A%0A%23%23%20Steps%20to%20reproduce%0A%0A%23%23%20Version%20%28from%20%60ebp%20--help%60%20or%20the%20admin%20page%29%0A%0A%23%23%20Anything%20else%3F%0A).
Screenshots help a lot. **If it involves money movement, someone's keys or
secrets, or bypassing sign-in, treat it as a vulnerability and use the private
channel above instead** — even if you're not sure.

## What to expect

We will acknowledge a report, tell you whether we agree it is a problem, and
credit you in the release notes if you wish. This is a small project; please
be patient.
