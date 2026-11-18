# Reproducible builds (plan, not built yet)

**Goal:** anyone can build a release from its source and get the **same bytes** the project
published, so the signature does not have to be taken on trust ("built by our workflow") but
can be checked against the source ("this is what that source produces").

**Why it matters:** the signed-update path ([UPGRADING.md](../UPGRADING.md)) makes the release
workflow and the GitHub account the single point of trust. A reproducible build lets an
independent person notice if a published image ever differs from the code in the repository.

## Do it in two stages

**Stage 1: the `ebp` binary.** The release already publishes `ebp-*.tar.gz` and `SHA256SUMS`.
Make `scripts/release.sh` produce byte-identical binaries on two machines, then add a CI job
that builds twice on different runners and fails if the hashes differ. Anyone can then run
`scripts/release.sh` at a tag and compare with `SHA256SUMS`.

What usually breaks it, and what to do:
- **Paths and time baked into the binary.** Build in a fixed directory, pass
  `--remap-path-prefix` for the source and cargo home, set `SOURCE_DATE_EPOCH` to the commit time.
- **Moving dependencies.** `Cargo.lock` is committed; always build with `--locked` (and
  `--offline` against vendored crates, see below).
- **Incremental and parallel noise.** `CARGO_INCREMENTAL=0`, one codegen unit if needed.
- **The toolchain.** `rust-toolchain.toml` pins it; the build image is already pinned by digest.

**Stage 2: the image.** Docker adds more moving parts:
- `apt-get install` in the image fetches whatever Debian serves that day. Either remove the
  packages (the healthcheck can stop needing `curl`), or install from a dated
  `snapshot.debian.org` mirror with exact versions.
- File timestamps and layer order: build with BuildKit `rewrite-timestamp=true` and
  `SOURCE_DATE_EPOCH`.
- The wallet program (barkd) is downloaded with a pinned sha256 already; keep that.
- Compare **layers** (or the binary inside), not the manifest, since provenance and SBOM
  attestations are attached next to the image and legitimately differ.

## Then publish a way to check

`scripts/verify-release.sh VERSION`: rebuild at the tag, compare to the published binary hash
and to the image's layer hashes, print match or mismatch. Mention it in each release's notes.

## Related, smaller steps that help

- Vendor the crates (`cargo vendor`) or build with `--locked` in CI so the build never asks
  crates.io for anything it did not already have; CI already runs `cargo audit`.
- Keep the Dependabot proposals for base images and actions; a human reads each one.

Not started. Order: stage 1 first (cheap, and CI proves it), then stage 2.
