# Versions & upgrades

Releases are cut by pushing a `v*` tag: `.github/workflows/release.yml`
builds the x86_64 and aarch64 musl binaries, `SHA256SUMS`, the GitHub
release, and the multi-arch (amd64 + arm64) image at
`ghcr.io/<owner>/<repo>`. `scripts/install.sh` and this doc resolve those
URLs; `scripts/release.sh` builds the same artifacts locally without
publishing.

## Docker one-liner deploy (the supported path)

```
docker compose pull
docker compose up -d
```

SQLite migrations run automatically on boot. Afterwards check
`GET /api/v1/health` and the dashboard's ASP indicator before taking
payments again.

## Bare binary (advanced self-hosters)

```
scripts/install.sh 0.1.0        # a specific release
scripts/install.sh              # latest
```

The installer detects the architecture and verifies the tarball against
`SHA256SUMS` before installing.

## Before upgrading

- Read the release notes on the releases page; the CHANGELOG states
  exactly what the server can and cannot do at each custody level.
- Upgrade one release at a time and read each release's notes — the spec
  requires the upgrade path to be tested in CI before a release is
  tagged, and until that job exists this doc says so rather than
  pretending otherwise.
- The barkd sidecar is version-pinned in docker-compose; bump it only
  with an EBP release that says it was tested against the new version.