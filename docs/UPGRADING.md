# Updating EBP

**EBP never updates itself.** There is no auto-update and no option to turn one on.
You choose when.

## How you find out

The admin page's **Home** tab shows "Update available" (or "Security update" when a
release title contains `[security]`), the steps for your host, and how many orders
are open. It learns this from one anonymous request to the public releases page,
cached for hours. Switch that off with `EBP_UPDATE_CHECK=false`; it never installs
anything. Read the release notes first, and update **one release at a time**.

Restarting is always safe: pending payments and shop notifications are stored and
resume.

## While only release candidates exist

The "Update available" notice only looks at full releases, so it will not appear for
a release candidate. To get the newest one, use the same steps below (`git pull`
fetches the newest code). Read the [CHANGELOG](../CHANGELOG.md) first to see what
changed. Your wallet, settings, orders and shop connection are kept.

## Update a DigitalOcean server (the usual way)

In the DigitalOcean website open your server, then **Console**, log in as `root`
and run:

```
cd /opt/ebp
git pull
./scripts/start.sh
```

Wait a few minutes (it rebuilds; on a small server this can take 10 minutes or more), then check that the admin **Home** page says the
wallet is connected before you take payments again. Nothing is lost: your data lives
in a separate volume, and a copy is taken before any database change. If you turned
on https with `start.sh`, that choice is remembered.

On any other Docker server, the same thing from the folder that holds
`docker-compose.yml`:

```
git pull
docker compose build
docker compose up -d
```

On Render (paid plan required, see [OTHER_HOSTS.md](OTHER_HOSTS.md)), `render.yaml`
sets `autoDeploy: false`: update with **Manual Deploy → Deploy latest commit** and
leave auto-deploy off.

## Update the WooCommerce plugin (separate from the server)

The server and the plugin are updated separately. When a release says the plugin
changed, **on your WooCommerce site** go to **Plugins → Add New → Upload Plugin**,
choose the new `ebp-bitcoin-woocommerce.zip` and, when WordPress says the plugin is
already installed, choose **Replace current with uploaded**. Your settings (server
address, API key, signing secret) are kept; open **WooCommerce → Settings →
Payments → Manage** next to **Bitcoin (Easy Bitcoin Payments)** and press **Save
changes** to confirm the green "connected" message. The zip is under **Assets** on the
[Releases page](https://github.com/reya01/easy_bitcoin_payments/releases).

## The safety net

If a new version needs to change the database, the server first copies it into a
`backups` folder beside it (the last three are kept, readable only by the server's
user). If the copy cannot be made, the server does not start rather than change your
data. The reverse is protected too: an **older** version refuses to start on data
written by a **newer** one, with a plain message.

To roll back, stop the server, put the copy back as `ebp.sqlite3`, and run the older
version. These copies live on the same disk, so they guard against a bad upgrade, not
against losing the disk: see [RECIPE_BACKUP.md](RECIPE_BACKUP.md).

## Before you update

- Read the release notes and the [CHANGELOG](../CHANGELOG.md).
- One release at a time. CI checks on every change that the previous build upgrades in
  place with your data intact and that an encrypted backup restores; it cannot check
  every jump between far-apart versions, which is why one step at a time is the rule.
- The wallet container (barkd) is version-pinned in `docker-compose.yml`. Change it
  only with an EBP release that says it was tested against the new version.
- Download a fresh backup first (Settings), especially before a bigger change.

## Advanced

**Verifying a release.** Release images are built by this repository's release
workflow, published by immutable digest (there is deliberately no `latest` tag),
signed keylessly with sigstore/cosign, and carry build provenance and an SBOM. The
release notes list the digest and the exact `cosign verify` command. To pin a
deployment, use `image: ghcr.io/<owner>/<repo>@sha256:<digest>`. Releases are cut by
pushing a `v*` tag, which builds the x86_64 and aarch64 binaries, `SHA256SUMS`, the
GitHub release and the multi-arch image. Maintainer to-do (a repository setting):
protect the `v*` tag pattern so only maintainers can create release tags. The release
workflow has not been run yet; run it on a test tag before the first real release.

**Bare binary** (self-hosters without Docker):

```
scripts/install.sh 0.1.0        # a specific release
scripts/install.sh              # latest
```

"Latest" means the newest *full* release. GitHub does not count release candidates
such as `0.1.0-rc2`, so while only candidates exist give the exact version:
`scripts/install.sh 0.1.0-rc2`. The installer detects the architecture and verifies
the tarball against `SHA256SUMS`. That file is itself signed (sigstore, keyless); to
check the signature too (optional, advanced; needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/)):

```
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp '^https://github.com/reya01/easy_bitcoin_payments/\.github/workflows/release\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

It prints `Verified OK` when `SHA256SUMS` was signed by this project's release workflow.

`scripts/release.sh` builds the same artifacts locally without publishing.
