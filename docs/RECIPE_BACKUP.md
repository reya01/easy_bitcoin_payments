# Backups, recovery words and a lost phone

Two things keep you safe, and they are different:

1. **Your 12 recovery words** (on paper). They can rebuild the small wallet on
   your server, and with it any money still in it. They are the real backup.
2. **A downloaded backup file.** It saves your order history and settings so a
   new server can pick up where the old one stopped.

Nothing below needs special tools. If a step does not match what you see, that is
a mistake in this page; please report it.

## Download a backup (2 minutes)

Admin page → **Settings → Download a backup**. Type your admin password (and your
authenticator code, if two-step sign-in is on). Your browser saves one file ending
in `.ebpbackup`, locked with that same password. EBP never stores it.

- **What is inside:** your order history and settings, and the server's encryption
  key, so this one file restores everything. When the server runs the wallet itself
  (the normal setup) the wallet folder is inside too. If you run a separate wallet
  container, the page says the wallet files are **not** inside; back up that
  container's data folder instead.
- **Two-step sign-in is not in the file.** The file opens with your password alone,
  so keeping the authenticator secret in it would let the password undo the second
  lock. After a restore, two-step sign-in is off; the admin page offers to turn it
  on again (Settings → Two-step sign-in).
- **Keep it off the server:** a password manager's file storage, a USB stick or
  another cloud account. A backup on the same disk is not a backup.
- **Use a long password.** The file can contain the wallet, and anyone who gets it
  can guess the password offline with no limit. Admin passwords must be 16+ characters
  (a few random words is ideal).
- **Passwords change.** The file opens with the admin password *at the time of the
  download*. After you change or reset the password, download a fresh backup.
  Lose the password and the file cannot be opened.
- **It is one moment in time.** Download a new one after anything important.

### Check that a backup opens

Nothing is changed. The easy way: in the admin page open **Settings**, find
**Check a backup**, choose the file you saved and type its password (the admin
password you had when you downloaded it). It says the backup is good and what it
holds. The file is not kept.

For a terminal check, on a DigitalOcean server, in the Console (the file has to be
on the server first, for example copied there with `scp` from your computer):

```
cd /opt/ebp
docker compose cp my-backup.ebpbackup ebp:/tmp/b.ebpbackup
docker compose exec ebp ebp backup verify /tmp/b.ebpbackup
```

(Without Docker: `ebp backup verify my-backup.ebpbackup`.) It asks for the password.

### Restore on a new server (the 12 words plus your backup file)

You need two things: your **12 recovery words** (paper) and your **latest backup
file** (ends in `.ebpbackup`) with the **admin password you had when you downloaded
it**.

**What each one brings back.**

| | Brings back | Does not bring back |
| --- | --- | --- |
| **The 12 words** | The money still in the server's wallet | Order history, settings |
| **The backup file** | Order history, your payout address, your admin password, your shop connection (API key and signing secret) | Money that moved after the download; two-step sign-in (it comes back **off**, turn it on again) |

The money always comes back from the words. The wallet folder inside the backup file
is never used, because it may be out of date.

**The steps.** Each one says where you do it.

0. **If the old server still exists, turn it off first** (on your host's website,
   power it off; do not delete it yet). **Never run the old and the new server at
   the same time:** both would use the same wallet and the same shop connection, and
   could get in each other's way, including with the money.
1. **On your host's website:** make a fresh server the same way as the first time
   ([START_HERE.md](START_HERE.md)), in the same mode (play money or real money).
   Stop when it shows you the setup page and the setup code.
2. **In your browser, on the new server's setup page:** type the setup code.
3. On the **Create your wallet** step, do **not** press Create. Press **Restore the
   wallet I already have, from my 12 words**.
4. Type your 12 words in order, tick that your paper copy exists, and press
   **Restore my wallet**. Only the setup page ever sees them. If it says the words
   were not accepted, check each word and its order on your paper copy.
