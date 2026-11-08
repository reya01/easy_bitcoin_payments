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

Nothing is changed. On a DigitalOcean server, in the Console:

```
cd /opt/ebp
docker compose cp my-backup.ebpbackup ebp:/tmp/b.ebpbackup
docker compose exec ebp ebp backup verify /tmp/b.ebpbackup
```

(Without Docker: `ebp backup verify my-backup.ebpbackup`.) It asks for the password.

### Restore on a new machine

Stop EBP, run `ebp backup restore my-backup.ebpbackup ./restored` (it refuses to
write into a folder that already has files), copy the files into the data folder,
and start EBP. The restored `ebp.key` goes next to the database (it is used when
`EBP_KEY_ENCRYPTION_KEY` is not set); if you set that variable yourself, put the
same value back. Your admin password, payout address and shop API keys are in the
restored database, so you sign in as before.

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
   Settings (this signs every device out), make a new API key on the **Connect** tab
   and paste it into your shop plugin, and download a fresh backup. With a separate
   wallet container, also rotate its token (`barkd secret refresh` on the server,
   update `EBP_BARKD_TOKEN`, restart).

You are back when the admin page opens on the new phone and shows the same
balances.

## Practice a restore once (before real money)

Proves the backup is real. Allow an hour, and use play money first.

1. Note the balance the admin page shows.
2. On a fresh server, install as in [START_HERE.md](START_HERE.md).
3. Restore the wallet from the recovery words with the wallet's own restore command.
   You type the words, nobody else. **The exact command is not written down here
   yet**; it will be added after the live signet drill
   ([SIGNET_DRILL.md](dev/SIGNET_DRILL.md)). Do not guess it.
4. Point EBP at it, start it, open the admin page.
5. The balance from step 1 should reappear. Some coins can take a round or two to
   show up after a restore; that is normal.

If the practice fails, the backup is a story, not a backup. Fix it before going
live, and note the date and what differed next to your paper copies.

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
  be read and must be re-issued from the **Connect** tab.
