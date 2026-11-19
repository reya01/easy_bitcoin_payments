# Signet checks for what is new in rc3

Play money only. About 45 minutes, plus one 15-minute wait. Do these on a
**signet** server (the kind with the **TEST MODE** banner), after the normal
[signet drill](SIGNET_DRILL.md). For each line write PASS, or what you saw
instead and the time. Never paste recovery words, the setup code, API keys or
signing secrets into a report.

**Use Google Chrome or Brave** (see [CONTRIBUTING.md](../../CONTRIBUTING.md)). Each
step says where it happens: **your payment server** (its `/admin` page or its
Console), **your WooCommerce site**, **the customer's payment page** (what a
shopper sees), or **your phone wallet**.

## A. The customer's payment page

Make a test order on **your WooCommerce site** priced at about 1,500 sats
(cheap, under a dollar) and open the payment page it sends you to.

- [ ] **Banner:** the top says exactly **TEST MODE: play money (signet). Nothing
      here is real.** (On a real-money server it says REAL MONEY instead; that is
      unchanged.)
- [ ] **Part payment.** From **your phone wallet** pay **about half** of the
      amount using **Ark**. Within a few seconds the page should say **Part paid:
      N sats to go**, show how much has arrived, count down **Time left to send
      the rest**, and say **Don't send the full amount again**. The Lightning and
      "Ark (fast)" box should be gone and the page should show Ark.
- [ ] **Finish it.** Pay exactly the rest. The page changes to **Paid. Thank you!**
      and the order moves to Processing on **your WooCommerce site**.
- [ ] **Window runs out.** Make a second test order, pay about half, then wait
      15 minutes without paying more. The page should say **the rest didn't come
      in time**, show a reference starting `inv_`, say to contact the store, and
      hide the QR code. Note: money you sent is **not** lost; it shows on
      **your payment server's** **Alerts** tab, and Home says there is a payment
      that needs you.
- [ ] **Timer runs out.** Make a third order and do not pay. When the timer
      reaches zero the QR code disappears and the page says **This payment
      request expired. Don't send money to it.**
- [ ] **Lost connection.** On a fourth order, turn on airplane mode for about 10
      seconds. The page should say **Can't reach the store's server.
      Reconnecting…**, and clear itself when the connection comes back.
- [ ] **On-chain QR** (only if on-chain is switched on): choose **Regular Bitcoin**.
      Scan the QR with a wallet that shows what it read: it should show the
      address **and** the amount. The big **Copy** button copies a link that
      starts `bitcoin:` and has `?amount=`; **Copy address only** copies just the
      address.

## B. Setup page (on a brand-new server, before you create the wallet)

- [ ] **Restore question.** Press **Restore the wallet I already have**. It asks
      **Which kind of money did your old server use?** Choosing **Real money**
      shows a tick-box on the same screen.
- [ ] **Word counter.** Type or paste words: **N of 12 words** updates as you go
      (pasting them one per line counts correctly). With anything other than 12
      words, **Restore my wallet** says how many it found and sends nothing.
- [ ] **Secrets.** On the last setup step, after **Create the store**, the API
      key and signing secret each have a **Copy** button, and a **Shown once**
      line above them.
- [ ] **Slow buttons.** **Create the wallet** and **Create the store** change to
      "…this can take a minute" while they work.
- [ ] **Changed setup code.** Part-way through setup, restart the server (in its
      Console: `cd /opt/ebp && docker compose restart`), then press a setup
      button. The page should go back to the code screen and say your code is no
      longer valid, your progress is saved, and where to find the new one.
- [ ] **Address check.** At the payout step choose **Ark wallet** and paste
      `ark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqqnaa07n`
      (a real-money address). A red **Danger** box appears that reads as full
      sentences (it starts "This server is in test mode…" and ends with the Bark docs
      link and a full stop, before "Bitcoin payments cannot be undone"). It still says
      you can go on if you are only testing with play money. Do not press Create
      with it unless you want to see what ticking the box does on play money.
- [ ] **Recovery-word note** reads in plain words; the technical detail is under
      **For your technical friend**.

## C. Your payment server's admin page

- [ ] **Confirmations.** Each of these asks first, in the page, and does
      **nothing** if you choose the Keep/Cancel button: **New signing secret**,
      **Turn off** (Connect tab), **Make a new API key**, **Move my money now**
      (Settings). Also **Stop** on an emergency withdrawal if one is running
      (leave it alone otherwise). Pressing Escape cancels.
- [ ] **Each message names the other side:** it says your **WooCommerce site**,
      a different website from this page.
- [ ] **Password change.** In Settings change the admin password: you are signed
      out and the sign-in screen says **Password changed. Sign in with your new
      password.** Signing out normally shows no such line.
- [ ] **Forgot your password?** On the sign-in screen the link opens the reset
      guide, and the text says it happens in your **payment server's Console**.
- [ ] **Try again.** With airplane mode on for a few seconds, reload Home: the
      update, backup-check and phone-alert cards say **We couldn't check…** with
      a **Try again** button, and recover after the connection returns.

## Write down

Anywhere you hesitated, guessed or got stuck, and any text that was confusing.
