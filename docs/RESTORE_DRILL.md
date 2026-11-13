# Practice a restore: pause your server, start a new one, bring everything back

Do this once, with play money, before you put real money on a server. It proves that
your 12 recovery words and your backup file really bring your shop back. If you only
read one page about backups, read this one and then actually do it.

It takes about an hour, most of it waiting. Each step says **where** you do it.
Your host may label things a little differently; the steps use DigitalOcean's words,
like [START_HERE.md](START_HERE.md).

**You will need**
- Your **12 recovery words** on paper, and your **admin password**.
- A way to save a file on **your own computer**, not on the server.
- Your WooCommerce admin, in case the new server has a different web address.

**Never type your 12 words anywhere except the new server's setup page.** Not in a
chat, not in a document, and nobody (including an AI assistant) should be asked to
look at them.

For what each piece brings back and what can be missing, see
[RECIPE_BACKUP.md](RECIPE_BACKUP.md) → **Restore on a new server**. This page is the
same process as a practice run, with checks.

## Part 1: Get the old server ready (about 10 minutes)

1. **On the old server, in your host's Console** (DigitalOcean: Droplet → Console, log
   in as `root`), update it so the practice runs on the current code. Run these one
   at a time and wait for each to finish:
   ```
   cd /opt/ebp
   git pull
   ./scripts/start.sh
   ```
2. **In your browser, on the old server's `/admin` page,** open **Home** and
   **Settings**. Write down your answer key:
   - the **balance** ("Held on your server")
   - the **number of orders**
   - the **payout address**
   - the **keep amount**
3. On **Home**, check that no "Routine wallet upkeep" is in progress and no order is
   waiting to be paid. If something is, wait until it finishes. Leave a few hundred
   sats on the server so there is something to bring back.
4. **In Settings, press Download encrypted backup.** It is a file ending in
   `.ebpbackup`. Save it on **your computer**, not on the server.
5. **In Settings, use Check a backup** on that exact file, with your password. Go on
   only if it says the backup is good.
6. Put your **12 paper words** and your **admin password** next to you.

## Part 2: Turn off the old server

7. **On your host's website,** open the old server and turn it off (DigitalOcean:
   **Power → Turn off**). **Do not delete it.** It is your way back if the practice
   fails.
8. Check that its `/admin` page no longer opens. **Never run the old and the new
   server at the same time:** both would use the same wallet and the same shop
   connection.

## Part 3: Make the new server (about 30 minutes, mostly waiting)

9. **On your host's website,** make a new server exactly as in
   [START_HERE.md](START_HERE.md), steps 1 to 3: the same setup text, region and
   size, and the **same mode** (play money or real money).
10. **In the new server's Console,** wait for the box that shows the web address and
    the **SETUP CODE**. Stop there. Do not set anything up yet.

## Part 4: Restore

11. **In your browser,** open the new server's address and type the setup code.
12. On the **Create your wallet** step, **do not press Create.** Press **Restore the
    wallet I already have, from my 12 words**.
13. Type the 12 words in order, tick the box saying your paper copy exists, and press
    **Restore my wallet**. If it says the words were not accepted, check each word and
    its order on your paper copy.
14. On **Bring back your orders and settings,** choose your `.ebpbackup` file, type
    the password you had **when you downloaded it**, and press **Restore from this
    backup**. Wait about a minute while the server restarts itself. If the page says
    the restore was refused, take a screenshot and stop there: nothing was
    overwritten.
15. **In your browser,** open the new address followed by `/admin`. Sign in with your
    **old** admin password.
16. **In Settings,** turn **two-step sign-in** back on. It always comes back **off**
    after a restore, on purpose.

## Part 5: Check that it worked

17. **On Home,** compare with your answer key: balance, number of orders, payout
    address, keep amount. Coins can take a few minutes (play money) or up to about an
    hour (real money) to appear. The old transfers list should be there too.
18. **On your WooCommerce site,** if the new server's web address is different, change
    the **server address** in the plugin's settings. The API key and the signing
    secret are unchanged, because they came back with the backup.
19. **Place a small test order** (about 300 sats). **"Your shop is receiving payment
    notices" on Home can be carried over from the backup, so only a real test order
    proves the shop connection.** Check that the order completes.
20. **In Settings, press Move my money to my payout address now.** Check that Home
    shows a new row in "Recent transfers" and that the money arrives in your own
    wallet. Part of the balance may say "still arriving or being renewed"; that part
    simply cannot be moved for a few minutes.
21. **In Settings, download a fresh backup** from the new server, check it with
    **Check a backup**, and keep it off the server.
22. Write down the date, how long the restore took, and anything that differed from
    your answer key. Keep this next to your paper words.

**You pass when** the balance, the orders, the payout address and the shop connection
match your answer key, a new test order completes, and **Move my money now** sends
money to your own wallet.

## What you may notice (and it is fine)

- A **"Routine wallet upkeep"** row can appear right after the restore: the new server
  checked its coins and renewed them. The balance does not change.
- **Orders paid after your last backup download** are not on the server's list.
  WooCommerce still has them.
- A payment that arrived while the old server was off may not match an order
  automatically. Compare WooCommerce's **On hold** orders with the money on Home.
- In the first hour after a restore, an old order showing as overpaid or a "payment
  can't be matched" item is worth a screenshot. It is not expected.

## Part 6: Clean up

23. Keep **one** server. If everything matched, delete the **old** server on your host's
    website (or leave it powered off for a few days as a fallback, then delete it).
24. If it failed, delete the **new** server and power the old one back on. Fix the
    problem before you put real money on a server. A backup that has never been
    restored is a story, not a backup.

## If something goes wrong

| What you see | What to do |
| --- | --- |
| "The words were not accepted" | Check each word and its order on paper. Do not type them anywhere else. |
| The restore says "refused" | Nothing was overwritten. The new server must be brand new: it refuses if it already finished setup or holds orders. Make a fresh one. |
| The backup will not open | It needs the admin password you had **when you downloaded it**. Without that password the file cannot be opened. |
| The balance is lower or missing for a while | Wait a few minutes (play money) or up to an hour (real money), then refresh. The money comes from the 12 words, not from the backup file. |
| The shop does not get notices | Check the server address in the plugin's settings on your WooCommerce site, then place a test order. |
| You lost the 12 words | The money in the server's wallet cannot be recovered from the backup file alone. This is why they are written on paper twice. |

More: [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md) and
[RECIPE_BACKUP.md](RECIPE_BACKUP.md).