5. On the next step, **Bring back your orders and settings**, choose your newest
   `.ebpbackup` file, type its password, and press **Restore from this backup**. The
   server restarts by itself; this takes about a minute. (No backup file? Press
   **I don't have a backup file** and set up a new store instead.)
6. **In your browser:** open your new server's address followed by `/admin`. Sign in
   with the admin password you had when you downloaded the backup.
7. In **Settings**, turn **two-step sign-in** back on.
8. On **Home**, check the balance, the payout address and your orders. Coins can take
   a round or two (a few minutes on play money, up to about an hour on real money) to
   appear after a restore.
9. If the new server has a different web address, on your **WooCommerce site** open
   the plugin's settings and change the server address. The API key and the signing
   secret are unchanged, because they came back with the backup.
10. **In Settings, download a fresh backup** and keep it off the server.

**What can be missing, and what to do.**

- **Orders paid after your last backup download** are not on the payment server's
  list, but WooCommerce still has them. Download backups regularly.
- **A payment that arrived while the old server was down** may not match an order
  automatically. Compare WooCommerce's **On hold** orders with the money on Home.
- **A restore never overwrites a working server.** If the new server already holds
  orders or finished setup, it refuses and keeps your files. Use a fresh server.
- **The words need the Ark server to cooperate.** If the Ark server itself is gone,
  getting Ark coins back is a different, emergency path that this page does not cover.
- **Without the backup file's password**, the file cannot be opened. Without the
  words, the money cannot be recovered from the file alone.
- **If the wizard's backup step does not work for you**, the older manual way is:
  stop EBP, run `ebp backup restore my-backup.ebpbackup ./restored`, copy only
  `ebp.sqlite3` and `ebp.key` (**not** the `barkd` folder) into the data folder, and
  start EBP again. Restore the wallet from the words first, either way.

After a restore the old files the server replaced are kept in a folder called
`restore-previous-<number>` inside the data folder, so nothing is ever thrown away.

## The recovery words

The setup wizard shows them once. For any wallet, any time:

- [ ] Only you are present: no screen sharing, no camera, no assistant of any kind.
- [ ] Write them on paper with a pen, **twice**.
- [ ] Keep the two copies in two separate safe places (not the same building or bag).
- [ ] Never photograph them or type them anywhere: no chat, document, cloud or
      password-manager note.
- [ ] Tick the box in the wizard. Setup cannot finish without it, on purpose.

Confirm, out loud if it helps, that both paper copies exist before you go on.

## Everything that matters, in one table

| What | Why | Where it lives |
| --- | --- | --- |
| Recovery words | Rebuild the server's wallet on any machine | Paper, two copies, two places |
| Your own payout wallet's seed (for example Arké) | That is where your money ends up: your keys | That wallet's own guidance |
| The downloaded backup file | History, settings, encryption key, wallet folder | Off the server |
| Settings you made yourself (`EBP_PUBLIC_BASE_URL`, and `EBP_BARKD_TOKEN` only with a separate wallet container) | Needed to start again | docker-compose environment |

Never backed up, by design: the wallet container's access token (environment only;
get it again with `barkd secret show`), the recovery words themselves (never
stored or logged), and customer data (there is almost none: no accounts).

## Lost phone

The phone is a window, not the vault. The money is on the server and in your payout
wallet, so losing the phone must not mean losing the business.

1. **Nothing about the payment server changed.** Orders keep working and payouts
   keep going to your saved payout address.
2. **Restore your own wallet** (if the phone held it): reinstall the app and restore
   it from **its own** seed, not from the server's recovery words.
3. **Open the admin page again** on the new phone: your server's address followed by
   `/admin`, then sign in with your password and an authenticator code. No phone and
   no backup codes? See [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md).
4. **If the phone may have been stolen**, not just lost: change the admin password in
   Settings (this signs every device out), make a new API key on your payment server's **Connect** tab
   and paste it into the plugin on your WooCommerce site, and download a fresh backup. With a separate
   wallet container, also rotate its token (`barkd secret refresh` on the server,
   update `EBP_BARKD_TOKEN`, restart).

You are back when the admin page opens on the new phone and shows the same
balances.

## Practice a restore once (before real money)

Proves the backup is real. Allow an hour, and use play money first.

1. On your current server, note the balance, the number of orders and the payout
   address on Home and in Settings. Leave a few hundred sats on the server. **Download
   a backup** (Settings) and keep the file.
2. **Power off the first server** on your host's website (do not delete it: it is
   your way back if the practice fails). Then make a **second, new server** the same
   way you made the first. Never have both running at once.
3. Follow **Restore on a new server** above, exactly as written.
4. **Pass:** the balance, the orders, the payout address and the shop connection match
   what you noted in step 1; a new test order completes; **Move my money now** works.
5. Write down the date and anything that differed next to your paper copies.
6. Keep only one server: delete the one you no longer use (if you keep the new one,
   delete the old one; if the practice failed, delete the new one and power the old
   one back on).

If the practice fails, the backup is a story, not a backup. Fix it before going live.

## Advanced

- **Continuous wallet backup.** barkd has a companion, `bark-backupd`, that
  continuously backs up the wallet folder, encrypted with a key derived from the
  wallet's recovery words. It is **not packaged with EBP yet**, so for now use the
  Settings download plus the recovery words.
- **Copying the wallet folder by hand.** Stop the wallet first
  (`docker compose stop barkd`), copy the data volume, then start it again; a copy
  of a running wallet can catch the database mid-write. Keep such copies encrypted
  and off-site: they hold everything needed to spend the money in the server wallet.
- **If you lose the encryption key** with no backup, the stored shop secrets cannot
  be read and must be re-issued from your payment server's **Connect** tab.
