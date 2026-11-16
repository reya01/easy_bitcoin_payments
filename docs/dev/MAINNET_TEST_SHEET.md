# Test-day sheet: the first test with real bitcoin (mainnet)

Stage 4 of [MAINNET_PLAN.md](MAINNET_PLAN.md). Keep it open beside you, do the
steps in order and **write down every place you hesitated, guessed or got
stuck**, plus the real times. Each step says where it happens: **your payment
server** (its Console or its `/admin` page), **your WooCommerce site**, or **your
phone wallet**.

**Real money, very small amounts.** About 10,000 sats for the whole test, never
more than about 2,000 sats per order. Only start once the release candidate with
the mainnet changes has passed the signet "Money safety" section and the restore
drill.

**Stop conditions: pause payments, do not retry, write down what you saw.** Any
transfer or renewal that fails twice; any amount that differs between the server
and your phone wallet; any warning about coins near expiry; any order marked paid
without the money arriving.

## If something goes wrong during the test

Your money is in the wallet on the payment server, and **you hold its 12
recovery words**, so it can always be restored (see
[RESTORE_DRILL.md](../RESTORE_DRILL.md)). Do not retry anything in a hurry. Match
what you see to a line below, write it down, then go to
[IF_SOMETHING_LOOKS_WRONG.md](../IF_SOMETHING_LOOKS_WRONG.md).

| What you see | What it means | What to do |
|---|---|---|
| Home shows **"payout address refused"** or a red **"Your payout address is not valid"** line | The address failed the safety check. **Nothing was sent.** | **Payment server `/admin`, Settings:** enter a correct address. Do not use "Use it anyway" on the real server. |
| Red line **"This server is set to real money, but its wallet is play money"** (or the reverse) | The server's mode and its wallet do not match. | Take no payments. This is a stop condition: write it down and ask. |
| A transfer shows **failed** | The server could not send. The money is still in the wallet. It tries again every 5 minutes. | If it fails twice, that is a stop condition: **Settings → Pause new payments**, write down the message shown, do not retry. |
| You paid, but the checkout page still says waiting | Detection can take a minute or two. | Do not pay again. Check **Payments** and **Alerts → Payments that need you**. A payment that arrives after an order closed shows up there. |
| An order is marked paid but your phone wallet shows nothing | Stop condition. | Press **Pause new payments** (Settings) and write down the order, the amounts and the times. |
| You cannot open the server's pages | The server may be off or restarting (it restarts once after setup). | Wait a few minutes, then use **your hosting company's Console** to check the server is running. Do not delete the server: the money can be restored from the 12 words onto a new one. |

What the server is expected to do when something fails, read from the code, is in
[MAINNET_CODE_PATHS.md](MAINNET_CODE_PATHS.md).

## Before you start

- [ ] The signet checks in [RC3_CHECKS.md](RC3_CHECKS.md) are done (the payment page, setup page and admin page changes of rc3).
- [ ] Your own Ark wallet on your phone (Arké or Noah) set to **Second's mainnet
      server** (`ark.second.tech`), with its own seed written down, and about
      10,000 sats in it or in another Lightning wallet to pay from.
- [ ] A WooCommerce test shop with a product priced **under one dollar**.
- [ ] The plugin zip from the new release (Releases page, under **Assets**).
- [ ] Pen and paper for the server's 12 recovery words, and a password manager.

## 1. A new server set to real money (about 30 minutes)

- [ ] **On DigitalOcean:** make a **new** Droplet exactly as in
      [START_HERE.md](../START_HERE.md) steps 1 to 3.
- [ ] **On the payment server, in the Console** (log in as `root`):
      ```
      cd /opt/ebp
      echo "EBP_MAX_INVOICE_SATS=5000" >> .env
      ebp-info
      ```
      Note the new setup code.
