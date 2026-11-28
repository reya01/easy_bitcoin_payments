# Archive

Finished plans, reviews, drill reports and test notes. They are kept so people can see
why things were done, but they are **not current**: for what is true and open now, read
[KNOWN_ISSUES.md](../../KNOWN_ISSUES.md), [MAINNET_RELEASE_STATUS.md](../MAINNET_RELEASE_STATUS.md),
[RC_CHECKLIST.md](../RC_CHECKLIST.md) and [HANDOFF.md](../HANDOFF.md).

Rule of thumb: when a plan is done, a test round is over, or a report has been acted on, move it
here (with `git mv`, so history follows), add a row below, and fix the links to it. Code and CI comments
that point at a history file should name its `archive/` path. `SIGNET_DRILL.md` stays in `docs/dev/` because
the signet smoke workflow names it, and `BARKD_0_9_0_UPGRADE.md` because it is the recipe for the next barkd move.

| File | What it was |
| --- | --- |
| [PHASE0_SPIKE_PLAN.md](PHASE0_SPIKE_PLAN.md) | The first questions about the Bark wallet, all answered on signet on 1 November 2026. |
| [BETA_PLAN.md](BETA_PLAN.md) | Ledger of the first hardening pass (merged). |
| [BEGINNER_ROADMAP.md](BEGINNER_ROADMAP.md) | The early plan for a phone-only, no-terminal setup (now built). |
| [REVIEW_2026-11-05.md](REVIEW_2026-11-05.md) | Code review of 5 November 2026. |
| [SECURITY_REVIEW_2026-11-05.md](SECURITY_REVIEW_2026-11-05.md) | Security review of 5 November 2026. |
| [DROPLET_TEST_RESULTS_2026-11-08.md](DROPLET_TEST_RESULTS_2026-11-08.md) | Notes from the first DigitalOcean test on signet, 8 November 2026. |
| [review-deepseek.md](review-deepseek.md), [review-glm.md](review-glm.md) | Early outside reviews of the spec. |
| [UX_RECOMMENDATIONS.md](UX_RECOMMENDATIONS.md) | An outside AI's UX review (14 November 2026). Part done in PRs #190 to #193, the rest decided against or not scheduled. |
| [PHASE1_PLAN.md](PHASE1_PLAN.md) | The barkd integration plan, code-complete on 1 November 2026, with its drill log. |
| [WATCHER_DESIGN_NOTE.md](WATCHER_DESIGN_NOTE.md) | Design note for the payment watcher re-reading the whole history; built. |
| [DROPLET_TEST_SHEET.md](DROPLET_TEST_SHEET.md) | The test-day sheet for a live DigitalOcean server, rounds 1 to 3 (last written for 0.2.2-rc1). Each release now gets its own short test sheet. |
| [MAINNET_TEST_SHEET.md](MAINNET_TEST_SHEET.md) | The test-day sheet for the first real-money test (stage 4 of the mainnet plan, 15 to 16 November 2026). |
| [RC3_CHECKS.md](RC3_CHECKS.md) | The signet checks for what was new in 0.2.0-rc3. |
| [MAINNET_DRILL_2026-11-15.md](MAINNET_DRILL_2026-11-15.md), [MAINNET_DRILL_2026-11-15_round2.md](MAINNET_DRILL_2026-11-15_round2.md) | The two live mainnet drills with two barkd wallets (15 November 2026). |
| [SERIES2_TESTS_2026-11-15.md](SERIES2_TESTS_2026-11-15.md) | The third drill session: empty-till claims, invoice windows, refresh and sweep costs. |
| [LIGHTNING_RECEIVE_FEES_2026-11-15.md](LIGHTNING_RECEIVE_FEES_2026-11-15.md) | What a Lightning receive really costs on barkd 0.8.0. |
| [MAINNET_DRILL_RESPONSE.md](MAINNET_DRILL_RESPONSE.md) | Item-by-item response to the mainnet drills. |
| [MAINNET_PLAN.md](MAINNET_PLAN.md) | The agreed plan for getting to mainnet (11 November 2026): restore, mainnet changes, test with small real money. Done. |
| [EMERGENCY_EXIT_2026-11-15.md](EMERGENCY_EXIT_2026-11-15.md) | The real-money emergency-exit drill: what worked, what got stuck, what it cost. |
| [MAINNET_CODE_PATHS.md](MAINNET_CODE_PATHS.md) | A reading of the real-money code paths before the first real-money test (a snapshot; [TECHNICAL.md](../TECHNICAL.md) is the current description). |
| [BARK_0_9_REVIEW.md](BARK_0_9_REVIEW.md) | The Bark 0.9 review of 19 November 2026 and the decision that v0.3 runs Bark 0.9 (done in 0.3.0-rc1). |
| [BARK_AUDIT.md](BARK_AUDIT.md), [bark-reference-0.8/](bark-reference-0.8/), [barkd-endpoints-0.8.txt](barkd-endpoints-0.8.txt) | The audit of 5 November 2026 against barkd 0.8.0, with the copy of Bark's docs and the endpoint list it used. The current API snapshot is `docs/barkd-openapi.json`. |
| [RC_CHECKLIST_HISTORY.md](RC_CHECKLIST_HISTORY.md) | The signet-era to-do lists and the draft notes for 0.2.2-rc1, moved out of the release checklist. |
