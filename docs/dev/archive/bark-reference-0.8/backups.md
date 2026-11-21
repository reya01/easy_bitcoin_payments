> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Backups

> Design patterns for backing up Bark-based wallets: seed phrase backups and the mailbox restore, and continuous wallet database backups for the SDK and Barkd.

There are two ways to back up a Bark-based wallet.

1. **[Record the mnemonic (seed phrase)](#seed-phrase-backups):** This alone is enough to recover a wallet's balance, with the Ark server's cooperation. It can't restore payment history or the state of any in-progress exits, and it's also the key that decrypts encrypted continuous backups.
2. **[Maintain a continuous backup of the wallet database](#continuous-backups):** A backup the user fully controls. It restores a wallet in full, payment history and in-progress exits included, without any support from the server.

## Seed phrase backups

Every Bark wallet posts a record of its VTXOs to a mailbox on the Ark server that only its mnemonic can unlock. A wallet restored from the mnemonic alone derives its keys, pulls the VTXO record back, verifies every VTXO, then rebuilds the spendable off-chain balance. A chain scan then recovers the on-chain balance.

The seed phrase alone is a safety net, but it isn't a complete backup.

* The backup depends on the server to cooperate and respond with the required data.
* It restores the balance, not the payment history or the state of an in-progress exit.

### Revealing the seed phrase

> [tab] Bark SDK
>
> The SDK does not hold the mnemonic for you by default. You'll need to ensure your app generates it, stores it, and reveals it when required.
>
> **1. Generate the phrase**
>
> Call `generateMnemonic` and pass the result in when you create the wallet.
>
> **2. Store it securely**
>
> Write the phrase to your app's secure storage.
>
> **3. Offer a reveal screen**
>
> Show the words on demand from a settings screen, behind your app's own authentication.

> [tab] Barkd
>
> By default, `barkd` generates the mnemonic and keeps it in its data directory upon wallet creation. You can reveal it through the REST API.
>
> **1. Enable the endpoint**
>
> Start `barkd` with the `--expose-mnemonic` flag. Without it, the mnemonic endpoint returns 404.
>
> **2. Fetch the phrase**
>
> Call `GET /api/v1/wallet/mnemonic` to show the words on demand.
>
> > [!WARNING]
> >
> > Keep the mnemonic endpoint behind your deployment's authentication. Anyone with access to the mnemonic can obtain full control of the wallet.

## Continuous backups

To ensure users don't have to rely on the Ark server for backups, your app should keep backups of the wallet database that update after every wallet state change, in addition to recording the mnemonic.

An up-to-date wallet database backup includes all the off-chain transactions that make up a user's VTXOs. Therefore it will restore the wallet in full: the whole balance, the payment history, and any in-progress exits, all without the Ark server's involvement.

Any good backup solution follows three rules.

* **Back up on every state change, not on a schedule:** Wallet state changes with every board, send, receive, refresh, offboard, and exit. Anything a backup misses can only be recovered through the seed phrase mailbox.
* **Store backups somewhere that outlives the host device:** A copy kept on the wallet's own device disappears with it.
* **Encrypt backups with a key derived from the mnemonic:** The seed holder can always decrypt, so a restore needs nothing but the phrase, and the backup grants no access that the seed doesn't already grant.

How you make continuous backups depends on what you're building on.

> [tab] Bark SDK
>
> The SDK persists wallet data through the storage your app provides, so backing it up is your app's responsibility.
>
> #### Mobile and desktop apps
>
> Wallet data typically lives in the app sandbox. Backups should go to storage that the user controls, such as their platform-native cloud drive, after each state change.
>
> [Noah](https://github.com/smolcars/noah), a full-stack React Native wallet, has a mature example of mobile backups for Bark.
>
> #### Web apps
>
> In the browser, the SDK stores wallet data in IndexedDB. IndexedDB is scoped to the site origin and can be evicted by the browser, so it's even more important to ship every change to another secure location and to make sure the user has recorded their mnemonic.

> [tab] Barkd
>
> `barkd` stores the wallet database (`db.sqlite`) and the mnemonic in the data directory, which defaults to `$HOME/.bark` on all operating systems. You can override it with the `--datadir` flag or the `BARKD_DATADIR` environment variable. The mnemonic file is stored in plain text, so restrict access to the directory and to anything that backs it up.
>
> Some options for backing the wallet database up on every state change:
>
> * **[Litestream](https://litestream.io/):** A streaming replicator that keeps a live, off-machine copy of the wallet database. The wallet database isn't formally tested in the WAL journal mode Litestream requires, so treat the approach as experimental.
> * **[bark-backupd](https://gitlab.com/ark-bitcoin/labs/bark-backupd):** Second's experimental backup daemon, currently under active development. Run it alongside `barkd` to take a snapshot of the wallet database on every change, encrypt each snapshot with a key derived from the mnemonic, and ship it to your chosen targets:
>   * A USB drive
>
>   * Nextcloud
>
>   * Google Drive
>
>   * Anything S3-compatible
>
> > [!WARNING]
> >
> > Test `bark-backupd restore` before you rely on bark-backupd.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.