- [ ] **In your browser, on the setup page, at Create your wallet:** the
      Network box shows **Signet** by default. Choose **Mainnet**: the warning
      and the tick-box appear, the Ark server changes to `https://ark.second.tech`,
      and the banner says **REAL MONEY**. Pressing Create without the tick is
      refused. Tick it and create the wallet.
- [ ] At the end of setup the server restarts once by itself (about a minute).
      Afterwards Home shows the real-money banner.

## 2. Setup wizard

- [ ] **Setup page:** create the wallet, write down the 12 words twice, confirm
      the three words, set the admin password.
- [ ] **Phone wallet:** open Receive and copy your `ark1…` address.
- [ ] **Setup page:** choose Ark wallet, paste the `ark1…` address, save the API key.
- [ ] **Payment server `/admin`:** sign in, turn on two-step sign-in.
- [ ] **Home:** the real-money banner shows; no red "wallet is play money" line.

## 3. Address checks (do these before any payment, so there is nothing to send)

**Do all of step 3 before step 5.** Several checks below put someone else's
address in Settings for a moment. A change to the payout address waits a day, but
if it were left pending the server would then send your money there. With nothing
on the server yet, that cannot cost anything. Cancel every pending change as soon
as you have seen the result.

**On the setup page** (the payout step; the check runs when you paste an address,
and a line or box appears under it). Nothing is saved until you press Create.
The sample addresses are public test values: they belong to nobody you know, so
never press Create with them.

| Paste this | You should see |
|---|---|
| Your own `ark1…` address from your phone wallet | Green **"Verified: a well-formed Ark address on the same Ark server as this wallet"**. If it is amber **"Could not verify"** instead, the server could not read its own Ark server details: wait a minute, paste again, and write it down if it stays amber. |
| Your `ark1…` address with **one character changed** in the middle | Red **Danger** box: "That address is not valid (a typo, or part of it is missing)". |
| `tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh` | Red Danger box: "That is a test-network address, but this server works with real Bitcoin", naming `ark.second.tech`, Noah (Android) and Arké (iPhone). |
| `tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx` | The same red Danger box (test-network address). Choose **On-chain** as the wallet type first. |
| `ark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqqnaa07n` | Red Danger box: "This Ark address is on a different Ark server than the one this server uses, so money sent to it may never arrive". (Well formed, but made for another server.) |
| `bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4` (wallet type **On-chain**) | Green **"Verified: a well-formed Bitcoin address for this network"**. |

- [ ] Every row above shows what it should. The Danger box on a real-money server
      must **not** say "if you are only testing with play money you can still go
      on"; it says nothing is sent to an address that fails the check.
- [ ] With a failed address and the tick-box **not** ticked, pressing **Create the
      store** is refused ("This address failed a check").
- [ ] Do **not** tick the box on the real server. (If you want to see what ticking
      does, do it on a spare play-money server: the store is created, and Home then
      shows the red "payout address refused" line. Nothing is ever sent to it.)

**In Settings** (changing the address later is strict: a refusal, with no
tick-box). Each of these must be **refused** with a clear message: the `tark1…`
and `tb1…` samples above, your `ark1…` with one character changed, and
`1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2` with its last character changed.

Each of these must be **accepted**, as a pending change. **Cancel it straight
away** with **Cancel the pending change**, then check Home shows no "payout
address change is waiting" line: `bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4`,
`bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0`,
`1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2`, `3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy`.

- [ ] All of the above as expected (write down any odd message, and whether the
      wording told you what to do).
- [ ] **Settings:** your own `ark1…` address is the active payout address again,
      and nothing is pending.

## 4. Record the server's terms

- [ ] **Payment server Console:** `cd /opt/ebp && ./scripts/drill_capture.sh`.
      Keep the file. Expected (11 November): rounds every 1 hour, 28-day coins,
      free renewal under 288 blocks, `ln_receive_anti_dos_required: false`.

## 5. First payment: Lightning, about 500 sats

- [ ] **Payment server `/admin`, Payments → Try a payment:** 500 sats, open the
      checkout page. The Lightning invoice starts `lnbc`; under "Other ways to
      pay" the Ark address starts `ark1`.
