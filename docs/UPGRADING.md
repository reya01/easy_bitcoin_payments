# Versions & upgrades

Releases are cut by pushing a `v*` tag: `.github/workflows/release.yml`
builds the x86_64 and aarch64 musl binaries, `SHA256SUMS`, the GitHub
release, and the multi-arch (amd64 + arm64) image at
`ghcr.io/<owner>/<repo>`. `scripts/install.sh` and this doc resolve those
URLs; `scripts/release.sh` builds the same artifacts locally without
publishing.

## EBP never updates itself

There is no auto-update, no "stable" branch that deploys on its own, and no
option to turn one on. You choose when to update:

- The admin page's Home tab shows "Update available" (and "Security update"
  when the release title contains `[security]`) with the steps for your host
  and how many orders are open. It learns this from one anonymous request to
  the public releases page, cached for hours; switch it off with
  `EBP_UPDATE_CHECK=false`. It never installs anything.
- On Render, `render.yaml` sets `autoDeploy: false`; update with
  Manual Deploy -> Deploy latest commit. Leave auto-deploy off.
- Restarting is safe at any time: pending payments and webhook notifications
  are stored and resume.

## Automatic backup before every upgrade

If a new version needs to change the database, the server first copies it to
a `backups` folder beside it (the last three are kept, readable only by the
server's user). If the copy cannot be made, the server does not start rather
than change your data. The reverse is also protected: an OLDER version refuses
to start on data written by a NEWER one, with a plain message. To roll back,
stop the server, put the copy back as `ebp.sqlite3`, and run the older
version. These copies live on the same disk, so they guard against a bad
upgrade, not against losing the disk: see RECIPE_BACKUP.md.

## Verifying a release (optional, advanced)

Release images are built by this repository's release workflow, published by
immutable digest (there is deliberately no `latest` tag), signed keylessly
with sigstore/cosign, and carry build provenance and an SBOM. The release
notes list the image digest and the exact `cosign verify` command. To pin a
deployment, use `image: ghcr.io/<owner>/<repo>@sha256:<digest>`.
Maintainer to-do (a repository setting, not code): protect the `v*` tag
pattern so only maintainers can create release tags. The release workflow has
not been run yet; run it on a test tag before the first real release.

## Docker one-liner deploy (the supported path)

```
git pull
docker compose build
docker compose up -d
```

These same three commands update the default (single-container) install. If
you use the advanced two-container install, your `.env` has
`COMPOSE_FILE=docker-compose.two-container.yml`; keep it there, and never run
the commands with a different compose file than you installed with: that would
start a second, empty stack.

SQLite migrations run automatically on boot. Afterwards check
`GET /api/v1/health` and the dashboard's ASP indicator before taking
payments again.

## Bare binary (advanced self-hosters)

```
scripts/install.sh 0.1.0        # a specific release
scripts/install.sh              # latest
```

The installer detects the architecture and verifies the tarball against
`SHA256SUMS` before installing. `SHA256SUMS` is itself signed (sigstore,
keyless) by the release workflow; to check that signature too (optional,
advanced; release workflow not yet run):

```
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp '^https://github.com/<owner>/<repo>/\.github/workflows/release\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

## Before upgrading

- Read the release notes on the releases page; the CHANGELOG states
  exactly what the server can and cannot do at each custody level.
- Upgrade one release at a time and read each release's notes — the spec
  requires the upgrade path to be tested in CI before a release is
  tagged, and until that job exists this doc says so rather than
  pretending otherwise.
- The barkd sidecar is version-pinned in docker-compose; bump it only
  with an EBP release that says it was tested against the new version.