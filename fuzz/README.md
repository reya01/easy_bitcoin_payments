# Fuzz tests

[libFuzzer](https://llvm.org/docs/LibFuzzer.html) through [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz):
each target feeds millions of random and malformed inputs to code that reads data from outside,
and stops at the first crash or broken rule.

| Target | What it reads | Rule checked |
| --- | --- | --- |
| `lightning_invoice` | Lightning invoices from the Ark server | never crashes |
| `payout_address` | addresses the owner pastes | never crashes; an accepted address is accepted again, unchanged |
| `webhook_signature` | payment-notice signatures | good ones verify; a changed body, secret, signature or an old one never does |
| `wallet_history` | the wallet's history entries (JSON) | never crashes; every payment has a positive amount and an address |
| `fiat_amount` | money amount and price | an answer is always 1 sat to the whole money supply |

The WooCommerce plugin is PHP and is not covered here; its checks are tested end to end in CI.

**In CI:** the `fuzz` workflow runs each target for 10 minutes every week and on demand
(Actions, fuzz, Run workflow). A failure prints the input that broke it (`Base64:` in the log).

**On your own computer** (needs rustup): `cargo install cargo-fuzz`, then from this folder
`cargo fuzz run payout_address -- -max_total_time=60`. To replay a failing input from CI, save it
(`echo '<base64>' | base64 -d > crash`) and run `cargo fuzz run <target> crash`.
