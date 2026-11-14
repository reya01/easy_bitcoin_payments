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

## Before you start

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

## 3. Address checks (Settings, change back afterwards)

**On the setup page** (try these on the new server's payout step, or on a spare
fresh server): when you paste an address, a line appears under the box.

- [ ] Your own `ark1…` address from your phone wallet: **Verified** (green).
- [ ] Your `ark1…` address with one character changed: a red **danger** box (money
      could be lost). Pressing Create is blocked until you tick "Use it anyway".
      Do not tick it on the real server.
- [ ] A `tark1…` address, and a `tb1…` address: red danger box that names
      `ark.second.tech`, Noah (Android) and Arké (iPhone).
- [ ] A valid Ark address from a **different Ark server** (if you can get one; a
      Bark signet address will do): red danger box saying it is on a different Ark
      server.
- [ ] Pressing Create with an address that failed and the box ticked saves it, and
      Home then shows the red "payout address refused" line (nothing is sent).
      Fix the address in Settings afterwards.

**In Settings** (changing the address later is still strict: a refusal, no tick-box).
Each of these must be **refused** with a clear message: a `tark1…`, a `tb1…`, your
`ark1…` with one character changed, a `1…` with one character changed.

Each of these must be **accepted** (then put your `ark1…` back; a change waits a
day, so cancel the pending change instead): a `bc1q…`, a `bc1p…`, a `1…`, a `3…`.

- [ ] All of the above as expected (write down any odd message, and whether the
      wording told you what to do).

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

## 11. Emergency withdrawal: look only

- [ ] **Settings → Emergency:** open it. It says it is switched off on real
      money, and shows the fee estimate. Write the estimate down. Do **not** try
      to start one on mainnet.

## 12. Leave it running

- [ ] Leave the server running with the kept 1,000 sats. After 3 days: no
      warnings, no phone alerts.
- [ ] Around day 26 to 28: the 28-day coin was renewed inside the free window,
      no fee, no expiry warning.
- [ ] At the end: **Settings → Move my money now**. Home shows 0 on the server.

## Write down for Stage 5

The Lightning receive fee; the parsed fee table and coin lifetime; the smallest
Ark transfer that worked; how long a transfer and a renewal really took; your
`ark1…` address format (for the "different Ark server" check); the emergency
withdrawal fee estimate.