- [ ] **Phone wallet:** pay it. The checkout page says **Paid**. Time: ____
- [ ] If no Lightning invoice can be made, write down the error, pay over Ark
      instead, then try Lightning again later.

## 6. Transfer to your wallet at 1,000

- [ ] **Try a payment:** about 2,000 sats, pay it from the phone wallet.
- [ ] **Home:** a "Routine wallet upkeep" row shows **requested**, later
      **renewed**. Mainnet rounds are hourly: allow up to about 75 minutes.
- [ ] **Home → Recent transfers:** about 1,000+ sats sent to your wallet, about
      1,000 kept. Time from payment to arrival: ____
- [ ] **Phone wallet:** the amount matches exactly (write down any fee).

## 7. Odd payments

- [ ] Overpay an order by a few sats, underpay one, let one expire. Same results
      as on signet.

## 8. WooCommerce

- [ ] **WooCommerce site:** install the new plugin zip, point it at this server
      (server address, API key, signing secret), buy the product under one dollar.
      The order goes to **Processing** by itself.
- [ ] The lost-notice check from the signet sheet (the order still finishes).

## 9. Renewal on a real round

- [ ] The coin from step 5 (Lightning coins live 1 to 3 days) gets renewed:
      **Home → Routine upkeep** shows requested, then renewed. Write down how long
      it waited, and whether any "failed" row or warning appeared.

## 10. Weekly wallet check, backup, restore

- [ ] **Home:** the weekly wallet check shows the right figures and folds away
      after "I checked: it matches".
- [ ] **Settings:** download a backup and check it.
- [ ] Do [RESTORE_DRILL.md](../RESTORE_DRILL.md) onto a second **real-money**
      server (set `EBP_MODE=live` there too, step 1 above), with only the kept
      1,000 sats at stake. Note any extra upkeep row right after the restore.

## 11. Emergency withdrawal: look first

- [ ] **Settings → Emergency:** open it. It shows the fee estimate, the
      "Set aside fee money now" box (minimum 1,500 sats; 5,000 to 10,000
      recommended) and, if you hold under 100,000 sats on the server, a reason
      it will not start (the 100,000-sat floor). Write the estimate down.
- [ ] Starting one is a deliberate real-money test: do it only as in the
      emergency-exit test plan, with two-step sign-in on, fee money confirmed,
      and an amount you can afford to lose. It then takes about a day to claim.

## 12. Leave it running

- [ ] Leave the server running with the kept 1,000 sats. After 3 days: no
      warnings, no phone alerts.
- [ ] Around day 26 to 28: the 28-day coin was renewed inside the free window,
      no fee, no expiry warning.
- [ ] At the end: **Settings → Move my money now**. Home shows 0 on the server.

## Numbers to write down as you go

| What | Where you find it | Your result |
|---|---|---|
| Rounds every ___ ; coin life ___ ; free renewal under ___ blocks | `drill_capture` file (step 4) | |
| Lightning receive fee on the 500-sat payment | Phone wallet, or the invoice detail | |
| Time from paying to **Paid** on the checkout page (Lightning) | Step 5 | |
| Time from payment to arrival in your wallet (transfer) | Step 6 | |
| Fee on that transfer, if any | Phone wallet vs. the amount sent | |
| Smallest Ark transfer that worked | Step 6 / 7 | |
| Time a renewal took ("requested" to "renewed") | Home → Routine upkeep | |
| Any "failed" row, warning, or alert, with its exact wording | Home / phone | |
| Emergency withdrawal fee estimate | Settings → Emergency | |
| Where you hesitated or guessed | Anywhere | |

## Write down for Stage 5

The Lightning receive fee; the parsed fee table and coin lifetime; the smallest
Ark transfer that worked; how long a transfer and a renewal really took; your
`ark1…` address format (for the "different Ark server" check); the emergency
withdrawal fee estimate.
