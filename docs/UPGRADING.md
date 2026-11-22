# Updating EBP

**EBP never updates itself.** There is no auto-update and no option to turn one on.
You choose when.

## How you find out

The admin page's **Home** tab shows "Update available" (or "Security update" when a
release title contains `[security]`), the steps for your host, and how many orders
are open. It learns this from one anonymous request to the public releases page,
cached for an hour. Switch that off with `EBP_UPDATE_CHECK=false`; it never installs
anything. Read the release notes first, and update **one release at a time**.

Restarting is always safe: pending payments and shop notifications are stored and
resume.

## Release candidates

Every release so far is a release candidate, and from 0.2.1-rc2 the Home tab offers them as
updates too. You still choose; nothing is installed for you. Read the
[CHANGELOG](../CHANGELOG.md) first to see what changed. Your wallet, settings, orders and shop
connection are kept.

## Update from the admin page (Home tab, new in 0.2.1-rc2, experimental)

With the **update helper** installed, you can update without the Console:

1. On the **Home** tab, the Version row says *Update available* and the **Update card** at the
   top offers the new version. Press **Start update**. This only checks the new version
   (about a minute); nothing changes.
2. The card then shows **Now** and **New** side by side, the signature result ("built and
   signed by this project's release workflow for v..., from commit ..."), the exact image
   (digest), and links to inspect it further on GitHub: the release page and the workflow run
   that built it. Download a fresh backup (the card links to it).
3. To go ahead, type your **admin password** and, if two-step sign-in is on, your **6-digit
   code**, and press **Update now**. The helper checks everything again and installs only the
   image you were shown. The page reloads itself when the new version answers.

If anything is wrong the card says why in plain words (for example "older than the version you
run" or "the signature check failed") and nothing is changed. The helper never goes backwards,
has no way to skip that check, and only installs this project's signed releases.

**Installing the helper.** New DigitalOcean servers get it automatically. On a server set up
earlier, open the Console once, log in as root, and run these two lines, one at a time (the
folder name may differ; `ls /opt` shows it):

```
cd /opt/ebp
./scripts/install-update-helper.sh
```

When it says the helper is installed, refresh the admin page: the Update card then shows **Start update**.

It is a short script (`scripts/update-helper.sh`) that runs as root, outside EBP, only when the
Home tab drops a request, and it runs the same `update.sh` checks described below. EBP itself
never gets control of the server. To remove it: `./scripts/install-update-helper.sh --uninstall`.

## Update a DigitalOcean server from the Console (about a minute)

In the DigitalOcean website open your server, then **Console**, log in as `root`
and run these, one at a time (the folder name may differ; `ls` shows it):

```
cd /opt/ebp
./scripts/update.sh --check 0.3.0-rc3
./scripts/update.sh 0.3.0-rc3
```

Use the version you want instead of `0.3.0-rc3`. The first command only **checks**:
it shows which version runs now, the version you asked for, the exact image
(its "digest") and whether the signature is genuine, and changes nothing. The second
shows the same and then asks you to **type yes** before it changes anything.

What `update.sh` does for you, in order:

1. It works out the version your server runs now and **refuses to go backwards** (an
   older version cannot read newer data).
2. It downloads the release's image address and checks the **signature** with
   [cosign](https://docs.sigstore.dev/cosign/overview/): the image must have been built and
   signed by this project's release workflow for exactly that version, from exactly that
   commit. If the check fails, nothing is used.
3. It shows you the version, the digest and the result, and waits for you to type `yes`.
4. It switches the server to that exact image and restarts it. If the download fails, the
   old setting is put back. It never quietly builds anything on the server instead.

**EBP never updates itself.** Nothing runs `update.sh` for you: no timer, no auto-update.

Then check that the admin **Home** page says the wallet is connected before you take
payments again. Nothing is lost: your data lives in a separate volume, and a copy is taken
before any database change. If you turned on https with `start.sh`, that choice is
remembered. Download a fresh backup (Settings) before you update.

**The first time**, your folder may not have `update.sh` yet: run `git pull` once, then the
commands above. `update.sh` then moves the folder to the release's own files, so later
updates use `update.sh` again (a plain `git pull` no longer applies, by design).

**The signature checker is fetched for you.** The first time, `update.sh` downloads
[cosign](https://docs.sigstore.dev/cosign/overview/) into the EBP folder and checks it against a
fingerprint (sha256) this project has pinned, so you install nothing by hand. If you already have
cosign installed, it uses that one.

The signed image is built for x86-64 servers (most DigitalOcean droplets). On an ARM server
use the build-from-source way below.

## Build it yourself instead (slower, fully independent)

If you would rather not rely on the project's prebuilt image, build the release from its
source on your own server:

```
cd /opt/ebp
./scripts/update.sh --from-source 0.3.0-rc3
```

It fetches that release's source, switches the folder to it, and builds the server there.
This takes several minutes (10 or more on a small server) and downloads the build's own
packages (Rust packages and Debian packages, plus the pinned wallet program), so it relies on
those sites instead of the signed image. It still **refuses to go backwards** and still
asks you to type yes. Nothing is checked by cosign in this mode, because nothing prebuilt is
used. To go back to the signed image later, run `./scripts/update.sh VERSION` again.

On any other Docker server, run the same scripts from the folder that holds
`docker-compose.yml`. (Developers working from a checkout can still run `git pull` and
`docker compose build`; that is not the supported update path.)

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
release notes list the digest and the exact `cosign verify` command (`update.sh` runs it for
you). A server keeps the exact image in its `.env` as `EBP_IMAGE=ghcr.io/<owner>/<repo>@sha256:<digest>`. Releases are cut by
pushing a `v*` tag, which builds the x86_64 binary (x86-64 only for now; ARM is planned
for v1.0), `SHA256SUMS`, the GitHub release and the image. Maintainer to-do (a repository setting):
protect the `v*` tag pattern so only maintainers can create release tags.

What these checks do and do not prove: the signatures and provenance show that this
repository's release workflow built and signed the files. They do not show that a
particular person vouches for them, so trust in a release rests on the workflow and on
the GitHub account that controls it. A signature made with a maintainer's own key,
published somewhere other than GitHub, is planned for the final release.

**Bare binary** (self-hosters without Docker):

```
scripts/install.sh 0.1.0        # a specific release
scripts/install.sh              # latest
```

"Latest" means the newest *full* release. GitHub does not count release candidates
such as `0.2.0-rc3`, so while only candidates exist give the exact version:
`scripts/install.sh 0.2.0-rc3`. The installer detects the architecture, checks that
`SHA256SUMS` was signed by this project's release workflow for that exact tag
(sigstore, keyless), then checks the tarball against `SHA256SUMS`. It downloads its own
copy of the signature checker (cosign) for this and checks that copy against a
fingerprint pinned in the script, so there is nothing to install first. If either check
fails it stops and installs nothing.

`scripts/release.sh` builds the same artifacts locally without publishing.

**What the signed update does not protect against.** `update.sh` itself comes from `git`, not from
the signed release: it is about 200 lines, so read it before you run it. If the GitHub account or
the release workflow were taken over, a validly signed bad image could be published; that is why
releases are cut by hand, nothing updates by itself, and you see the version and digest first.
Reproducible builds, so anyone can check a published image against the source, are planned
([dev/REPRODUCIBLE_BUILDS.md](dev/REPRODUCIBLE_BUILDS.md)).
